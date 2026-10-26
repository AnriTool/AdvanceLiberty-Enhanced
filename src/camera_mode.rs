//! Original USA camera preference, applied at frame boundaries and recorded.
use gba_core::MemMap;
pub const NAMES: [&str; 4] = ["Auto", "Low", "Medium", "High"];

pub fn set(bus: &mut MemMap, mode: u8) -> Result<(), &'static str> {
    if bus.rom.get(0xac..0xb0) != Some(b"BGTE") || mode > 3 {
        return Err("Invalid USA camera mode");
    }
    // Native callback 08008028 only stores r2 at 02010c60+6.
    bus.ewram[0x10c66] = mode;
    Ok(())
}
pub fn cycle(bus: &mut MemMap) -> Option<u8> {
    let current = *bus.ewram.get(0x10c66)?;
    if current > 3 {
        return None;
    }
    let next = (current + 1) % 4;
    set(bus, next).ok()?;
    Some(next)
}
pub fn changes(text: &str) -> Result<Vec<(u64, u8)>, String> {
    let mut result = Vec::new();
    for line in text.lines().filter_map(|s| s.strip_prefix("# camera ")) {
        let (frame, mode) = line.split_once(' ').ok_or("Invalid camera event")?;
        let frame = frame.parse::<u64>().map_err(|_| "Invalid camera frame")?;
        let mode = mode.parse::<u8>().map_err(|_| "Invalid camera mode")?;
        if mode > 3 || result.last().is_some_and(|&(last, _)| last > frame) {
            return Err("Invalid camera mode or event ordering".into());
        }
        result.push((frame, mode));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn camera_cycle_and_recorded_events() {
        let mut rom = vec![0; 0x200];
        rom[0xac..0xb0].copy_from_slice(b"BGTE");
        let mut bus = MemMap::new(rom);
        let original = bus.ewram.clone();
        for mode in [1, 2, 3, 0] {
            assert_eq!(cycle(&mut bus), Some(mode));
        }
        assert_eq!(bus.ewram, original);
        assert!(set(&mut bus, 4).is_err());
        bus.rom[0xac] = 0;
        assert_eq!(cycle(&mut bus), None);
        assert_eq!(bus.ewram, original);
        assert_eq!(
            changes("gba-input v1\n0 3ff\n# camera 12 1\n# camera 12 2\n# camera 20 3\n").unwrap(),
            vec![(12, 1), (12, 2), (20, 3)]
        );
        for invalid in [
            "# camera 1 4",
            "# camera 2 0\n# camera 1 1",
            "# camera no 1",
        ] {
            assert!(changes(invalid).is_err());
        }
    }
    #[test]
    #[ignore = "requires local USA ROM"]
    fn native_camera_callback_only_changes_the_preference() {
        let rom = std::fs::read("roms/usa.gba").unwrap();
        // Verify the exact handler whose behavior the host shortcut mirrors.
        assert_eq!(
            &rom[0x8028..0x8034],
            &[1, 72, 130, 113, 1, 32, 112, 71, 96, 12, 1, 2]
        );
        for mode in 0..4u8 {
            let mut native = gba_core::Machine::new(rom.clone());
            native.cpu.set_thumb(true);
            native.cpu.regs[15] = 0x08008028;
            native.cpu.regs[2] = mode as u32;
            native.cpu.regs[14] = 0x08000101;
            let mut host = MemMap::new(rom.clone());
            set(&mut host, mode).unwrap();
            for _ in 0..4 {
                native.step();
            }
            assert_eq!(host.ewram, native.bus.ewram);
            assert_eq!(native.cpu.regs[15], 0x08000100);
        }
    }
}
