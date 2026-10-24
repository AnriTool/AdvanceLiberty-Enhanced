//! Opt-in USA on-foot aim. Movement reads always retain the game's own angle.
use gba_core::{capi::RT_API, Machine, MemMap};
use std::{cell::Cell, ffi::c_void, path::Path};

fn word(bus: &MemMap, address: u32) -> Option<u32> {
    let i = address.checked_sub(0x02000000)? as usize;
    Some(u32::from_le_bytes(
        bus.ewram.get(i..i.checked_add(4)?)?.try_into().ok()?,
    ))
}

pub fn player(bus: &MemMap) -> Option<(u32, u32, [f64; 2])> {
    if crate::control_context::detect(bus) != crate::control_context::Context::OnFoot {
        return None;
    }
    let actor = word(bus, 0x02000f10)?;
    let position = word(bus, actor + 24)?;
    Some((
        actor,
        position,
        [
            word(bus, position)? as i32 as f64 / 65536.0,
            word(bus, position + 4)? as i32 as f64 / 65536.0,
        ],
    ))
}

/// The actual presented actor root, in the SDL logical coordinate system.
#[derive(Clone, Copy, Debug)]
pub struct Projection {
    pub actor: u32,
    pub screen: [f64; 2],
    pub world: [f64; 2],
    pub pixels_per_world: [f64; 2],
    pub size: [u32; 2],
}
impl Projection {
    pub fn original(bus: &MemMap) -> Option<Self> {
        let (actor, _, world) = player(bus)?;
        let iw = |o| i32::from_le_bytes(bus.iwram[o..o + 4].try_into().unwrap()) as f64;
        let scale = [iw(0x60) / 65536.0, iw(0x64) / 65536.0];
        Some(Self {
            actor,
            world,
            screen: std::array::from_fn(|i| (world[i].floor() - iw(0x50 + i * 4)) * scale[i]),
            pixels_per_world: scale,
            size: [240, 160],
        })
    }
    pub fn angle(self, actor: u32, current: [f64; 2], cursor: [f64; 2]) -> Option<u16> {
        if actor != self.actor
            || (0..2).any(|i| {
                !cursor[i].is_finite()
                    || cursor[i] < 0.0
                    || cursor[i] >= self.size[i] as f64
                    || !self.pixels_per_world[i].is_finite()
                    || self.pixels_per_world[i] <= 0.0
            })
        {
            return None;
        }
        let screen_delta = [cursor[0] - self.screen[0], cursor[1] - self.screen[1]];
        // Avoid an unstable heading when the pointer is on the actor's feet.
        let pixel_scale = self.size[1] as f64 / 160.0;
        if screen_delta[0].hypot(screen_delta[1]) < 3.0 * pixel_scale {
            return None;
        }
        let delta: [f64; 2] = std::array::from_fn(|i| {
            self.world[i] + screen_delta[i] / self.pixels_per_world[i] - current[i]
        });
        if !delta.iter().all(|v| v.is_finite()) || delta[0].hypot(delta[1]) < 0.01 {
            return None;
        }
        Some(
            ((delta[0].atan2(-delta[1]) / std::f64::consts::TAU * 1024.0).round() as i32)
                .rem_euclid(1024) as u16,
        )
    }
}

#[derive(Clone, Copy)]
struct Aim {
    actor: u32,
    position: u32,
    angle: u32,
}
thread_local! { static ACTIVE: Cell<Option<Aim>> = const {Cell::new(None)}; }
pub fn set(bus: &MemMap, angle: Option<u16>) {
    let aim = angle.filter(|&a| a < 1024).and_then(|angle| {
        let (actor, position, _) = player(bus)?;
        Some(Aim {
            actor,
            position,
            angle: (angle as u32) << 16,
        })
    });
    ACTIVE.with(|a| a.set(aim));
}
fn override_angle(bus: &MemMap, pc: u32, address: u32) -> Option<u32> {
    // Verify the exact load instruction as well as its PC. Other reads,
    // especially 0801d21a (movement), must observe unmodified guest memory.
    let opcode = match pc {
        0x08018bb8 => 0x68c5u16,
        0x0801453e => 0x6800,
        0x08024a24 | 0x08024c26 => 0x68c0,
        0x080270bc => 0x68c0,
        _ => return None,
    };
    let aim = ACTIVE.with(Cell::get)?;
    let i = (pc - 0x08000000) as usize;
    if address != aim.position + 12
        || bus.rom.get(i..i + 2) != Some(opcode.to_le_bytes().as_slice())
        || word(bus, 0x02000f10) != Some(aim.actor)
        || word(bus, aim.actor + 24) != Some(aim.position)
        || word(bus, aim.actor + 8).map(|v| v & 255) != Some(2)
    {
        return None;
    }
    Some(aim.angle)
}
pub extern "C" fn read32(ptr: *mut c_void, address: u32) -> u32 {
    let value = (RT_API.read32)(ptr, address);
    let m = unsafe { &*(ptr as *const Machine) };
    override_angle(&m.bus, m.cpu.regs[15], address).unwrap_or(value)
}

pub fn load(path: &Path) -> Result<Vec<(u64, Option<u16>)>, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines();
    if lines.next() != Some("gba-aim v1") {
        return Err("Invalid mouse aim header".into());
    }
    let mut points = Vec::new();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        let (frame, angle) = line.split_once(' ').ok_or("Invalid aim row")?;
        let frame = frame.parse::<u64>()?;
        let angle = if angle == "none" {
            None
        } else {
            Some(angle.parse::<u16>()?)
        };
        if angle.is_some_and(|a| a >= 1024) || points.last().is_some_and(|&(f, _)| f >= frame) {
            return Err("Invalid aim angle/order".into());
        }
        points.push((frame, angle));
    }
    if points.first().map(|p| p.0) != Some(0) {
        return Err("Aim stream must start at frame zero".into());
    }
    Ok(points)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn viewport_angles_target_world_and_delayed_actor() {
        let p = Projection {
            actor: 1,
            screen: [427.0, 240.0],
            world: [100.0, 100.0],
            pixels_per_world: [3.0, 3.0],
            size: [854, 480],
        };
        for (cursor, angle) in [
            ([427.0, 180.0], 0),
            ([487.0, 240.0], 256),
            ([427.0, 300.0], 512),
            ([367.0, 240.0], 768),
            ([487.0, 180.0], 128),
        ] {
            assert_eq!(p.angle(1, [100.0, 100.0], cursor), Some(angle));
        }
        assert_eq!(p.angle(1, [110.0, 100.0], [487.0, 210.0]), Some(128));
        assert_eq!(p.angle(2, [100.0, 100.0], [487.0, 240.0]), None);
        assert_eq!(p.angle(1, [100.0, 100.0], [427.0, 240.0]), None);
        assert_eq!(p.angle(1, [100.0, 100.0], [-1.0, 240.0]), None);
        assert_eq!(p.angle(1, [100.0, 100.0], [854.0, 240.0]), None);
    }
    #[test]
    fn selective_reads_never_turn_movement_or_other_actors() {
        let mut bus = MemMap::new(vec![0; 0x40000]);
        let put = |bus: &mut MemMap, a: u32, v: u32| {
            let i = (a - 0x02000000) as usize;
            bus.ewram[i..i + 4].copy_from_slice(&v.to_le_bytes());
        };
        put(&mut bus, 0x02000f10, 0x02001000);
        put(&mut bus, 0x02001018, 0x02002000);
        put(&mut bus, 0x02001008, 2);
        bus.rom[0x24a24..0x24a26].copy_from_slice(&0x68c0u16.to_le_bytes());
        ACTIVE.with(|a| {
            a.set(Some(Aim {
                actor: 0x02001000,
                position: 0x02002000,
                angle: 128 << 16,
            }))
        });
        assert_eq!(
            override_angle(&bus, 0x08024a24, 0x0200200c),
            Some(128 << 16)
        );
        assert_eq!(override_angle(&bus, 0x0801d21a, 0x0200200c), None);
        assert_eq!(override_angle(&bus, 0x08024a24, 0x0200210c), None);
        put(&mut bus, 0x02000f10, 0x02001100);
        assert_eq!(override_angle(&bus, 0x08024a24, 0x0200200c), None);
        ACTIVE.with(|a| a.set(None));
    }
}
