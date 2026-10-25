//! Opt-in protection for resident USA cash sprites using borrowed matrices.
use gba_core::Machine;
pub const CHECK: u32 = 0x0806071f;
const BASE: usize = 0x35cf0;
const STRIDE: usize = 0x68;

pub fn apply(m: &mut Machine) -> bool {
    let offset = m.cpu.regs[8] as usize;
    if m.cpu.regs[2] != 2
        || offset % STRIDE != 0
        || offset / STRIDE >= 4
        || m.cpu.regs[7] != 0x02000000 + (BASE + offset + 0x64) as u32
        || m.bus.rom.get(0xac..0xb0) != Some(b"BGTE")
        || m.bus.rom.get(0x6071e..0x60720) != Some(&[0x00, 0x2a])
    {
        return false;
    }
    let b = &m.bus.ewram;
    let p = BASE + offset;
    let word = |o| u32::from_le_bytes(b[p + o..p + o + 4].try_into().unwrap());
    let sprite_flags = u16::from_le_bytes(b[p + 0x20..p + 0x22].try_into().unwrap());
    let matrix = word(0x34);
    if b[p + 0x58] != 1
        || b[p + 0x5b] != 0
        || b[p + 0x5a] > 21
        || word(0x64) & 2 != 0
        || word(0x64) & 4 == 0
        || word(0x18) == 0
        || sprite_flags & 3 != 2
        || !(0x02022ac0..0x02022e40).contains(&matrix)
        || (matrix - 0x02022ac0) % 28 != 0
        || !spare_resources(b)
    {
        return false;
    }
    // This cash destructor does not free its borrowed matrix. Leave the
    // earlier global pressure check and cleanup of other entities intact;
    // execute the existing CMP/branch with this local cleanup reason cleared.
    m.cpu.regs[2] = 0;
    true
}

// The original pressure check returns early for affine scarcity. Independently
// check its remaining reserves too, rather than masking a simultaneous shortage.
pub(crate) fn spare_resources(b: &[u8]) -> bool {
    let half = |o: usize| {
        b.get(o..o + 2)
            .map(|v| u16::from_le_bytes(v.try_into().unwrap()))
    };
    let word = |o: usize| {
        b.get(o..o + 4)
            .map(|v| u32::from_le_bytes(v.try_into().unwrap()))
    };
    let Some(mut node) = word(0x22f50) else {
        return false;
    };
    let mut palettes = 0;
    while node != 0 {
        if !(0x02022e50..0x02022f50).contains(&node)
            || (node - 0x02022e50) % 16 != 0
            || palettes >= 16
        {
            return false;
        }
        palettes += 1;
        node = word((node - 0x02000000) as usize + 8).unwrap_or(0);
    }
    if palettes <= 1 {
        return false;
    }
    let (Some(capacity), Some(shift), Some(mut index)) =
        (half(0x110c2), half(0x110c4), half(0x110c6))
    else {
        return false;
    };
    if shift > 15 {
        return false;
    }
    let mut free = 0u64;
    let mut seen = 0;
    while index != 0x7777 {
        if index >= capacity || seen >= capacity {
            return false;
        }
        seen += 1;
        let p = 0x110cc + index as usize * 8;
        let (Some(size), Some(next)) = (half(p), half(p + 4)) else {
            return false;
        };
        if size == 0 || size > i16::MAX as u16 {
            return false;
        }
        free += (size as u64) << shift;
        index = next;
    }
    free > 0x5ff
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Machine {
        let mut rom = vec![0; 0x60720];
        rom[0xac..0xb0].copy_from_slice(b"BGTE");
        rom[0x6071e..0x60720].copy_from_slice(&[0, 0x2a]);
        let mut m = Machine::new(rom);
        m.cpu.regs[2] = 2;
        m.cpu.regs[7] = 0x02035d54;
        for (o, v) in [(0x64, 4u32), (0x18, 0x0201cf00), (0x34, 0x02022c10)] {
            m.bus.ewram[BASE + o..BASE + o + 4].copy_from_slice(&v.to_le_bytes());
        }
        m.bus.ewram[BASE + 0x58] = 1;
        m.bus.ewram[BASE + 0x5a] = 17;
        m.bus.ewram[BASE + 0x20] = 0x22;
        for (o, v) in [(0x22f50, 0x02022e50u32), (0x22e58, 0x02022e60)] {
            m.bus.ewram[o..o + 4].copy_from_slice(&v.to_le_bytes());
        }
        for (o, v) in [
            (0x110c2, 1u16),
            (0x110c4, 5),
            (0x110c6, 0),
            (0x110cc, 64),
            (0x110d0, 0x7777),
        ] {
            m.bus.ewram[o..o + 2].copy_from_slice(&v.to_le_bytes());
        }
        m
    }
    #[test]
    fn retains_cash_without_changing_memory_or_value() {
        let mut m = fixture();
        let memory = m.bus.ewram.clone();
        let mut registers = m.cpu.regs;
        assert!(apply(&mut m));
        registers[2] = 0;
        assert_eq!(m.cpu.regs, registers);
        assert_eq!(m.bus.ewram, memory);
    }
    #[test]
    fn preserves_other_pressure_expiry_collection_and_owned_matrix_paths() {
        for reason in [0, 1, 3] {
            let mut m = fixture();
            m.cpu.regs[2] = reason;
            assert!(!apply(&mut m));
        }
        for (o, v) in [
            (0x58, 0),
            (0x5b, 1),
            (0x5a, 22),
            (0x64, 6),
            (0x64, 0),
            (0x20, 0x23),
        ] {
            let mut m = fixture();
            m.bus.ewram[BASE + o] = v;
            assert!(!apply(&mut m));
        }
        let mut m = fixture();
        m.cpu.regs[8] = 4 * STRIDE as u32;
        assert!(!apply(&mut m));
        let mut m = fixture();
        m.cpu.regs[7] += 4;
        assert!(!apply(&mut m));
        let mut m = fixture();
        m.bus.ewram[0x22e58..0x22e5c].fill(0);
        assert!(!apply(&mut m));
        let mut m = fixture();
        m.bus.ewram[0x110cc..0x110ce].copy_from_slice(&32u16.to_le_bytes());
        assert!(!apply(&mut m));
    }
}
