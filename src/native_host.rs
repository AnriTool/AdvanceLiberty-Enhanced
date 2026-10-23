//! Native helpers for predecoded PSR accesses and BIOS calls. No fetch/decode.
use gba_core::{
    capi::RtApi,
    cpu::{Exception, FLAG_T},
    Bus, Machine, Mode,
};
use std::ffi::c_void;

#[repr(C)]
pub struct Api {
    pub base: RtApi,
    read_psr: extern "C" fn(*mut c_void, u32) -> u32,
    write_psr: extern "C" fn(*mut c_void, u32, u32, u32),
    swi: extern "C" fn(*mut c_void, u32, u32, u32) -> u32,
}

impl Api {
    pub fn new(base: RtApi) -> Self {
        Self {
            base,
            read_psr,
            write_psr,
            swi,
        }
    }
}

extern "C" fn read_psr(m: *mut c_void, saved: u32) -> u32 {
    let m = unsafe { &mut *(m as *mut Machine) };
    if saved != 0 {
        m.cpu.spsr()
    } else {
        m.cpu.cpsr
    }
}

extern "C" fn write_psr(m: *mut c_void, saved: u32, fields: u32, value: u32) {
    let m = unsafe { &mut *(m as *mut Machine) };
    let mut mask = 0u32;
    for byte in 0..4 {
        if fields & (1 << byte) != 0 {
            mask |= 0xff << (byte * 8);
        }
    }
    if saved != 0 {
        m.cpu.set_spsr((m.cpu.spsr() & !mask) | (value & mask));
    } else {
        if m.cpu.mode() == Mode::User {
            mask &= 0xff000000;
        }
        mask &= !FLAG_T;
        m.cpu.set_cpsr((m.cpu.cpsr & !mask) | (value & mask));
    }
}

extern "C" fn swi(m: *mut c_void, num: u32, next: u32, cycles: u32) -> u32 {
    let m = unsafe { &mut *(m as *mut Machine) };
    let key = m.cpu.regs[15] | m.cpu.thumb() as u32;
    m.bus.note_fetch(key);
    if gba_core::hle::bios_call(&mut m.cpu, &mut m.bus, num) || m.bus.note_unhandled_swi(num) {
        m.bus.note_swi_returned();
    } else {
        m.cpu.enter_exception(Exception::Swi, next);
    }
    m.cpu.regs[15] = match m.cpu.branch.take() {
        Some(target) => target & if m.cpu.thumb() { !1 } else { !3 },
        None => next,
    };
    m.bus.tick(cycles as u64);
    m.cpu.regs[15] | m.cpu.thumb() as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use gba_core::capi::RT_API;
    #[test]
    fn psr_helpers_match_interpreter_including_banked_registers_and_user_restrictions() {
        for mode in [Mode::User, Mode::Sys, Mode::Irq, Mode::Svc] {
            for saved in [false, true] {
                for fields in [1u32, 8, 9, 15] {
                    let opcode = 0xe120f003 | ((saved as u32) << 22) | (fields << 16);
                    let mut expected = Machine::new(opcode.to_le_bytes().to_vec());
                    expected.cpu.set_cpsr(mode as u32);
                    expected.cpu.regs[3] = 0xf0000032; // switch to IRQ; attempted T write
                    let mut actual = expected.try_fork_for_research().unwrap();
                    (RT_API.interp_one)(&mut expected as *mut _ as *mut c_void);
                    write_psr(
                        &mut actual as *mut _ as *mut c_void,
                        saved as u32,
                        fields,
                        0xf0000032,
                    );
                    actual.cpu.regs[15] += 4;
                    (RT_API.tick)(&mut actual as *mut _ as *mut c_void, 4);
                    assert_eq!(actual.cpu.regs, expected.cpu.regs);
                    assert_eq!(actual.cpu.cpsr, expected.cpu.cpsr);
                    assert_eq!(actual.cpu.spsr(), expected.cpu.spsr());
                    // Switching back reveals whether SP/LR banking was preserved.
                    actual.cpu.set_cpsr(mode as u32);
                    expected.cpu.set_cpsr(mode as u32);
                    assert_eq!(actual.cpu.regs, expected.cpu.regs);
                }
            }
        }
    }

    #[test]
    fn native_bios_wait_and_arithmetic_match_interpreter() {
        for num in [2u32, 5, 6, 8] {
            let mut expected = Machine::new((0xef000000u32 | (num << 16)).to_le_bytes().to_vec());
            expected.cpu.regs[0] = 100;
            expected.cpu.regs[1] = 3;
            let mut actual = expected.try_fork_for_research().unwrap();
            (RT_API.interp_one)(&mut expected as *mut _ as *mut c_void);
            swi(&mut actual as *mut _ as *mut c_void, num, 0x08000004, 4);
            assert_eq!(actual.cpu.regs, expected.cpu.regs);
            assert_eq!(actual.cpu.cpsr, expected.cpu.cpsr);
            assert_eq!(actual.bus.halted, expected.bus.halted);
            assert_eq!(actual.bus.bios_open, expected.bus.bios_open);
            assert_eq!(actual.bus.ewram, expected.bus.ewram);
            assert_eq!(actual.bus.iwram, expected.bus.iwram);
        }
    }
}
