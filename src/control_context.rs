//! Read-only USA controlled-entity checks, independent of the presentation camera.
use gba_core::MemMap;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Context {
    #[default]
    Menu,
    OnFoot,
    Vehicle,
}

fn word(bus: &MemMap, address: u32) -> Option<u32> {
    if address & 3 != 0 {
        return None;
    }
    let offset = address.checked_sub(0x02000000)? as usize;
    Some(u32::from_le_bytes(
        bus.ewram
            .get(offset..offset.checked_add(4)?)?
            .try_into()
            .ok()?,
    ))
}

pub fn detect(bus: &MemMap) -> Context {
    // USA 08001484..080014a0 reads this same player pointer and +8 type byte.
    // The pointer changes to the occupied vehicle and back when entering/exiting.
    if bus.rom.get(0xac..0xb0) != Some(b"BGTE") {
        return Context::Menu;
    }
    let display = u16::from_le_bytes(bus.io[..2].try_into().unwrap());
    // Forced blank can be a single upload phase during ordinary driving;
    // it must not change the input layout while a movement key is held.
    if display & 0x1407 != 0x1401 {
        return Context::Menu;
    }
    let Some(actor) = word(bus, 0x02000f10) else {
        return Context::Menu;
    };
    let read = |offset: u32| actor.checked_add(offset).and_then(|a| word(bus, a));
    let Some(kind) = read(8).map(|v| v & 255) else {
        return Context::Menu;
    };
    let context = match kind {
        2 => Context::OnFoot,
        4 => Context::Vehicle,
        _ => return Context::Menu,
    };
    let Some(descriptor) = read(16) else {
        return Context::Menu;
    };
    let Some(offset) = descriptor.checked_sub(0x08000000).map(|v| v as usize) else {
        return Context::Menu;
    };
    if descriptor & 3 != 0
        || bus.rom.get(offset..offset.saturating_add(4)) != Some(kind.to_le_bytes().as_slice())
    {
        return Context::Menu;
    }
    let (Some(position), Some(animation)) = (read(24), read(28)) else {
        return Context::Menu;
    };
    if word(bus, position).is_none()
        || word(bus, animation) != Some(position)
        || animation.checked_add(48).and_then(|a| word(bus, a)) != Some(actor)
    {
        return Context::Menu;
    }
    context
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample(kind: u32) -> MemMap {
        let mut rom = vec![0; 0x200];
        rom[0xac..0xb0].copy_from_slice(b"BGTE");
        rom[0x100..0x104].copy_from_slice(&kind.to_le_bytes());
        let mut bus = MemMap::new(rom);
        bus.io[..2].copy_from_slice(&0x1741u16.to_le_bytes());
        let mut put = |address, value: u32| {
            let offset = (address - 0x02000000) as usize;
            bus.ewram[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        };
        put(0x02000f10, 0x02001000);
        put(0x02001008, kind | 0x1800);
        put(0x02001010, 0x08000100);
        put(0x02001018, 0x02002000);
        put(0x0200101c, 0x02002100);
        put(0x02002100, 0x02002000);
        put(0x02002130, 0x02001000);
        bus
    }
    #[test]
    fn follows_controlled_actor_not_nearby_cars_or_fixed_allocation() {
        assert_eq!(detect(&sample(2)), Context::OnFoot);
        assert_eq!(detect(&sample(4)), Context::Vehicle);
        assert_eq!(detect(&sample(3)), Context::Menu);
        let mut bus = sample(4);
        bus.ewram[0x2130..0x2134].copy_from_slice(&0x02001004u32.to_le_bytes());
        assert_eq!(detect(&bus), Context::Menu);
    }
    #[test]
    fn invalid_pointers_rom_and_menu_output_keep_standard_buttons() {
        for pointer in [0, 0x0203fffc, 0x02001001, 0x03001000, u32::MAX] {
            let mut bus = sample(4);
            bus.ewram[0xf10..0xf14].copy_from_slice(&pointer.to_le_bytes());
            assert_eq!(detect(&bus), Context::Menu);
        }
        for display in [0u16, 0x1740, 0x0101, 0x1341, 0x0741] {
            let mut bus = sample(4);
            bus.io[..2].copy_from_slice(&display.to_le_bytes());
            assert_eq!(detect(&bus), Context::Menu);
        }
        let mut upload = sample(4);
        upload.io[..2].copy_from_slice(&0x17c1u16.to_le_bytes());
        assert_eq!(detect(&upload), Context::Vehicle);
        let mut bus = sample(4);
        bus.rom[0xac] = b'X';
        assert_eq!(detect(&bus), Context::Menu);
    }
}
