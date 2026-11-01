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
    // The driver relocates its IWRAM buffers when scene resources change.
    // DMA/FIFO samples already in flight own their values independently; old
    // shadow addresses must not accumulate across missions or enter snapshots.
    let buffers = [buffer(bus, 0), buffer(bus, 2)];
    bus.pcm_overlay
        .as_mut()
        .unwrap()
        .memory
        .retain(|&address, _| {
            buffers
                .iter()
                .flatten()
                .any(|&start| (start..start + 320).contains(&address))
        });
    let mut audible = std::collections::HashSet::new();
    // A queued copy of an already audible sample is a replacement, not a
    // missing independent effect. Never amplify it with its older copies.
    for group in 0..4 {
        if let Some(head) = slot(word(bus, DRIVER + 0xb8c + group * 4)) {
            audible.insert(word(bus, head));
        }
    }
    let recovered: Vec<_> = bus
        .pcm_overlay
        .as_ref()
        .unwrap()
        .voices
        .iter()
        .filter(|v| !audible.contains(&v.sample))
        .cloned()
        .collect();
    for voice in &recovered {
        audible.insert(voice.sample);
    }
    prepare_group(bus, 0, &mut audible, &recovered);
    prepare_group(bus, 2, &mut audible, &[]);
    let voices = &mut bus.pcm_overlay.as_mut().unwrap().voices;
    for voice in voices.iter_mut() {
        voice.position = (voice.position + BLOCK as u32).min(voice.length);
    }
    voices.retain(|v| v.position < v.length);
}
fn prepare_group(
    bus: &mut MemMap,
    group: usize,
    audible: &mut std::collections::HashSet<u32>,
    recovered: &[gba_core::pcm_overlay::PcmVoice],
) {
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
        if bus.ewram[p + 0x21] & 0x10 == 0
            && volume > 0.0
            && volume <= 256.0
            && length <= 0x100000
            && !audible.contains(&sample)
        {
            if let Some(offset) = sample.checked_sub(0x08000000).map(|v| v as usize) {
                if offset >= 16 && bus.rom.get(offset - 16..offset - 12) == Some(&[0, 0, 0, 0]) {
                    if let Some(data) = bus.rom.get(offset..offset.saturating_add(length)) {
                        audible.insert(sample);
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
    for voice in recovered {
        let start = (voice.sample - 0x08000000) as usize;
        if let Some(data) = bus.rom.get(start..start + voice.length as usize) {
            for (i, output) in mixed[..count].iter_mut().enumerate() {
                if let Some(&value) = data.get(voice.position as usize + i) {
                    *output += value as i8 as f32 * voice.volume as f32 / 256.0;
                }
            }
        }
    }
    let overlay = bus.pcm_overlay.as_mut().unwrap();
    for (i, &value) in mixed[..count].iter().enumerate() {
        overlay.memory.insert(buffer + 64 + i as u32, value);
    }
}
// The high-level weapon sound path can reject a shot before the PCM allocator
// sees it (020001dc is nonzero during certain competing vehicle sounds).
// Observe that rejected request and supply host audio without clearing the guest
// flag, allocating a guest voice, or changing the returned read value.
pub extern "C" fn read8(m: *mut c_void, address: u32) -> u32 {
    let value = (gba_core::capi::RT_API.read8)(m, address);
    if value != 0 && address == 0x020001dc {
        let machine = unsafe { &mut *(m as *mut Machine) };
        if machine.cpu.regs[15] == 0x08034d72 && machine.bus.pcm_overlay.is_some() {
            recover_weapon(&mut machine.bus, machine.cpu.regs[4], machine.cpu.regs[1]);
        }
    }
    value
}
fn recover_weapon(bus: &mut MemMap, actor: u32, sound: u32) {
    if word(bus, 0xf10) != actor || !(201..=472).contains(&sound) {
        return;
    }
    let entry = 0xca74dc + (sound - 201) as usize * 4;
    let Some(pointer) = bus.rom.get(entry..entry + 4) else {
        return;
    };
    let Some(header) = u32::from_le_bytes(pointer.try_into().unwrap()).checked_sub(0x08000000)
    else {
        return;
    };
    let offset = header as usize;
    let Some(data) = bus.rom.get(offset..offset + 16) else {
        return;
    };
    if data[..4] != [0; 4] {
        return;
    }
    let length = u32::from_le_bytes(data[12..16].try_into().unwrap()) / BLOCK as u32 * BLOCK as u32;
    if length == 0
        || length > 0x100000
        || bus
            .rom
            .get(offset + 16..offset + 16 + length as usize)
            .is_none()
    {
        return;
    }
    let setting = bus.ewram[0x10c61] as usize;
    let Some(&shift) = bus.rom.get(0xca665c + setting) else {
        return;
    };
    if shift > 8 {
        return;
    }
    let voice = gba_core::pcm_overlay::PcmVoice {
        sample: 0x08000000 + header + 16,
        position: 0,
        length,
        volume: 256 >> shift,
    };
    let voices = &mut bus.pcm_overlay.as_mut().unwrap().voices;
    voices.retain(|old| old.sample != voice.sample);
    if voices.len() == 16 {
        voices.remove(0);
    }
    voices.push(voice);
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
    #[test]
    fn blocked_player_weapon_is_host_only_retriggered_once_and_follows_sfx_volume() {
        let mut rom = vec![0; 16 * 1024 * 1024];
        rom[0xca74dc..0xca74e0].copy_from_slice(&0x08001000u32.to_le_bytes());
        rom[0x100c..0x1010].copy_from_slice(&480u32.to_le_bytes());
        rom[0x1010..0x11f0].fill(40);
        rom[0xca665c] = 1; // the same attenuation table used by the guest allocator
        let mut m = Machine::new(rom);
        m.bus.pcm_overlay = Some(Default::default());
        put(&mut m.bus, 0xf10, 0x02010000);
        put(&mut m.bus, DRIVER + 0xb9c, 0x03006700);
        m.bus.ewram[0x1dc] = 7;
        m.cpu.regs[15] = 0x08034d72;
        m.cpu.regs[4] = 0x02010000;
        m.cpu.regs[1] = 201;
        let original = m.bus.ewram.clone();
        let ptr = &mut m as *mut Machine as *mut c_void;
        assert_eq!(read8(ptr, 0x020001dc), 7);
        prepare(&mut m.bus);
        assert_eq!(
            m.bus.pcm_overlay.as_ref().unwrap().memory[&0x03006740],
            20.0
        );
        assert_eq!(m.bus.pcm_overlay.as_ref().unwrap().voices[0].position, 160);
        let encoded = serde_json::to_vec(m.bus.pcm_overlay.as_ref().unwrap()).unwrap();
        let restored: gba_core::pcm_overlay::PcmOverlay = serde_json::from_slice(&encoded).unwrap();
        assert!(restored.valid_snapshot([0, 0]));
        assert_eq!(restored.voices[0].sample, 0x08001010);
        assert_eq!(restored.voices[0].position, 160);
        let mut legacy: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
        legacy.as_object_mut().unwrap().remove("voices");
        let legacy: gba_core::pcm_overlay::PcmOverlay = serde_json::from_value(legacy).unwrap();
        assert!(legacy.voices.is_empty());
        assert_eq!(read8(ptr, 0x020001dc), 7);
        assert_eq!(m.bus.pcm_overlay.as_ref().unwrap().voices.len(), 1);
        assert_eq!(m.bus.pcm_overlay.as_ref().unwrap().voices[0].position, 0);
        assert_eq!(m.bus.ewram, original);
        for _ in 0..3 {
            prepare(&mut m.bus);
        }
        assert!(m.bus.pcm_overlay.as_ref().unwrap().voices.is_empty());
        m.cpu.regs[4] = 0x02020000; // do not replay every NPC at the player's loudness
        assert_eq!(read8(ptr, 0x020001dc), 7);
        assert!(m.bus.pcm_overlay.as_ref().unwrap().voices.is_empty());
        m.cpu.regs[4] = 0x02010000;
        m.bus.ewram[0x1dc] = 0; // an accepted request must use the original driver
        assert_eq!(read8(ptr, 0x020001dc), 0);
        assert!(m.bus.pcm_overlay.as_ref().unwrap().voices.is_empty());
    }

    #[test]
    fn relocating_driver_buffers_discards_old_shadow_memory_and_keeps_snapshots_valid() {
        let mut m = Machine::new(vec![0; 2048]);
        let b = &mut m.bus;
        b.pcm_overlay = Some(Default::default());
        for start in [0x03006700, 0x03004f00, 0x03000100, 0x03006700] {
            put(b, DRIVER + 0xb9c, start);
            put(b, DRIVER + 0xba0, start + 224);
            b.pcm_overlay.as_mut().unwrap().memory.insert(start, 19.0);
            prepare(b);
            let overlay = b.pcm_overlay.as_ref().unwrap();
            assert_eq!(overlay.memory[&start], 19.0); // current prefix is retained
            assert!(overlay
                .memory
                .keys()
                .all(|a| (start..start + 544).contains(a)));
            assert!(overlay.valid_snapshot([0, 0]));
        }
    }

    #[test]
    fn repeated_effects_are_not_stacked_or_duplicated_across_groups() {
        let mut rom = vec![0; 4096];
        rom[16..336].fill(40);
        let mut m = Machine::new(rom);
        let b = &mut m.bus;
        b.pcm_overlay = Some(Default::default());
        put(b, DRIVER + 0xb9c, 0x03006700);
        put(b, DRIVER + 0xba0, 0x030067e0);
        put(b, DRIVER + 0xb8c, POOL);
        for i in 0..3 {
            let p = slot(POOL + i * STRIDE).unwrap();
            put(b, p, 0x08000010);
            put(b, p + 0xc, 320);
            put(b, p + 0x10, 320);
            b.ewram[p + 8..p + 10].copy_from_slice(&256u16.to_le_bytes());
            put(b, p + 0x18, 160 << 8);
            put(b, p + 0x24, if i < 2 { POOL + (i + 1) * STRIDE } else { 0 });
        }
        prepare(b);
        assert_eq!(b.pcm_overlay.as_ref().unwrap().memory[&0x03006740], 0.0);
        put(b, slot(POOL).unwrap(), 0x08000810);
        prepare(b);
        assert_eq!(b.pcm_overlay.as_ref().unwrap().memory[&0x03006740], 40.0); // one, not two
        put(b, DRIVER + 0xb94, POOL + STRIDE); // already audible on the other FIFO
        prepare(b);
        assert_eq!(b.pcm_overlay.as_ref().unwrap().memory[&0x03006740], 0.0);
        assert_eq!(b.pcm_overlay.as_ref().unwrap().memory[&0x03006820], 0.0);
    }
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
        put(b, slot(POOL).unwrap(), 0x08000410); // a different audible sample
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
