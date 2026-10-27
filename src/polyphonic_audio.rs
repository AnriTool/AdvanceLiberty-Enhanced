//! USA PCM driver sidecar: mix queued PCM effects suppressed by priority.
//! The original driver owns allocation, positions, loops, volume and lifetime.
use gba_core::{Machine, MemMap};
use std::ffi::c_void;
const DRIVER: usize = 0x27330;
const POOL: u32 = 0x02027344;
const STRIDE: u32 = 44;
const BLOCK: usize = 160;
fn word(bus: &MemMap, offset: usize) -> u32 {
    u32::from_le_bytes(bus.ewram[offset..offset + 4].try_into().unwrap())
}
fn buffer(bus: &MemMap, group: usize) -> Option<u32> {
    let p = word(bus, DRIVER + if group == 0 { 0xb9c } else { 0xba0 });
    (0x03000000..=0x03007ec0).contains(&p).then_some(p)
}
fn slot(pointer: u32) -> Option<usize> {
    let offset = pointer.checked_sub(POOL)?;
    (offset < 64 * STRIDE && offset % STRIDE == 0).then_some((pointer - 0x02000000) as usize)
}
fn prefix(bus: &mut MemMap, group: usize) {
    let Some(buffer) = buffer(bus, group) else {
        return;
    };
    let (offset, count) = if group == 0 {
        (BLOCK as u32, 64)
    } else {
        (word(bus, DRIVER + 0xba8), word(bus, DRIVER + 0xbac).min(64))
    };
    if offset > 256 {
        return;
    }
    let overlay = bus.pcm_overlay.as_mut().unwrap();
    for i in 0..count {
        let value = overlay
            .memory
            .get(&(buffer + offset + i))
            .copied()
            .unwrap_or(0.0);
        overlay.memory.insert(buffer + i, value);
    }
}
fn prepare(bus: &mut MemMap) {
    prepare_group(bus, 0);
    prepare_group(bus, 2);
}
fn prepare_group(bus: &mut MemMap, group: usize) {
    let Some(buffer) = buffer(bus, group) else {
        return;
    };
    let mut mixed = [0.0f32; 256];
    let mut pointer = word(bus, DRIVER + 0xb8c + group * 4);
    let output_step = if group == 0 {
        0xa000
    } else {
        slot(pointer)
            .map(|p| word(bus, p + 0x18))
            .filter(|s| (0x4000..=0x10000).contains(s))
            .unwrap_or(0xa000)
    };
    let count = (output_step >> 8) as usize;
    let mut visited = 0u64;
    if let Some(head) = slot(pointer) {
        visited |= 1u64 << ((pointer - POOL) / STRIDE);
        pointer = word(bus, head + 0x24); // never duplicate the audible voice
    } else {
        pointer = 0;
    }
    while let Some(p) = slot(pointer) {
        let bit = 1u64 << ((pointer - POOL) / STRIDE);
        if visited & bit != 0 {
            break;
        }
        visited |= bit;
        let sample = word(bus, p);
        let position = word(bus, p + 4) as u64;
        let step = if group == 0 {
            0xa000
        } else {
            word(bus, p + 0x18)
        };
        let length = word(bus, p + 0x0c).min(word(bus, p + 0x10)) as usize;
        let volume = u16::from_le_bytes(bus.ewram[p + 8..p + 10].try_into().unwrap()) as f32;
        // Paused voices advance neither in the driver nor in the sidecar.
        // Only the verified raw signed-8-bit PCM path is supported here.
        if bus.ewram[p + 0x21] & 0x10 == 0 && volume <= 256.0 && length <= 0x100000 {
            if let Some(offset) = sample.checked_sub(0x08000000).map(|v| v as usize) {
                if offset >= 16 && bus.rom.get(offset - 16..offset - 12) == Some(&[0, 0, 0, 0]) {
                    if let Some(data) = bus.rom.get(offset..offset.saturating_add(length)) {
                        // Original completion/loop restart occurs at the next
                        // tick; the remainder of this chunk is silence.
                        for (i, output) in mixed[..count].iter_mut().enumerate() {
                            // Group 2 voices may have independent pitch. Map
                            // each to the selected head's actual timer grid.
                            let index = ((position
                                + i as u64 * 256 * step as u64 / output_step as u64)
                                >> 8) as usize;
                            if let Some(&value) = data.get(index) {
                                *output += value as i8 as f32 * volume / 256.0;
                            }
                        }
                    }
                }
            }
        }
        pointer = word(bus, p + 0x24);
    }
    let overlay = bus.pcm_overlay.as_mut().unwrap();
    for (i, &value) in mixed[..count].iter().enumerate() {
        overlay.memory.insert(buffer + 64 + i as u32, value);
    }
}
pub extern "C" fn read32(m: *mut c_void, address: u32) -> u32 {
    if matches!(address, 0x02027ebc | 0x02027ecc | 0x02027ed0) {
        let machine = unsafe { &mut *(m as *mut Machine) };
        if machine.bus.pcm_overlay.is_some() {
            match machine.cpu.regs[15] {
                0x0803242e => prefix(&mut machine.bus, 0),
                0x0803244e => prefix(&mut machine.bus, 2),
                0x08032496 => prepare(&mut machine.bus),
                _ => (),
            }
        }
    }
    crate::mouse_aim::read32(m, address)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn put(bus: &mut MemMap, offset: usize, value: u32) {
        bus.ewram[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    #[test]
    fn second_effect_group_keeps_independent_pitch_and_prefix_length() {
        let mut rom = vec![0; 2048];
        for i in 0..320 {
            rom[16 + i] = (i % 80) as u8;
        }
        let mut m = Machine::new(rom);
        let b = &mut m.bus;
        b.pcm_overlay = Some(Default::default());
        put(b, DRIVER + 0xba0, 0x030067e0);
        put(b, DRIVER + 0xb94, POOL);
        let head = slot(POOL).unwrap();
        let tail = slot(POOL + STRIDE).unwrap();
        put(b, head + 0x18, 160 << 8);
        put(b, head + 0x24, POOL + STRIDE);
        put(b, tail, 0x08000010);
        put(b, tail + 0xc, 320);
        put(b, tail + 0x10, 320);
        put(b, tail + 0x18, 80 << 8);
        put(b, tail + 4, 10 << 8);
        b.ewram[tail + 8..tail + 10].copy_from_slice(&256u16.to_le_bytes());
        prepare(b);
        for i in 0..160 {
            assert_eq!(
                b.pcm_overlay.as_ref().unwrap().memory[&(0x03006820 + i)],
                ((10 + i / 2) % 80) as f32
            );
        }
        put(b, DRIVER + 0xba8, 160);
        put(b, DRIVER + 0xbac, 32);
        prefix(b, 2);
        assert_eq!(b.pcm_overlay.as_ref().unwrap().memory[&0x030067e0], 58.0);
        assert!(!b
            .pcm_overlay
            .as_ref()
            .unwrap()
            .memory
            .contains_key(&0x03006800));
    }
    #[test]
    fn suppressed_voices_follow_position_gain_pause_end_and_driver_prefix() {
        let mut rom = vec![0; 2048];
        rom[16..336].fill(40);
        let mut m = Machine::new(rom);
        let b = &mut m.bus;
        b.pcm_overlay = Some(Default::default());
        put(b, DRIVER + 0xb9c, 0x03006700);
        put(b, DRIVER + 0xb8c, POOL);
        for i in 0..3 {
            let p = slot(POOL + i * STRIDE).unwrap();
            put(b, p, 0x08000010);
            put(b, p + 0xc, 320);
            put(b, p + 0x10, 320);
            b.ewram[p + 8..p + 10].copy_from_slice(&128u16.to_le_bytes());
            put(b, p + 0x24, if i < 2 { POOL + (i + 1) * STRIDE } else { 0 });
        }
        let secondary = slot(POOL + STRIDE).unwrap();
        put(b, secondary + 4, 300 << 8); // 20 samples left
        b.ewram[slot(POOL + 2 * STRIDE).unwrap() + 0x21] = 0x10; // paused
        let original = b.ewram.clone();
        prepare(b);
        assert_eq!(b.ewram, original);
        let overlay = b.pcm_overlay.as_ref().unwrap();
        for i in 0..160 {
            assert_eq!(
                overlay.memory[&(0x03006740 + i)],
                if i < 20 { 20.0 } else { 0.0 }
            );
        }
        b.pcm_overlay
            .as_mut()
            .unwrap()
            .memory
            .insert(0x030067a0, 17.0);
        prefix(b, 0);
        assert_eq!(b.pcm_overlay.as_ref().unwrap().memory[&0x03006700], 17.0);
        // A malformed cycle cannot duplicate the audible head or hang.
        put(b, secondary + 0x24, POOL);
        prepare(b);
        assert_eq!(b.pcm_overlay.as_ref().unwrap().memory[&0x03006740], 20.0);
    }
}
