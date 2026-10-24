//! Read-only USA world-anchor/OAM checks and scene-copy OBJ rendering.
use crate::building_projection::{bytes, word};
use gba_core::capi::RT_API;
use gba_core::{ppu, Machine, MemMap};
use std::sync::Arc;
use std::{cell::RefCell, ffi::c_void};
use std::{collections::HashMap, io::Write, path::Path};
thread_local! {static POOL_WRITES:RefCell<Option<Vec<String>>>=const {RefCell::new(None)};}
pub struct PoolGuard;
pub fn pool_guard() -> PoolGuard {
    POOL_WRITES.with(|p| *p.borrow_mut() = Some(Vec::new()));
    PoolGuard
}
impl Drop for PoolGuard {
    fn drop(&mut self) {
        POOL_WRITES.with(|p| *p.borrow_mut() = None);
    }
}
fn pool_write(m: *mut c_void, address: u32, width: u32, value: u32) {
    if !(0x0201cec0..0x0201d6c0).contains(&address) {
        return;
    }
    let m = unsafe { &*(m as *const Machine) };
    POOL_WRITES.with(|p| {
        if let Some(rows) = p.borrow_mut().as_mut() {
            rows.push(format!(
                "{},{:08x},{:08x},{address:08x},{width},{value:08x},{}",
                m.bus.frames,
                m.cpu.regs[15],
                m.cpu.regs[14],
                m.cpu.regs[..13]
                    .iter()
                    .map(|v| format!("{v:08x}"))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
    });
}
pub extern "C" fn write8(m: *mut c_void, a: u32, v: u32) {
    pool_write(m, a, 1, v);
    (RT_API.write8)(m, a, v);
}
pub extern "C" fn write16(m: *mut c_void, a: u32, v: u32) {
    pool_write(m, a, 2, v);
    (RT_API.write16)(m, a, v);
}
pub extern "C" fn write32(m: *mut c_void, a: u32, v: u32) {
    pool_write(m, a, 4, v);
    (RT_API.write32)(m, a, v);
}
fn half(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn axis(world: i32, offset: i8, bound: i32, scale: i32, camera: i32) -> (i32, f64) {
    let integer = (world >> 16)
        .wrapping_add(offset as i32)
        .wrapping_sub(bound)
        .wrapping_mul(scale)
        >> 16;
    let bound = bound as f64 + camera as f64 / 65536.0 - (camera >> 16) as f64;
    let precise = (world as f64 / 65536.0 + offset as f64 - bound) * scale as f64 / 65536.0;
    (integer, precise)
}
// USA animation builder copies the actor position before applying pose offsets.
// Observe the copy itself, not a guessed relationship between nearby sprites.
pub const ACTOR_POSITION_COPY: u32 = 0x080270b7;
pub const VEHICLE_UPDATE: u32 = 0x08016ef1;
fn ew_word(source: &MemMap, address: u32) -> Option<u32> {
    let offset = address.checked_sub(0x02000000)? as usize;
    Some(u32::from_le_bytes(
        source.ewram.get(offset..offset + 4)?.try_into().ok()?,
    ))
}
#[derive(Clone, Copy)]
struct RootBinding {
    parent: u32,
    position: u32,
    pose_position: u32,
    frame: u64,
    vehicle: bool,
    pose_world: Option<[i32; 2]>,
}
#[derive(Default)]
pub struct MotionRoots {
    bindings: HashMap<u32, RootBinding>,
}
impl MotionRoots {
    pub fn observe(&mut self, m: &Machine) {
        let r = &m.cpu.regs;
        let owner = r[6].wrapping_add(4);
        self.bindings.remove(&owner);
        let Ok(parent) = word(m, r[13].wrapping_add(16)) else {
            return;
        };
        let parent = parent as u32;
        let position = r[0].wrapping_sub(12); // LDMIA r0!, {r3,r4,r7} just completed
        if ew_word(&m.bus, parent.wrapping_add(24)) != Some(position)
            || ew_word(&m.bus, owner) != Some(r[1])
            || r[1] != r[6].wrapping_add(76)
            || ew_word(&m.bus, position) != Some(r[3])
            || ew_word(&m.bus, position.wrapping_add(4)) != Some(r[4])
        {
            return;
        }
        self.bindings.insert(
            owner,
            RootBinding {
                parent,
                position,
                pose_position: r[1],
                frame: m.bus.frames,
                vehicle: false,
                pose_world: Some([r[3] as i32, r[4] as i32]),
            },
        );
    }
    pub fn observe_vehicle(&mut self, m: &Machine) {
        // 08016ef0 receives the vehicle animation controller. Its +30
        // points to the physics actor; +3c is its world OBJ descriptor.
        let controller = m.cpu.regs[0];
        let Some(parent) = ew_word(&m.bus, controller.wrapping_add(48)) else {
            return;
        };
        let Some(owner) = ew_word(&m.bus, controller.wrapping_add(60)) else {
            return;
        };
        self.bindings.remove(&owner);
        let Some(position) = ew_word(&m.bus, parent.wrapping_add(24)) else {
            return;
        };
        if ew_word(&m.bus, controller) != Some(position)
            || ew_word(&m.bus, parent.wrapping_add(28)) != Some(controller)
            || ew_word(&m.bus, controller.wrapping_add(52)) != Some(VEHICLE_UPDATE)
            || ew_word(&m.bus, owner) != Some(position)
        {
            return;
        }
        self.bindings.insert(
            owner,
            RootBinding {
                parent,
                position,
                pose_position: position,
                frame: m.bus.frames,
                vehicle: true,
                pose_world: None,
            },
        );
    }
    fn anchor(&self, source: &MemMap, owner: u32) -> Option<MotionAnchor> {
        let b = self.bindings.get(&owner)?;
        if source.frames.saturating_sub(b.frame) > 120
            || ew_word(source, owner)? != b.pose_position
            || ew_word(source, b.parent.checked_add(24)?)? != b.position
        {
            return None;
        }
        // Actor physics can advance again between pose construction and upload.
        // Use the root captured at the actual pose copy for all its parts.
        // Vehicles separately use their checked projection-time placement.
        let world = b.pose_world.unwrap_or([
            ew_word(source, b.position)? as i32,
            ew_word(source, b.position.checked_add(4)?)? as i32,
        ]);
        let iw = |offset| i32::from_le_bytes(source.iwram[offset..offset + 4].try_into().unwrap());
        Some(MotionAnchor {
            identity: [b.parent, b.position],
            vehicle: b.vehicle,
            screen: std::array::from_fn(|i| {
                axis(
                    world[i],
                    0,
                    iw(0x50 + i * 4),
                    iw(0x60 + i * 4),
                    iw(0x14 + i * 4),
                )
                .1
            }),
        })
    }
}
#[derive(Clone, Copy)]
struct MotionAnchor {
    identity: [u32; 2],
    screen: [f64; 2],
    vehicle: bool,
}
#[derive(Clone, Copy)]
struct Anchor {
    correction: [f64; 2],
    slope: [f64; 2],
    world_screen: [f64; 2],
}
#[derive(Clone, Copy)]
struct Placement {
    owner: u32,
    integer: [i32; 2],
    anchor: Anchor,
}
#[derive(Default)]
pub struct Observer {
    cached: HashMap<u32, (u32, [i32; 2])>,
    anchors: HashMap<u32, Anchor>,
    placements: HashMap<u32, Placement>,
    rows: Vec<String>,
    pub wide: bool,
    offscreen: Vec<Sprite>,
    offscreen_failures: Vec<String>,
}
impl Observer {
    #[inline]
    pub fn interested(key: u32) -> bool {
        matches!(
            key,
            0x08012771
                | 0x08012897
                | 0x080128a5
                | 0x080140ab
                | 0x080145c3
                | 0x08014a77
                | 0x08014649
                | 0x08014001
        )
    }
    pub fn live_snapshot(&self, source: &MemMap) -> Result<Snapshot, String> {
        let control = half(&source.io, 0);
        if control & 7 != 1 || control & 0x8000 != 0 {
            return Err(format!("Unsupported display mode/OBJ window {control:04x}"));
        }
        let mut nodes = Vec::new();
        let mut node = u32::from_le_bytes(source.ewram[0x1d6c4..0x1d6c8].try_into().unwrap());
        while node != 0 {
            if nodes.len() >= 128
                || nodes.contains(&node)
                || node & 3 != 0
                || !(0x02000000..=0x0203fff0).contains(&node)
            {
                return Err("Invalid object chain".into());
            }
            let p = (node - 0x02000000) as usize;
            if source.ewram[p..p + 6] != source.oam[nodes.len() * 8..nodes.len() * 8 + 6] {
                return Err("Object upload not finished".into());
            }
            nodes.push(node);
            node = u32::from_le_bytes(source.ewram[p + 8..p + 12].try_into().unwrap());
        }
        let mut sprites = Vec::new();
        for slot in 0..128 {
            let Some(mut sprite) = Sprite::read(source, slot)? else {
                continue;
            };
            sprite.node = nodes.get(slot).copied().unwrap_or(0);
            if sprite.node != 0 {
                sprite.depth = source.ewram[(sprite.node - 0x02000000) as usize + 6];
            }
            if let Some(p) = self.placements.get(&sprite.node) {
                if p.integer[0] & 511 != half(&source.oam, slot * 8 + 2) as i32 & 511
                    || p.integer[1] & 255 != half(&source.oam, slot * 8) as i32 & 255
                {
                    return Err("World placement mismatch".into());
                }
                sprite.x = p.integer[0];
                sprite.y = p.integer[1];
                sprite.owner = p.owner;
                sprite.correction = p.anchor.correction;
                sprite.slope = p.anchor.slope;
                sprite.world_screen = Some(p.anchor.world_screen);
            } else if let Some((owner, p)) = self.cached.get(&sprite.node) {
                if p[0] & 511 != half(&source.oam, slot * 8 + 2) as i32 & 511
                    || p[1] & 255 != half(&source.oam, slot * 8) as i32 & 255
                {
                    return Err("Screen placement mismatch".into());
                }
                sprite.owner = *owner;
                if sprite.priority != 0 {
                    sprite.x = p[0];
                    sprite.y = p[1];
                    sprite.slope = [0x60, 0x64].map(|o| {
                        i32::from_le_bytes(source.iwram[o..o + 4].try_into().unwrap()) as f64
                            / 65536.
                    });
                    sprite.world_screen = Some(p.map(|v| v as f64));
                }
            } else if sprite.priority != 0 {
                return Err("Unclassified world object".into());
            }
            sprite.hires = sprite.slope != [0.0; 2];
            sprites.push(sprite);
        }
        for (i, extra) in self.offscreen.iter().enumerate() {
            let mut extra = extra.clone();
            let before = sprites
                .iter()
                .filter(|s| !s.host_only && s.priority == extra.priority)
                .find(|s| s.depth > extra.depth)
                .map(|s| s.slot)
                .unwrap_or(nodes.len());
            extra.order = before * 256 + i + 1;
            sprites.push(extra);
        }
        Ok(Snapshot::new(sprites, source))
    }
    pub fn observe(&mut self, m: &Machine, key: u32) -> Result<(), String> {
        if key == 0x08012771 {
            self.offscreen.clear();
            self.anchors.clear();
            self.placements.clear();
            self.cached.clear();
        }
        if key == 0x08012897 || key == 0x080128a5 {
            let node = m.cpu.regs[0];
            self.placements.remove(&node);
            self.cached.remove(&node);
        }
        if key == 0x080140ab {
            let owner = m.cpu.regs[5];
            let node = m.cpu.regs[2];
            let data = bytes(m, owner + 8, 4)?;
            let position = [
                i16::from_le_bytes(data[..2].try_into().unwrap()) as i32,
                i16::from_le_bytes(data[2..].try_into().unwrap()) as i32,
            ];
            self.cached.insert(node, (owner, position));
        }
        if key == 0x080145c3 {
            let owner = m.cpu.regs[6];
            let ptr = word(m, owner)? as u32;
            let world = [word(m, ptr)?, word(m, ptr + 4)?];
            let offset = bytes(m, owner + 0x22, 2)?;
            let bounds = [word(m, 0x03000050)?, word(m, 0x03000054)?];
            let scale = [word(m, 0x03000060)?, word(m, 0x03000064)?];
            let camera = [word(m, 0x03000014)?, word(m, 0x03000018)?];
            let projected: [(i32, f64); 2] = std::array::from_fn(|i| {
                axis(world[i], offset[i] as i8, bounds[i], scale[i], camera[i])
            });
            if projected[0].0 != m.cpu.regs[9] as i32 || projected[1].0 != m.cpu.regs[8] as i32 {
                return Err(format!("World OBJ projection mismatch for {owner:08x}"));
            }
            let anchor = Anchor {
                correction: projected.map(|(integer, precise)| precise - integer as f64),
                slope: scale.map(|s| s as f64 / 65536.0),
                world_screen: std::array::from_fn(|i| {
                    axis(world[i], 0, bounds[i], scale[i], camera[i]).1
                }),
            };
            self.anchors.insert(owner, anchor);
            self.rows.push(format!(
                "{},{owner:08x},{ptr:08x},{},{},{},{},{},{},{},{},{},{}",
                m.bus.frames,
                world[0],
                world[1],
                camera[0],
                camera[1],
                scale[0],
                scale[1],
                projected[0].0,
                projected[1].0,
                projected[0].1,
                projected[1].1
            ));
        }
        if key == 0x08014649 && self.wide {
            let x = m.cpu.regs[9] as i32;
            let y = m.cpu.regs[8] as i32;
            let half_width = m.cpu.regs[1] as i32;
            let half_height = m.cpu.regs[4] as i32 / 2;
            // Only add objects rejected horizontally by GBA, inside the new sides.
            if (x + half_width < 0 || x - half_width > 240)
                && x + half_width >= -23
                && x - half_width <= 263
                && y + half_height >= 0
                && y - half_height <= 160
            {
                match self.offscreen_sprite(m) {
                    Ok(Some(sprite)) => self.offscreen.push(sprite),
                    Ok(None) => {}
                    Err(reason) => self
                        .offscreen_failures
                        .push(format!("owner={:08x}: {reason}", m.cpu.regs[6])),
                }
            }
        }
        if key == 0x08014001 && self.wide {
            let x = m.cpu.regs[1] as i32;
            let y = m.cpu.regs[2] as i32;
            let w = m.cpu.regs[4] as i32;
            let h = m.cpu.regs[6] as i32;
            if (x + w < 0 || x - w > 240)
                && x + w >= -23
                && x - w <= 263
                && y + h >= 0
                && y - h <= 160
            {
                match self.side_sprite(m, true) {
                    Ok(Some(sprite)) if sprite.priority != 0 => self.offscreen.push(sprite),
                    Ok(_) => {}
                    Err(reason) => self
                        .offscreen_failures
                        .push(format!("screen owner={:08x}: {reason}", m.cpu.regs[5])),
                }
            }
        }
        // attr1 has been written; attr0 write at this PC follows immediately.
        if key == 0x08014a77 {
            let owner = m.cpu.regs[6];
            let anchor = *self
                .anchors
                .get(&owner)
                .ok_or("OBJ placement without checked anchor")?;
            self.placements.insert(
                m.cpu.regs[7],
                Placement {
                    owner,
                    integer: [m.cpu.regs[9] as i32, m.cpu.regs[8] as i32],
                    anchor,
                },
            );
        }
        Ok(())
    }
    pub fn side_failures(&self) -> &[String] {
        &self.offscreen_failures
    }
    pub fn record_side_failure(&mut self, reason: String) {
        self.offscreen_failures.push(reason);
    }
    fn offscreen_sprite(&self, m: &Machine) -> Result<Option<Sprite>, String> {
        self.side_sprite(m, false)
    }
    fn side_sprite(&self, m: &Machine, screen: bool) -> Result<Option<Sprite>, String> {
        let owner = m.cpu.regs[if screen { 5 } else { 6 }];
        let anchor = if screen {
            Anchor {
                correction: [0.; 2],
                slope: [0x60, 0x64].map(|o| {
                    i32::from_le_bytes(m.bus.iwram[o..o + 4].try_into().unwrap()) as f64 / 65536.
                }),
                world_screen: [m.cpu.regs[1] as i32 as f64, m.cpu.regs[2] as i32 as f64],
            }
        } else {
            let Some(anchor) = self.anchors.get(&owner) else {
                return Ok(None);
            };
            *anchor
        };
        let mut fork = m.try_fork_for_research().map_err(str::to_string)?;
        // Skip only the original rectangle rejection; preserve visibility flags,
        // animation, allocation and all subsequent sprite placement operations.
        let xy_screen = [m.cpu.regs[1] as i32, m.cpu.regs[2] as i32];
        fork.cpu.regs[15] = if screen { 0x0801401a } else { 0x08014684 };
        if screen {
            fork.cpu.regs[3] = 0;
        }
        crate::scene_replay::presentation_span(
            &mut fork,
            if screen { 0x08014105 } else { 0x08014a79 },
        )?;
        let node = word(&fork, owner + 24)? as u32;
        if node == 0 {
            return Ok(None);
        }
        let attrs = bytes(&fork, node, 6)?.to_vec();
        let depth = bytes(&fork, node + 6, 1)?[0];
        let xy = if screen {
            xy_screen
        } else {
            [fork.cpu.regs[9] as i32, fork.cpu.regs[8] as i32]
        };
        // Finish pending texture uploads only in this isolated presentation copy.
        fork.cpu.regs[15] = 0x08012d44;
        fork.cpu.regs[14] = 0x08000001;
        crate::scene_replay::presentation_span(&mut fork, 0x08000001)?;
        // Commit the OBJ palette queue; 08013354 uploads background palettes.
        fork.cpu.regs[15] = 0x08013550;
        fork.cpu.regs[14] = 0x08000001;
        crate::scene_replay::presentation_span(&mut fork, 0x08000001)?;
        fork.bus.oam[127 * 8..127 * 8 + 6].copy_from_slice(&attrs);
        let a1 = u16::from_le_bytes(attrs[2..4].try_into().unwrap());
        if attrs[1] & 1 != 0 {
            let matrix = word(&fork, owner + 52)? as u32;
            let data = bytes(&fork, matrix, 8)?.to_vec();
            let group = ((a1 >> 9) & 31) as usize * 32;
            for i in 0..4 {
                fork.bus.oam[group + 6 + i * 8..group + 8 + i * 8]
                    .copy_from_slice(&data[i * 2..i * 2 + 2]);
            }
        }
        let Some(mut sprite) = Sprite::read(&fork.bus, 127)? else {
            return Ok(None);
        };
        sprite.host_only = true;
        sprite.depth = depth;
        sprite.generation = 1;
        sprite.node = owner;
        sprite.owner = owner;
        sprite.x = xy[0];
        sprite.y = xy[1];
        sprite.correction = anchor.correction;
        sprite.slope = anchor.slope;
        sprite.world_screen = Some(anchor.world_screen);
        sprite.hires = true;
        Ok(Some(sprite))
    }
    pub fn capture(
        &self,
        source: &MemMap,
        dir: &Path,
    ) -> Result<Snapshot, Box<dyn std::error::Error>> {
        std::fs::create_dir(dir)?;
        std::fs::write(dir.join("source.ewram"), &source.ewram)?;
        std::fs::write(dir.join("source.oam"), &source.oam)?;
        std::fs::write(dir.join("source.io"), &source.io)?;
        std::fs::write(dir.join("source.vram"), &source.vram)?;
        std::fs::write(dir.join("source.palette"), &source.palette)?;
        let pool = POOL_WRITES.with(|p| {
            p.borrow()
                .as_ref()
                .map(|r| r.join("\n"))
                .unwrap_or_default()
        });
        std::fs::write(dir.join("pool-writes.csv"),format!("frame,pc,lr,address,width,value,r0,r1,r2,r3,r4,r5,r6,r7,r8,r9,r10,r11,r12\n{pool}\n"))?;
        let mut file = std::fs::File::create_new(dir.join("anchors.csv"))?;
        writeln!(file,"frame,owner,position,world_x_raw,world_y_raw,camera_x_raw,camera_y_raw,scale_x_raw,scale_y_raw,integer_x,integer_y,continuous_x,continuous_y")?;
        for row in &self.rows {
            writeln!(file, "{row}")?;
        }
        let mut nodes = Vec::new();
        let mut node = u32::from_le_bytes(source.ewram[0x1d6c4..0x1d6c8].try_into().unwrap());
        while node != 0 {
            if nodes.len() >= 128
                || nodes.contains(&node)
                || node & 3 != 0
                || !(0x02000000..=0x0203fff0).contains(&node)
            {
                return Err("Invalid scene OBJ descriptor chain".into());
            }
            let p = (node - 0x02000000) as usize;
            let slot = nodes.len();
            if source.ewram[p..p + 6] != source.oam[slot * 8..slot * 8 + 6] {
                return Err(format!("OBJ descriptor/OAM mismatch slot {slot}").into());
            }
            nodes.push(node);
            node = u32::from_le_bytes(source.ewram[p + 8..p + 12].try_into().unwrap());
        }
        let mut sprites = Vec::new();
        let mut report = std::fs::File::create_new(dir.join("slots.csv"))?;
        writeln!(report,"slot,descriptor,owner,priority,classification,x,y,continuous_x,continuous_y,slope_x,slope_y")?;
        for slot in 0..128 {
            let Some(mut sprite) = Sprite::read(source, slot)? else {
                continue;
            };
            let node = nodes.get(slot).copied().unwrap_or(0);
            let placement = self.placements.get(&node);
            if let Some(p) = placement {
                if p.integer[0] & 511 != half(&source.oam, slot * 8 + 2) as i32 & 511
                    || p.integer[1] & 255 != half(&source.oam, slot * 8) as i32 & 255
                {
                    return Err("Checked OBJ placement differs from OAM".into());
                }
                sprite.correction = p.anchor.correction;
                sprite.slope = p.anchor.slope;
            } else if let Some((_, position)) = self.cached.get(&node) {
                if position[0] & 511 != half(&source.oam, slot * 8 + 2) as i32 & 511
                    || position[1] & 255 != half(&source.oam, slot * 8) as i32 & 255
                {
                    return Err("Cached screen position differs from OAM".into());
                }
            } else if sprite.priority != 0 {
                return Err(format!(
                    "Unclassified non-interface OBJ slot {slot}, priority {}",
                    sprite.priority
                )
                .into());
            }
            writeln!(
                report,
                "{slot},{node:08x},{:08x},{},{},{},{},{},{},{},{}",
                placement.map_or_else(|| self.cached.get(&node).map_or(0, |c| c.0), |p| p.owner),
                sprite.priority,
                if placement.is_some() {
                    "world_checked"
                } else {
                    if self.cached.contains_key(&node) {
                        "cached_screen_checked"
                    } else {
                        "screen_candidate"
                    }
                },
                sprite.x,
                sprite.y,
                sprite.x as f64 + sprite.correction[0],
                sprite.y as f64 + sprite.correction[1],
                sprite.slope[0],
                sprite.slope[1]
            )?;
            sprites.push(sprite);
        }
        let snapshot = Snapshot::new(sprites, source);
        let control = half(&source.io, 0);
        let blend = half(&source.io, 0x50);
        if control & 7 != 1 || control & 0xe000 != 0 || (blend & 0xc0 != 0 && blend & 0x3f != 0) {
            return Err(
                "OBJ scene composition requires mode 1 without windows or active BG blending"
                    .into(),
            );
        }
        let black = raw_layers(source, 0x1000, 0);
        let white = raw_layers(source, 0x1000, 0x7fff);
        for y in 0..160 {
            for x in 0..240 {
                let expected =
                    (black[y * 240 + x] == white[y * 240 + x]).then_some(black[y * 240 + x]);
                if snapshot.pixel(x as f64, y as f64, 0, false).map(|p| p.0) != expected {
                    return Err(format!("OBJ sampler/PPU mismatch at {x},{y}").into());
                }
            }
        }
        let backgrounds: Vec<_> = (0..3)
            .map(|bg| {
                let black = raw_layers(source, 0x100 << bg, 0);
                let white = raw_layers(source, 0x100 << bg, 0x7fff);
                black
                    .into_iter()
                    .zip(white)
                    .map(|(a, b)| (a == b).then_some(a))
                    .collect::<Vec<_>>()
            })
            .collect();
        let full = raw_layers(source, control & 0x1700, half(&source.palette, 0));
        for y in 0..160 {
            for x in 0..240 {
                let i = y * 240 + x;
                let mut color = half(&source.palette, 0) & 0x7fff;
                let mut rank = (4u16, 4u16);
                for bg in 0..3 {
                    let next = (half(&source.io, 8 + bg * 2) & 3, bg as u16);
                    if control & (0x100 << bg) != 0 && next < rank {
                        if let Some(p) = backgrounds[bg][i] {
                            color = p;
                            rank = next;
                        }
                    }
                }
                if control & 0x1000 != 0 {
                    if let Some((p, priority, _)) = snapshot.pixel(x as f64, y as f64, 0, false) {
                        if priority <= rank.0 {
                            color = p;
                        }
                    }
                }
                if color != full[i] {
                    return Err(format!("Original scene priority mismatch at {x},{y}").into());
                }
            }
        }
        crate::scene_replay::ppm(&dir.join("reference-full.ppm"), &full, 240, 160)?;
        std::fs::write(dir.join("checks.txt"),format!("anchor_checks={}\nactive_objects={}\nworld_objects={}\nscreen_candidates={}\noriginal_obj_pixels_checked=38400\noriginal_scene_priority_pixels_checked=38400\n",self.rows.len(),snapshot.sprites.len(),snapshot.sprites.iter().filter(|s|s.slope!=[0.0;2]).count(),snapshot.sprites.iter().filter(|s|s.slope==[0.0;2]).count()))?;
        std::fs::write(dir.join("source.oam"), &source.oam)?;
        Ok(snapshot)
    }
}
#[derive(PartialEq)]
struct Texture {
    width: usize,
    height: usize,
    pixels: Vec<Option<u16>>,
}
impl Texture {
    fn sample(
        &self,
        x: f64,
        y: f64,
        bounds: [usize; 2],
        affine: Option<[f64; 4]>,
        flips: [bool; 2],
    ) -> Option<u16> {
        let (u, v) = if let Some([pa, pb, pc, pd]) = affine {
            let dx = x - bounds[0] as f64 / 2.0;
            let dy = y - bounds[1] as f64 / 2.0;
            (
                (pa * dx + pb * dy + self.width as f64 / 2.0).floor() as i32,
                (pc * dx + pd * dy + self.height as f64 / 2.0).floor() as i32,
            )
        } else {
            let u = x.floor() as i32;
            let v = y.floor() as i32;
            (
                if flips[0] {
                    self.width as i32 - 1 - u
                } else {
                    u
                },
                if flips[1] {
                    self.height as i32 - 1 - v
                } else {
                    v
                },
            )
        };
        if u < 0 || v < 0 || u >= self.width as i32 || v >= self.height as i32 {
            None
        } else {
            self.pixels[v as usize * self.width + u as usize]
        }
    }
}
#[derive(Clone)]
struct Sprite {
    pixels: Arc<Vec<Option<u16>>>,
    host_only: bool,
    node: u32,
    owner: u32,
    generation: u64,
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    priority: u16,
    semi: bool,
    slot: usize,
    order: usize,
    depth: u8,
    correction: [f64; 2],
    slope: [f64; 2],
    world_screen: Option<[f64; 2]>,
    motion: Option<MotionAnchor>,
    zoom: f64,
    texture: Option<Arc<Texture>>,
    affine: Option<[f64; 4]>,
    flips: [bool; 2],
    hires: bool,
}
impl Sprite {
    fn read(source: &MemMap, slot: usize) -> Result<Option<Self>, String> {
        let a0 = half(&source.oam, slot * 8);
        let a1 = half(&source.oam, slot * 8 + 2);
        let a2 = half(&source.oam, slot * 8 + 4);
        let affine = a0 & 0x100 != 0;
        let shape = (a0 >> 14) as usize;
        if (!affine && a0 & 0x200 != 0) || shape == 3 {
            return Ok(None);
        }
        if a0 & 0x1800 != 0 {
            return Err(format!("Unsupported OBJ effect/mosaic slot {slot}"));
        }
        let sizes = [
            [(8, 8), (16, 16), (32, 32), (64, 64)],
            [(16, 8), (32, 8), (32, 16), (64, 32)],
            [(8, 16), (8, 32), (16, 32), (32, 64)],
        ];
        let (w, h) = sizes[shape][(a1 >> 14) as usize];
        let (bw, bh) = if affine && a0 & 0x200 != 0 {
            (w * 2, h * 2)
        } else {
            (w, h)
        };
        let group = ((a1 >> 9) & 31) as usize * 32;
        let matrix = |o| half(&source.oam, group + o) as i16 as i32;
        let eight = a0 & 0x2000 != 0;
        let step = if eight { 2 } else { 1 };
        let stride = if half(&source.io, 0) & 0x40 != 0 {
            w / 8 * step
        } else {
            32
        };
        let mut texels = vec![None; (w * h) as usize];
        for v in 0..h {
            for u in 0..w {
                let tile = (a2 & 1023) as usize + (v / 8 * stride + u / 8 * step) as usize;
                if half(&source.io, 0) & 7 >= 3 && tile < 512 {
                    continue;
                }
                let address = 0x10000
                    + if eight {
                        tile / 2 * 64 + (v % 8 * 8 + u % 8) as usize
                    } else {
                        tile * 32 + (v % 8 * 4 + u % 8 / 2) as usize
                    };
                if address >= 0x18000 {
                    continue;
                }
                let b = source.vram[address];
                let index = if eight {
                    b as usize
                } else {
                    let n = if u & 1 == 0 { b & 15 } else { b >> 4 };
                    if n == 0 {
                        0
                    } else {
                        (a2 >> 12) as usize * 16 + n as usize
                    }
                };
                if index != 0 {
                    texels[(v * w + u) as usize] =
                        Some(half(&source.palette, (256 + index) * 2) & 0x7fff);
                }
            }
        }
        let texture = Arc::new(Texture {
            width: w as usize,
            height: h as usize,
            pixels: texels,
        });
        let affine_matrix = affine
            .then(|| [matrix(6), matrix(14), matrix(22), matrix(30)].map(|v| v as f64 / 256.0));
        let flips = [!affine && a1 & 0x1000 != 0, !affine && a1 & 0x2000 != 0];
        let mut pixels = vec![None; (bw * bh) as usize];
        for y in 0..bh {
            for x in 0..bw {
                pixels[(y * bw + x) as usize] = texture.sample(
                    x as f64,
                    y as f64,
                    [bw as usize, bh as usize],
                    affine_matrix,
                    flips,
                );
            }
        }
        let mut x = (a1 & 511) as i32;
        let mut y = (a0 & 255) as i32;
        if x >= 240 {
            x -= 512;
        }
        if y >= 160 {
            y -= 256;
        }
        Ok(Some(Self {
            pixels: Arc::new(pixels),
            host_only: false,
            node: 0,
            owner: 0,
            generation: 0,
            width: bw as usize,
            height: bh as usize,
            x,
            y,
            priority: (a2 >> 10) & 3,
            semi: a0 & 0x400 != 0,
            slot,
            order: (slot + 1) * 256,
            depth: 0,
            correction: [0.0; 2],
            slope: [0.0; 2],
            world_screen: None,
            motion: None,
            zoom: 1.0,
            texture: Some(texture),
            affine: affine_matrix,
            flips,
            hires: false,
        }))
    }
    fn sample(&self, x: f64, y: f64, delta: i32, continuous: bool) -> Option<u16> {
        self.sample_shift(x, y, [delta, 0], continuous)
    }
    fn sample_shift(&self, x: f64, y: f64, delta: [i32; 2], continuous: bool) -> Option<u16> {
        let correction = if continuous {
            self.correction
        } else {
            [0.0; 2]
        };
        let left = self.x as f64 + correction[0]
            - if continuous {
                self.slope[0] * delta[0] as f64 / 65536.0
            } else {
                0.0
            };
        let top = self.y as f64 + correction[1]
            - if continuous {
                self.slope[1] * delta[1] as f64 / 65536.0
            } else {
                0.0
            };
        let local = [(x - left) / self.zoom, (y - top) / self.zoom];
        self.sample_local(local, continuous)
    }
    #[inline]
    fn sample_local(&self, local: [f64; 2], continuous: bool) -> Option<u16> {
        if continuous && self.hires {
            if local[0] < 0.0
                || local[1] < 0.0
                || local[0] >= self.width as f64
                || local[1] >= self.height as f64
            {
                return None;
            }
            if let Some(texture) = &self.texture {
                return texture.sample(
                    local[0],
                    local[1],
                    [self.width, self.height],
                    self.affine,
                    self.flips,
                );
            }
        }
        let (u, v) = (local[0].floor() as i32, local[1].floor() as i32);
        if u < 0 || v < 0 || u >= self.width as i32 || v >= self.height as i32 {
            None
        } else {
            self.pixels[v as usize * self.width + u as usize]
        }
    }
}
#[derive(Clone)]
pub struct Snapshot {
    sprites: Vec<Sprite>,
    hud: std::sync::Arc<Vec<Option<u16>>>,
}
#[cfg(test)]
#[test]
#[ignore = "manual renderer benchmark using a captured scene; no CPU emulation"]
fn benchmark_captured_composition() {
    let stem = std::env::var("ADVANCE_BENCH_SCENE").expect("ADVANCE_BENCH_SCENE");
    let mut bus = MemMap::new(Vec::new());
    for (extension, target) in [
        ("io", &mut bus.io),
        ("iwram", &mut bus.iwram),
        ("vram", &mut bus.vram),
        ("oam", &mut bus.oam),
        ("palette", &mut bus.palette),
    ] {
        target.copy_from_slice(&std::fs::read(format!("{stem}.{extension}")).unwrap());
    }
    let sprites = (0..128)
        .filter_map(|slot| Sprite::read(&bus, slot).unwrap())
        .map(|mut s| {
            if s.priority != 0 {
                s.hires = true;
                s.slope = [1.0; 2];
            }
            s
        })
        .collect();
    let snapshot = Snapshot::new(sprites, &bus);
    let building: Vec<_> = (0..720 * 480)
        .map(|i| if i % 17 == 0 { 1 } else { 0 })
        .collect();
    let start = std::time::Instant::now();
    let mut hash = 0;
    for _ in 0..300 {
        let frame = snapshot.compose_camera(
            std::hint::black_box(&bus),
            &building,
            &bus.palette,
            [12000, -8000],
            3,
            1.03,
        );
        hash = crate::hash(std::hint::black_box(&frame));
    }
    println!(
        "compose_bench scene={stem} ms={:.3} hash={hash:016x}",
        start.elapsed().as_secs_f64() * 1000.0
    );
}
#[cfg(test)]
#[test]
#[ignore = "manual visual check of captured gameplay HUD; ADVANCE_HUD_SCENES and ADVANCE_HUD_OUTPUT"]
fn hud_captured_composition() {
    let directory = std::path::PathBuf::from(std::env::var("ADVANCE_HUD_SCENES").unwrap());
    let output = std::path::PathBuf::from(std::env::var("ADVANCE_HUD_OUTPUT").unwrap());
    std::fs::create_dir_all(&output).unwrap();
    for stem in [
        "001675", "001850", "002100", "002600", "002910", "002950", "003450",
    ] {
        let mut source = MemMap::new(Vec::new());
        for (ext, target) in [
            ("io", &mut source.io),
            ("iwram", &mut source.iwram),
            ("ewram", &mut source.ewram),
            ("vram", &mut source.vram),
            ("palette", &mut source.palette),
            ("oam", &mut source.oam),
        ] {
            target
                .copy_from_slice(&std::fs::read(directory.join(format!("{stem}.{ext}"))).unwrap());
        }
        let sprites = (0..128)
            .filter_map(|slot| Sprite::read(&source, slot).unwrap())
            .map(|mut s| {
                if s.priority != 0 {
                    s.slope = [1.0; 2];
                }
                s
            })
            .collect();
        let snapshot = Snapshot::new(sprites, &source);
        let a = raw_layers(&source, 0x100, 0);
        let b = raw_layers(&source, 0x100, 0x7fff);
        let base: Vec<u8> = a
            .iter()
            .zip(&b)
            .map(|(&a, &b)| {
                if a != b {
                    0
                } else {
                    (1..256)
                        .find(|i| half(&source.palette, i * 2) & 0x7fff == a)
                        .expect("Opaque BG0 palette index") as u8
                }
            })
            .collect();
        let building: Vec<_> = (0..720 * 480)
            .map(|i| base[(i / 720 / 3) * 240 + (i % 720 / 3)])
            .collect();
        let unmodified = (source.io.clone(), source.vram.clone(), source.oam.clone());
        for (name, hud) in [
            ("100", crate::hud_layout::Settings::default()),
            (
                "75",
                crate::hud_layout::Settings {
                    radar: crate::hud_layout::Size::Medium,
                    status: crate::hud_layout::Size::Medium,
                    messages: crate::hud_layout::Size::Medium,
                },
            ),
            (
                "50",
                crate::hud_layout::Settings {
                    radar: crate::hud_layout::Size::Half,
                    status: crate::hud_layout::Size::Half,
                    messages: crate::hud_layout::Size::Half,
                },
            ),
        ] {
            let start = std::time::Instant::now();
            let image = snapshot.compose_camera_hud(
                &source,
                &building,
                &source.palette,
                [0, 0],
                3,
                1.0,
                hud,
            );
            println!(
                "hud_compose {stem} {name}% ms={:.3}",
                start.elapsed().as_secs_f64() * 1000.0
            );
            crate::scene_replay::ppm(&output.join(format!("{stem}-{name}.ppm")), &image, 720, 480)
                .unwrap();
            // HUD changes must leave the center of the world bit-identical.
            if name != "100" {
                let original =
                    snapshot.compose_camera(&source, &building, &source.palette, [0, 0], 3, 1.0);
                for y in 192..360 {
                    for x in 192..528 {
                        assert_eq!(
                            image[y * 720 + x],
                            original[y * 720 + x],
                            "world changed {stem} {name} at {x},{y}"
                        );
                    }
                }
            }
        }
        assert_eq!((source.io, source.vram, source.oam), unmodified);
    }
}
impl Snapshot {
    pub fn use_hud_from(&mut self, latest: &Self) {
        self.hud = latest.hud.clone();
    }
    pub fn refresh_palette(&mut self, source: &MemMap) -> Result<(), String> {
        let decoded: Vec<_> = self
            .sprites
            .iter()
            .map(|sprite| {
                if sprite.host_only {
                    return Ok(None);
                }
                Sprite::read(source, sprite.slot)?
                    .map(Some)
                    .ok_or_else(|| "Palette refresh lost an existing OBJ".to_owned())
            })
            .collect::<Result<_, _>>()?;
        for (sprite, colors) in self.sprites.iter_mut().zip(decoded) {
            if let Some(colors) = colors {
                sprite.pixels = colors.pixels;
                sprite.texture = colors.texture;
            }
        }
        self.hud = Arc::new(hud_layer(source));
        Ok(())
    }
    pub fn bind_motion_roots(&mut self, roots: &MotionRoots, source: &MemMap) {
        let anchors: HashMap<_, _> = roots
            .bindings
            .keys()
            .filter_map(|&owner| roots.anchor(source, owner).map(|a| (owner, a)))
            .collect();
        // Feet and other parts can refer directly to the same checked actor
        // position. They must receive exactly the body's translation.
        let positions: HashMap<_, _> = anchors.values().map(|a| (a.identity[1], *a)).collect();
        for sprite in &mut self.sprites {
            sprite.motion = anchors
                .get(&sprite.owner)
                .copied()
                .or_else(|| positions.get(&ew_word(source, sprite.owner)?).copied());
            if let Some(motion) = sprite.motion.as_mut().filter(|m| m.vehicle) {
                // Vehicle descriptors point directly at the physics position.
                // Use the checked projection-time value: physics can already
                // have advanced again by the end of the graphics upload.
                if let Some(screen) = sprite.world_screen {
                    motion.screen = screen;
                }
            }
        }
    }
    pub fn trace_anchors(&self, frame: u64, delta: [i32; 2], output: &mut std::fs::File) {
        for s in &self.sprites {
            if let Some(anchor) = s.world_screen {
                let correction: [f64; 2] = std::array::from_fn(|i| {
                    s.correction[i] - s.slope[i] * delta[i] as f64 / 65536.0
                });
                let hash = s.pixels.iter().fold(0xcbf29ce484222325u64, |h, p| {
                    (h ^ p.map_or(0xffff, |p| p) as u64).wrapping_mul(0x1_0000_01b3)
                });
                let (root, motion) = s
                    .motion
                    .map_or(([0; 2], [f64::NAN; 2]), |m| (m.identity, m.screen));
                let matrix = s.affine.unwrap_or([f64::NAN; 4]);
                let _ =
                    writeln!(
                    output,
                    "{frame},{:08x},{:08x},{},{},{},{},{},{},{},{},{hash:016x},{:08x},{:08x},{},{},{},{},{},{},{},{},{}",
                    s.node,
                    s.owner,
                    s.x,
                    s.y,
                    correction[0],
                    correction[1],
                    anchor[0],
                    anchor[1],
                    s.x as f64 + correction[0] - anchor[0],
                    s.y as f64 + correction[1] - anchor[1],
                    root[0], root[1], motion[0], motion[1], matrix[0],matrix[1],matrix[2],matrix[3],s.hires,s.motion.is_some_and(|m|m.vehicle),s.zoom
                );
            }
        }
    }
    pub fn nodes(&self) -> Vec<u32> {
        self.sprites
            .iter()
            .map(|s| s.node)
            .filter(|n| *n != 0)
            .collect()
    }
    fn new(sprites: Vec<Sprite>, source: &MemMap) -> Self {
        Self {
            sprites,
            hud: Arc::new(hud_layer(source)),
        }
    }
    pub fn stamp(&mut self, generations: &HashMap<u32, u64>) {
        for s in &mut self.sprites {
            if !s.host_only {
                s.generation = generations.get(&s.node).copied().unwrap_or(0);
            }
        }
    }
    pub fn interpolate(&self, previous: &Self, alpha: f64, delta: [i32; 2]) -> Self {
        let mut result = self.clone();
        for s in &mut result.sprites {
            if s.slope == [0.0; 2] || s.node == 0 || s.generation == 0 {
                continue;
            }
            let Some(old) = previous.sprites.iter().find(|p| {
                p.owner == s.owner
                    && p.slope != [0.0; 2]
                    && ((p.node == s.node && p.generation == s.generation)
                        || ((p.host_only || s.host_only)
                            && p.motion.is_some()
                            && p.motion.map(|m| m.identity) == s.motion.map(|m| m.identity)))
            }) else {
                continue;
            };
            let (a, b) = match (old.motion, s.motion) {
                (Some(a), Some(b)) if a.identity == b.identity => (a.screen, b.screen),
                (None, None) => {
                    let (Some(a), Some(b)) = (old.world_screen, s.world_screen) else {
                        continue;
                    };
                    // A projected world point is not necessarily an actor root:
                    // bat animation owner 02025284 is a temporary pose object whose
                    // world point itself rotates around the actor. Blend translation
                    // only within the same rasterized pose and local placement.
                    // A new pose and its position must take effect together.
                    if old.width != s.width
                        || old.height != s.height
                        || old.pixels != s.pixels
                        || (0..2).any(|i| {
                            let old_xy = [old.x, old.y][i] as f64 + old.correction[i];
                            let new_xy = [s.x, s.y][i] as f64 + s.correction[i];
                            ((old_xy - a[i]) - (new_xy - b[i])).abs() > 1e-6
                        })
                    {
                        continue;
                    }
                    (a, b)
                }
                _ => continue,
            };
            if (a[0] - b[0]).abs() > 32.0 || (a[1] - b[1]).abs() > 32.0 {
                continue;
            }
            // Only vehicles have a checked rigid body here. Their source
            // texture stays unchanged while the OAM matrix rotates/scales it.
            // Never blend matrices across animation cells or reused actors.
            if s.motion.is_some_and(|m| m.vehicle)
                && old
                    .motion
                    .is_some_and(|m| Some(m.identity) == s.motion.map(|n| n.identity) && m.vehicle)
                && s.width == old.width
                && s.height == old.height
                && s.texture.is_some()
                && s.texture == old.texture
            {
                if let (Some(a), Some(b)) = (old.affine, s.affine) {
                    let cosine =
                        (a[0] * b[0] + a[1] * b[1]) / (a[0].hypot(a[1]) * b[0].hypot(b[1]));
                    // Reject discontinuous turns/degenerate matrices.
                    if cosine > 0.5
                        && [a, b]
                            .iter()
                            .all(|m| (m[0] - m[3]).abs() < 1e-9 && (m[1] + m[2]).abs() < 1e-9)
                    {
                        if alpha <= 0.0 {
                            s.affine = Some(a);
                        } else if alpha < 1.0 {
                            // Interpolate the shortest rotation arc and scale
                            // separately; coefficient lerp shrinks a turning car.
                            let angle = a[1].atan2(a[0]);
                            let turn = (b[1].atan2(b[0]) - angle + std::f64::consts::PI)
                                .rem_euclid(std::f64::consts::TAU)
                                - std::f64::consts::PI;
                            let size =
                                a[0].hypot(a[1]) + (b[0].hypot(b[1]) - a[0].hypot(a[1])) * alpha;
                            let (sin, cos) = (angle + turn * alpha).sin_cos();
                            s.affine = Some([size * cos, size * sin, -size * sin, size * cos]);
                        }
                    }
                }
            }
            s.correction = std::array::from_fn(|i| {
                // A checked actor root may translate across pose changes.
                // The latest bitmap and all its local offsets stay together.
                s.correction[i]
                    + (a[i] - b[i]) * (1.0 - alpha)
                    + s.slope[i] * delta[i] as f64 / 65536.0
            });
        }
        result
    }
    pub fn interpolate_projected(
        &self,
        previous: &Self,
        old_graphics: &MemMap,
        graphics: &MemMap,
        alpha: f64,
        delta: [i32; 2],
    ) -> Self {
        let iw = |b: &MemMap, o| i32::from_le_bytes(b.iwram[o..o + 4].try_into().unwrap());
        let old_scale = [iw(old_graphics, 0x60), iw(old_graphics, 0x64)];
        let new_scale = [iw(graphics, 0x60), iw(graphics, 0x64)];
        if old_scale == new_scale {
            return self.interpolate(previous, alpha, delta);
        }
        let origin = |b: &MemMap, i: usize| {
            let c = iw(b, 0x14 + i * 4);
            axis(c, 0, iw(b, 0x50 + i * 4), iw(b, 0x60 + i * 4), c).1
        };
        let map = |p: [f64; 2]| {
            std::array::from_fn(|i| {
                origin(graphics, i)
                    + (p[i] - origin(old_graphics, i)) * new_scale[i] as f64 / old_scale[i] as f64
            })
        };
        let mut normalized = previous.clone();
        for sprite in &mut normalized.sprites {
            sprite.world_screen = sprite.world_screen.map(map);
            if let Some(m) = &mut sprite.motion {
                m.screen = map(m.screen);
            }
            if let Some(matrix) = &mut sprite.affine {
                for i in 0..4 {
                    matrix[i] *= old_scale[i % 2] as f64 / new_scale[i % 2] as f64;
                }
            }
        }
        self.interpolate(&normalized, alpha, delta)
    }
    pub fn zoomed(&self, zoom: f64) -> Self {
        let mut result = self.clone();
        if zoom == 1.0 {
            return result;
        }
        for sprite in &mut result.sprites {
            if sprite.slope == [0.0; 2] {
                continue;
            }
            sprite.correction = std::array::from_fn(|i| {
                let xy = [sprite.x, sprite.y][i] as f64;
                120.0 + (xy + sprite.correction[i] - 120.0) * zoom - xy
            });
            sprite.slope = sprite.slope.map(|v| v * zoom);
            sprite.zoom *= zoom;
        }
        result
    }
    fn pixel(&self, x: f64, y: f64, delta: i32, continuous: bool) -> Option<(u16, u16, usize)> {
        self.sprites
            .iter()
            .filter_map(|s| {
                s.sample(x, y, delta, continuous)
                    .map(|p| (p, s.priority, s.order))
            })
            .min_by_key(|p| (p.1, p.2))
    }
    pub fn compose(
        &self,
        source: &MemMap,
        building: &[u8],
        palette: &[u8],
        delta: i32,
        scale: usize,
    ) -> Vec<u16> {
        self.compose_shift(source, building, palette, [delta, 0], scale)
    }
    pub fn compose_shift(
        &self,
        source: &MemMap,
        building: &[u8],
        palette: &[u8],
        delta: [i32; 2],
        scale: usize,
    ) -> Vec<u16> {
        self.compose_camera(source, building, palette, delta, scale, 1.0)
    }
    pub fn compose_camera(
        &self,
        source: &MemMap,
        building: &[u8],
        palette: &[u8],
        delta: [i32; 2],
        scale: usize,
        zoom: f64,
    ) -> Vec<u16> {
        self.compose_camera_hud(
            source,
            building,
            palette,
            delta,
            scale,
            zoom,
            Default::default(),
        )
    }
    pub fn compose_camera_hud(
        &self,
        source: &MemMap,
        building: &[u8],
        palette: &[u8],
        delta: [i32; 2],
        scale: usize,
        zoom: f64,
        hud_settings: crate::hud_layout::Settings,
    ) -> Vec<u16> {
        self.compose_view(
            source,
            building,
            palette,
            delta,
            crate::viewport::View::new(scale, Default::default()),
            zoom,
            hud_settings,
            None,
        )
    }
    pub fn compose_view(
        &self,
        source: &MemMap,
        building: &[u8],
        palette: &[u8],
        delta: [i32; 2],
        view: crate::viewport::View,
        zoom: f64,
        hud_settings: crate::hud_layout::Settings,
        world: Option<&crate::ground_projection::WorldGround>,
    ) -> Vec<u16> {
        let scale = view.scale;
        use crate::color_effects::{Effects, Pixel};
        // The USA gameplay HUD has a screen-fixed 32x32 weapon OBJ at (200,8).
        // Without that marker, BG1 may be a dialogue or full-screen panel:
        // preserve its layout rather than cutting it into gameplay HUD regions.
        let standard_hud = self.sprites.iter().any(|s| {
            s.priority == 0
                && s.slope == [0.0; 2]
                && s.x == 200
                && s.y == 8
                && s.width == 32
                && s.height == 32
        });
        let hud_settings = if standard_hud {
            hud_settings
        } else {
            Default::default()
        };
        let layout = (hud_settings != Default::default() || view.padding != 0)
            .then(|| crate::hud_layout::Layout::new_view(hud_settings, view, standard_hud));
        let hud_pixel = |x: usize, y: usize| match &layout {
            Some(layout) => layout.pixel(&self.hud, x, y),
            None => self.hud[y / scale * 240 + x / scale],
        };
        let width = view.width;
        let height = view.height;
        let len = width * height;
        let control = half(&source.io, 0);
        let ground =
            crate::ground_projection::continuous_layer_world(source, view, delta, zoom, world);
        let mut objects = vec![None; len];
        let mut order: Vec<_> = self.sprites.iter().collect();
        order.sort_by_key(|s| std::cmp::Reverse((s.priority, s.order)));
        if control & 0x1000 != 0 {
            for s in order {
                let position = [
                    s.x as f64 + s.correction[0] - s.slope[0] * delta[0] as f64 / 65536.0,
                    s.y as f64 + s.correction[1] - s.slope[1] * delta[1] as f64 / 65536.0,
                ];
                let (position, factor) = if s.slope == [0.0; 2] && s.priority == 0 {
                    crate::hud_layout::sprite(
                        hud_settings,
                        position,
                        [s.width as f64 * s.zoom, s.height as f64 * s.zoom],
                    )
                } else {
                    (position, 1.0)
                };
                let edge = if standard_hud && s.slope == [0.0; 2] && s.priority == 0 {
                    crate::hud_layout::edge_offset(
                        [s.x as f64, s.y as f64],
                        [s.width as f64, s.height as f64],
                        view.offset(),
                    )
                } else {
                    0.0
                };
                let [left, top] = position;
                let x_offset = (view.padding as f64 + edge * scale as f64).round();
                let sprite_zoom = s.zoom * factor;
                let bound =
                    |v: f64, n: usize| (v * scale as f64).ceil().clamp(0.0, n as f64) as usize;
                let x_bound = |v: f64| {
                    (v * scale as f64 + x_offset)
                        .ceil()
                        .clamp(0.0, width as f64) as usize
                };
                let x_start = x_bound(left);
                let x_end = x_bound(left + s.width as f64 * sprite_zoom);
                let y_start = bound(top, height);
                let y_end = bound(top + s.height as f64 * sprite_zoom, height);
                if x_start == x_end || y_start == y_end {
                    continue;
                }
                let local_x: Vec<_> = (x_start..x_end)
                    .map(|x| ((x as f64 - x_offset) / scale as f64 - left) / sprite_zoom)
                    .collect();
                for y in y_start..y_end {
                    let local_y = (y as f64 / scale as f64 - top) / sprite_zoom;
                    for (offset, &local_x) in local_x.iter().enumerate() {
                        let x = x_start + offset;
                        if let Some(color) = s.sample_local([local_x, local_y], true) {
                            objects[y * width + x] = Some((
                                Pixel {
                                    color,
                                    layer: 4,
                                    semi: s.semi,
                                },
                                s.priority * 8,
                                s.hires && s.slope != [0.0; 2],
                            ));
                        }
                    }
                }
            }
        }
        let priorities = [
            half(&source.io, 8) & 3,
            half(&source.io, 10) & 3,
            half(&source.io, 12) & 3,
        ];
        let backdrop = Pixel {
            color: half(&source.palette, 0) & 0x7fff,
            layer: 5,
            semi: false,
        };
        let mut image = vec![0; len];
        let effects = Effects::new(&source.io);
        let windows = crate::rect_windows::Windows::new(&source.io)
            .with_view(
                std::array::from_fn(|i| {
                    let slope = i32::from_le_bytes(
                        source.iwram[0x60 + i * 4..0x64 + i * 4].try_into().unwrap(),
                    ) as f64
                        / 65536.0;
                    delta[i] as f64 / 65536.0 * slope
                }),
                zoom,
            )
            .prepare_view(view);
        if effects.inactive(self.sprites.iter().any(|s| s.semi)) {
            // No second color can affect output. Resolve just the nearest
            // visible surface; avoid blend bookkeeping for every pixel.
            for y in 0..height {
                for x in 0..width {
                    let i = y * width + x;
                    let mask = windows.mask(x, y);
                    let mut front = (backdrop.color, 40);
                    if control & 0x400 != 0 && mask & 4 != 0 {
                        if let Some(color) = ground[i] {
                            front = (color, priorities[2] * 8 + 3);
                        }
                    }
                    if control & 0x100 != 0
                        && mask & 1 != 0
                        && priorities[0] * 8 + 1 < front.1
                        && building[i] != 0
                    {
                        front = (
                            half(palette, building[i] as usize * 2),
                            priorities[0] * 8 + 1,
                        );
                    }
                    if control & 0x200 != 0 && mask & 2 != 0 && priorities[1] * 8 + 2 < front.1 {
                        if let Some(color) = hud_pixel(x, y) {
                            front = (color, priorities[1] * 8 + 2);
                        }
                    }
                    if let Some((pixel, rank, _)) = objects[i] {
                        if mask & 16 != 0 && rank < front.1 {
                            front = (pixel.color, rank);
                        }
                    }
                    image[i] = front.0 & 0x7fff;
                }
            }
            return image;
        }
        for y in 0..height {
            for x in 0..width {
                let i = y * width + x;
                let mask = windows.mask(x, y);
                let mut first = (backdrop, 40u16);
                let mut second = (backdrop, 41u16);
                let mut insert = |pixel: Pixel, rank: u16| {
                    if rank < first.1 {
                        second = first;
                        first = (pixel, rank);
                    } else if rank < second.1 {
                        second = (pixel, rank);
                    }
                };
                if control & 0x100 != 0 && mask & 1 != 0 && building[i] != 0 {
                    insert(
                        Pixel {
                            color: half(palette, building[i] as usize * 2) & 0x7fff,
                            layer: 0,
                            semi: false,
                        },
                        priorities[0] * 8 + 1,
                    );
                }
                if control & 0x200 != 0 && mask & 2 != 0 {
                    if let Some(color) = hud_pixel(x, y) {
                        insert(
                            Pixel {
                                color,
                                layer: 1,
                                semi: false,
                            },
                            priorities[1] * 8 + 2,
                        );
                    }
                }
                if control & 0x400 != 0 && mask & 4 != 0 {
                    if let Some(color) = ground[i] {
                        insert(
                            Pixel {
                                color,
                                layer: 2,
                                semi: false,
                            },
                            priorities[2] * 8 + 3,
                        );
                    }
                }
                if let Some((pixel, rank, _)) = objects[i] {
                    if mask & 16 != 0 {
                        insert(pixel, rank);
                    }
                }
                image[i] = effects.apply_window(first.0, second.0, mask & 32 != 0);
                // The game alternates the deck/OBJ priority to suggest visibility
                // beneath a bridge. Keep that world OBJ at a steady half opacity
                // on either phase, without retaining sprites from older scenes.
                // Buildings, HUD and other foreground surfaces still occlude it.
                if windows.bridge_deck(mask) && mask & 16 != 0 {
                    if let Some((object, _, true)) = objects[i] {
                        if object.semi {
                            let ground = if first.0.layer == 2 && second.0.layer == 4 {
                                Some(first.0.color)
                            } else if first.0.layer == 4 && second.0.layer == 2 {
                                Some(second.0.color)
                            } else {
                                None
                            };
                            if let Some(ground) = ground {
                                image[i] =
                                    crate::color_effects::bridge_translucency(object.color, ground);
                            }
                        }
                    }
                }
            }
        }
        image
    }
}
// BG1 is a text background in mode 1. Read its tile spans directly instead
// of allocating two complete GPU copies and compositing two full frames to
// infer transparency from different backdrop colors.
fn hud_layer(source: &MemMap) -> Vec<Option<u16>> {
    let cnt = half(&source.io, 0x0a);
    let char_base = ((cnt >> 2) & 3) as usize * 0x4000;
    let screen_base = ((cnt >> 8) & 31) as usize * 0x800;
    let eight = cnt & 0x80 != 0;
    let (wt, ht) = match cnt >> 14 {
        0 => (32, 32),
        1 => (64, 32),
        2 => (32, 64),
        _ => (64, 64),
    };
    let hofs = (half(&source.io, 0x14) & 511) as usize;
    let vofs = (half(&source.io, 0x16) & 511) as usize;
    let mut layer = vec![None; 38400];
    for y in 0..160 {
        let ty = (y + vofs) & (ht * 8 - 1);
        let mut x = 0;
        while x < 240 {
            let tx = (x + hofs) & (wt * 8 - 1);
            let span = (8 - tx % 8).min(240 - x);
            let block = (ty / 256) * (wt / 32) + tx / 256;
            let entry = half(
                &source.vram,
                screen_base + (block * 1024 + (ty / 8 % 32) * 32 + tx / 8 % 32) * 2,
            );
            let tile = (entry & 1023) as usize;
            let py = if entry & 0x800 != 0 {
                7 - ty % 8
            } else {
                ty % 8
            };
            let row =
                char_base + tile * if eight { 64 } else { 32 } + py * if eight { 8 } else { 4 };
            for i in 0..span {
                let px = if entry & 0x400 != 0 {
                    7 - (tx % 8 + i)
                } else {
                    tx % 8 + i
                };
                let addr = row + if eight { px } else { px / 2 };
                let index = if addr >= 0x10000 {
                    0
                } else if eight {
                    source.vram[addr] as usize
                } else {
                    let byte = source.vram[addr];
                    let nibble = if px & 1 == 0 { byte & 15 } else { byte >> 4 } as usize;
                    if nibble == 0 {
                        0
                    } else {
                        ((entry >> 12) as usize) * 16 + nibble
                    }
                };
                if index != 0 {
                    layer[y * 240 + x + i] = Some(half(&source.palette, index * 2) & 0x7fff);
                }
            }
            x += span;
        }
    }
    layer
}
pub(crate) fn raw_layers(source: &MemMap, layers: u16, backdrop: u16) -> Vec<u16> {
    let mut gpu = MemMap::new(Vec::new());
    gpu.io.clone_from(&source.io);
    gpu.vram.clone_from(&source.vram);
    gpu.palette.clone_from(&source.palette);
    gpu.oam.clone_from(&source.oam);
    let cnt = half(&source.io, 0);
    gpu.io[..2].copy_from_slice(&((cnt & 0x40) | 1 | layers).to_le_bytes());
    gpu.io[0x50..0x56].fill(0);
    gpu.palette[..2].copy_from_slice(&backdrop.to_le_bytes());
    gpu.aff_dirty = [true; 2];
    for y in 0..160 {
        ppu::render_scanline(&mut gpu, y);
    }
    gpu.framebuffer
}
#[cfg(test)]
mod tests {
    #[test]
    fn world_placement_keeps_positive_coordinates_beyond_gba_right_edge() {
        let mut m = Machine::new(Vec::new());
        m.bus.io[..2].copy_from_slice(&1u16.to_le_bytes());
        for slot in 0..128 {
            m.bus.oam[slot * 8 + 1] = 2;
        }
        m.bus.oam[..2].copy_from_slice(&60u16.to_le_bytes());
        m.bus.oam[2..4].copy_from_slice(&248u16.to_le_bytes());
        m.bus.oam[4..6].copy_from_slice(&(3u16 << 10).to_le_bytes());
        let node = 0x0201cec0u32;
        let offset = (node - 0x02000000) as usize;
        m.bus.ewram[offset..offset + 6].copy_from_slice(&m.bus.oam[..6]);
        m.bus.ewram[0x1d6c4..0x1d6c8].copy_from_slice(&node.to_le_bytes());
        let mut observer = Observer::default();
        observer.placements.insert(
            node,
            Placement {
                owner: 0x02002550,
                integer: [248, 60],
                anchor: Anchor {
                    correction: [0.25; 2],
                    slope: [1.; 2],
                    world_screen: [252., 64.],
                },
            },
        );
        let snapshot = observer.live_snapshot(&m.bus).unwrap();
        assert_eq!(snapshot.sprites[0].x, 248);
        assert_eq!(snapshot.sprites[0].y, 60);
        assert_eq!(half(&m.bus.oam, 2), 248);
        observer.placements.clear();
        observer.cached.insert(node, (0x02002550, [248, 60]));
        let prop = observer.live_snapshot(&m.bus).unwrap();
        assert_eq!(prop.sprites[0].x, 248);
        assert_eq!(prop.sprites[0].world_screen, Some([248., 60.]));
    }
    #[test]
    fn palette_refresh_preserves_private_side_texture_when_live_slot_is_absent() {
        let mut bus = MemMap::new(Vec::new());
        bus.vram[0x10000..0x10020].fill(0x11);
        bus.palette[514..516].copy_from_slice(&31u16.to_le_bytes());
        let mut sprite = Sprite::read(&bus, 0).unwrap().unwrap();
        sprite.host_only = true;
        sprite.slot = 127;
        let pixels = sprite.pixels.clone();
        let mut snapshot = Snapshot::new(vec![sprite], &bus);
        bus.oam[127 * 8 + 1] = 2;
        bus.palette[514..516].copy_from_slice(&0x7c00u16.to_le_bytes());
        snapshot.refresh_palette(&bus).unwrap();
        assert!(Arc::ptr_eq(&snapshot.sprites[0].pixels, &pixels));
        assert_eq!(pixels[0], Some(31));
    }
    #[test]
    fn side_sprite_and_oam_sprite_interpolate_only_with_same_checked_actor_root() {
        let bus = MemMap::new(Vec::new());
        let mut a = Sprite::read(&bus, 0).unwrap().unwrap();
        a.node = 0x0201cec0;
        a.owner = 0x02002550;
        a.generation = 7;
        a.slope = [1.; 2];
        a.motion = Some(MotionAnchor {
            identity: [0x02004000, 0x02004100],
            screen: [10., 20.],
            vehicle: false,
        });
        let previous = Snapshot::new(vec![a.clone()], &bus);
        a.node = a.owner;
        a.host_only = true;
        a.generation = 1;
        a.x = 4;
        a.motion.as_mut().unwrap().screen[0] = 14.;
        let current = Snapshot::new(vec![a.clone()], &bus);
        let result = current.interpolate(&previous, 0.5, [0; 2]);
        assert_eq!(result.sprites[0].correction, [-2., 0.]);
        a.motion.as_mut().unwrap().identity[1] += 4;
        let reused = Snapshot::new(vec![a], &bus).interpolate(&previous, 0.5, [0; 2]);
        assert_eq!(reused.sprites[0].correction, [0.; 2]);
    }

    use super::*;
    #[test]
    fn wide_world_sprites_keep_scale_and_exact_center_pixels() {
        let mut source = MemMap::new(Vec::new());
        source.io[..2].copy_from_slice(&0x1001u16.to_le_bytes());
        source.palette[0..2].copy_from_slice(&0x7c00u16.to_le_bytes());
        source.palette[514..516].copy_from_slice(&0x03e0u16.to_le_bytes());
        source.vram[0x10000..0x10020].fill(0x11);
        source.oam[0..2].copy_from_slice(&80u16.to_le_bytes());
        source.oam[2..4].copy_from_slice(&100u16.to_le_bytes());
        source.oam[4..6].copy_from_slice(&0x0c00u16.to_le_bytes());
        let mut sprite = Sprite::read(&source, 0).unwrap().unwrap();
        sprite.slope = [1.0; 2];
        sprite.hires = true;
        let view = crate::viewport::View::new(3, crate::viewport::Aspect::Wide);
        for x in [-8, 0, 100, 232, 244] {
            for correction in [0.0, 0.25, -0.5] {
                sprite.x = x;
                sprite.correction = [correction, 0.25];
                let snapshot = Snapshot::new(vec![sprite.clone()], &source);
                let original = snapshot.compose_camera(
                    &source,
                    &vec![0; 720 * 480],
                    &source.palette,
                    [0, 0],
                    3,
                    1.0,
                );
                let wide = snapshot.compose_view(
                    &source,
                    &vec![0; view.width * view.height],
                    &source.palette,
                    [0, 0],
                    view,
                    1.0,
                    Default::default(),
                    None,
                );
                for y in 0..480 {
                    for x in 0..720 {
                        assert_eq!(
                            original[y * 720 + x],
                            wide[y * view.width + x + view.padding]
                        );
                    }
                }
                if x == 244 {
                    assert!(wide.iter().any(|c| *c == 0x03e0));
                    assert!(!original.iter().any(|c| *c == 0x03e0));
                }
            }
        }
    }
    #[test]
    fn panels_without_gameplay_hud_marker_keep_original_layout() {
        let mut source = MemMap::new(Vec::new());
        source.io[..2].copy_from_slice(&0x0201u16.to_le_bytes());
        let snapshot = Snapshot {
            sprites: Vec::new(),
            hud: Arc::new(vec![Some(0x1234); 38400]),
        };
        let layer = vec![0; 38400];
        let original = snapshot.compose_camera(&source, &layer, &source.palette, [0, 0], 1, 1.0);
        let small = crate::hud_layout::Settings {
            radar: crate::hud_layout::Size::Half,
            status: crate::hud_layout::Size::Half,
            messages: crate::hud_layout::Size::Half,
        };
        let scaled =
            snapshot.compose_camera_hud(&source, &layer, &source.palette, [0, 0], 1, 1.0, small);
        assert_eq!(scaled, original);
        assert!(scaled.iter().all(|p| *p == 0x1234));
    }
    #[test]
    fn direct_hud_matches_two_ppu_passes_for_tile_formats_flips_scroll_and_sizes() {
        let mut source = MemMap::new(Vec::new());
        for (i, byte) in source.vram.iter_mut().enumerate() {
            *byte = ((i * 37 + i / 17) % 256) as u8;
        }
        for (i, color) in source.palette.chunks_exact_mut(2).enumerate() {
            color.copy_from_slice(&((i as u16 * 79) | 0x8000).to_le_bytes());
        }
        let (mut direct_ns, mut reference_ns) = (0u128, 0u128);
        for size in 0..4u16 {
            for eight in [0, 0x80u16] {
                for char_base in [0, 12u16] {
                    for scroll in [0, 7, 509u16] {
                        let cnt = size << 14 | eight | char_base | 0x1800;
                        source.io[0xa..0xc].copy_from_slice(&cnt.to_le_bytes());
                        source.io[0x14..0x16].copy_from_slice(&scroll.to_le_bytes());
                        source.io[0x16..0x18].copy_from_slice(&(511 - scroll).to_le_bytes());
                        let start = std::time::Instant::now();
                        let a = raw_layers(&source, 0x200, 0);
                        let b = raw_layers(&source, 0x200, 0x7fff);
                        let expected: Vec<_> = a
                            .into_iter()
                            .zip(b)
                            .map(|(a, b)| (a == b).then_some(a))
                            .collect();
                        reference_ns += start.elapsed().as_nanos();
                        let start = std::time::Instant::now();
                        let actual = hud_layer(&source);
                        direct_ns += start.elapsed().as_nanos();
                        assert_eq!(actual, expected, "cnt={cnt:04x} scroll={scroll}");
                    }
                }
            }
        }
        eprintln!(
            "hud_sampler reference_ms={:.3} direct_ms={:.3}",
            reference_ns as f64 / 1e6,
            direct_ns as f64 / 1e6
        );
    }
    #[test]
    fn composited_color_effects_and_semitransparent_obj_match_ppu() {
        for mode in 0..4u16 {
            for semi in [false, true] {
                for coefficient in [0, 1, 7, 8, 15, 16, 31u16] {
                    let mut bus = MemMap::new(Vec::new());
                    for i in 0..128 {
                        bus.oam[i * 8 + 1] = 2;
                    }
                    bus.io[..2].copy_from_slice(&0x1341u16.to_le_bytes());
                    bus.io[8..10].copy_from_slice(&0x0182u16.to_le_bytes());
                    bus.io[10..12].copy_from_slice(&0x0285u16.to_le_bytes());
                    bus.vram[..64].fill(1);
                    bus.vram[0x4000..0x4040].fill(2);
                    bus.vram[0x10000..0x10020].fill(0x33);
                    for (i, color) in [(1, 0x4a8du16), (2, 0x06a1), (259, 0x289b)] {
                        bus.palette[i * 2..i * 2 + 2].copy_from_slice(&color.to_le_bytes());
                    }
                    bus.oam[..2]
                        .copy_from_slice(&(8u16 | if semi { 0x400 } else { 0 }).to_le_bytes());
                    bus.oam[2..4].copy_from_slice(&8u16.to_le_bytes());
                    bus.io[0x50..0x52].copy_from_slice(&(0x3f3fu16 | (mode << 6)).to_le_bytes());
                    bus.io[0x52..0x54]
                        .copy_from_slice(&(coefficient | ((31 - coefficient) << 8)).to_le_bytes());
                    bus.io[0x54..0x56].copy_from_slice(&coefficient.to_le_bytes());
                    let sprites = vec![Sprite::read(&bus, 0).unwrap().unwrap()];
                    let snapshot = Snapshot::new(sprites, &bus);
                    let actual =
                        snapshot.compose_shift(&bus, &vec![1; 38400], &bus.palette, [0; 2], 1);
                    for y in 0..160 {
                        ppu::render_scanline(&mut bus, y);
                    }
                    assert_eq!(
                        actual, bus.framebuffer,
                        "mode={mode} semi={semi} coefficient={coefficient}"
                    );
                }
            }
        }
    }
    #[test]
    fn rectangular_windows_layer_masks_and_color_effects_match_ppu() {
        for flags in [0u16, 0x2000, 0x4000, 0x6000] {
            for rect in [[0x0414u16, 0x0614], [0xe008, 0x980c], [0x1010, 0x1010]] {
                for (inside, outside) in [(0x1737u16, 0x37u16), (0x2717, 0x0f)] {
                    for mode in 0..4u16 {
                        for semi in [false, true] {
                            let mut bus = MemMap::new(Vec::new());
                            for i in 0..128 {
                                bus.oam[i * 8 + 1] = 2;
                            }
                            bus.io[..2].copy_from_slice(&(0x1741u16 | flags).to_le_bytes());
                            for (o, v) in [
                                (8, 0x1f83u16),
                                (10, 0x1d84),
                                (12, 0x1e09),
                                (0x40, rect[0]),
                                (0x44, rect[1]),
                                (0x42, 0x0818),
                                (0x46, 0x0218),
                                (0x48, inside),
                                (0x4a, outside),
                                (0x50, 0x3f3f | (mode << 6)),
                                (0x52, 0x0b05),
                                (0x54, 7),
                            ] {
                                bus.io[o..o + 2].copy_from_slice(&v.to_le_bytes());
                            }
                            bus.vram[..64].fill(1); // BG0 opaque
                            bus.vram[0x4000 + 64..0x4000 + 128].fill(2); // BG1 tile 1
                            for x in 0..32 {
                                bus.vram[0xe800 + x * 2] = 1;
                            } // HUD top row
                            bus.vram[0x8000..0x8040].fill(3); // BG2 opaque
                            bus.vram[0x10000..0x10020].fill(0x44);
                            for (i, c) in [(1, 0x4a8du16), (2, 0x06a1), (3, 0x03e0), (260, 0x289b)]
                            {
                                bus.palette[i * 2..i * 2 + 2].copy_from_slice(&c.to_le_bytes());
                            }
                            bus.oam[..2].copy_from_slice(
                                &(8u16 | if semi { 0x400 } else { 0 }).to_le_bytes(),
                            );
                            bus.oam[2..4].copy_from_slice(&8u16.to_le_bytes());
                            let sprite = Sprite::read(&bus, 0).unwrap().unwrap();
                            let snapshot = Snapshot::new(vec![sprite], &bus);
                            let actual = snapshot.compose_shift(
                                &bus,
                                &vec![1; 38400],
                                &bus.palette,
                                [0; 2],
                                1,
                            );
                            for y in 0..160 {
                                ppu::render_scanline(&mut bus, y);
                            }
                            if let Some(i) = actual
                                .iter()
                                .zip(bus.framebuffer.iter())
                                .position(|(a, b)| a != b)
                            {
                                panic!("pixel {},{} actual={:04x} expected={:04x} flags={flags:04x} rect={rect:?} inside={inside:04x} mode={mode} semi={semi}", i%240,i/240,actual[i],bus.framebuffer[i]);
                            }
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn bridge_ground_reveal_matches_ppu_and_hides_car_only_inside_deck() {
        let mut bus = MemMap::new(Vec::new());
        for i in 0..128 {
            bus.oam[i * 8 + 1] = 2;
        }
        for (o, v) in [
            (0, 0x3741u16),
            (8, 0x1f83),
            (10, 0x1d84),
            (12, 0x1e09),
            (0x40, 0x000c),
            (0x44, 0x00a0),
            (0x48, 0x17),
            (0x4a, 0x37),
            (0x50, 0x1044),
            (0x52, 0x1f00),
        ] {
            bus.io[o..o + 2].copy_from_slice(&v.to_le_bytes());
        }
        bus.vram[..64].fill(1);
        bus.vram[0x8000..0x8040].fill(3);
        bus.vram[0x10000..0x10020].fill(0x44);
        for (i, c) in [(1, 0x7c00u16), (3, 0x03e0), (260, 0x001f)] {
            bus.palette[i * 2..i * 2 + 2].copy_from_slice(&c.to_le_bytes());
        }
        bus.oam[..2].copy_from_slice(&8u16.to_le_bytes());
        bus.oam[2..4].copy_from_slice(&8u16.to_le_bytes());
        bus.oam[4..6].copy_from_slice(&(2u16 << 10).to_le_bytes());
        let snapshot = Snapshot::new(vec![Sprite::read(&bus, 0).unwrap().unwrap()], &bus);
        let actual = snapshot.compose_shift(&bus, &vec![1; 38400], &bus.palette, [0; 2], 1);
        for y in 0..160 {
            ppu::render_scanline(&mut bus, y);
        }
        assert_eq!(actual, bus.framebuffer);
        assert_eq!(actual[10 * 240 + 10], 0x03e0); // deck hides the car
        assert_eq!(actual[10 * 240 + 14], 0x001f); // reveal outside the deck
    }
    #[test]
    fn palette_refresh_updates_cached_world_and_hud_colors_without_moving_actors() {
        let mut bus = MemMap::new(Vec::new());
        bus.io[..2].copy_from_slice(&0x1741u16.to_le_bytes());
        bus.io[10..12].copy_from_slice(&0x1c84u16.to_le_bytes());
        for i in 0..128 {
            bus.oam[i * 8 + 1] = 2;
        }
        bus.oam[..2].copy_from_slice(&8u16.to_le_bytes());
        bus.oam[2..4].copy_from_slice(&8u16.to_le_bytes());
        bus.vram[0x10000..0x10020].fill(0x44);
        bus.vram[0x4000..0x4040].fill(1);
        bus.palette[520..522].copy_from_slice(&0x001fu16.to_le_bytes());
        bus.palette[2..4].copy_from_slice(&0x03e0u16.to_le_bytes());
        let mut sprite = Sprite::read(&bus, 0).unwrap().unwrap();
        sprite.correction = [0.25, -0.5];
        sprite.hires = true;
        sprite.slope = [1.0; 2];
        sprite.generation = 42;
        let mut snapshot = Snapshot::new(vec![sprite], &bus);
        let old = snapshot.clone();
        assert_eq!(
            old.sprites[0].sample_shift(9.0, 9.0, [0; 2], true),
            Some(0x001f)
        );
        assert_eq!(old.hud[0], Some(0x03e0));
        bus.palette[520..522].copy_from_slice(&0x7c00u16.to_le_bytes());
        bus.palette[2..4].copy_from_slice(&0x7fffu16.to_le_bytes());
        snapshot.refresh_palette(&bus).unwrap();
        assert_eq!(
            snapshot.sprites[0].sample_shift(9.0, 9.0, [0; 2], true),
            Some(0x7c00)
        );
        assert_eq!(snapshot.sprites[0].pixels[0], Some(0x7c00));
        assert_eq!(snapshot.hud[0], Some(0x7fff));
        assert_eq!(snapshot.sprites[0].correction, [0.25, -0.5]);
        assert_eq!(snapshot.sprites[0].generation, 42);
        assert_eq!(old.sprites[0].pixels[0], Some(0x001f));
    }
    #[test]
    fn enhanced_bridge_translucency_is_stable_across_priority_phases() {
        let mut bus = MemMap::new(Vec::new());
        for i in 0..128 {
            bus.oam[i * 8 + 1] = 2;
        }
        for (o, v) in [
            (0, 0x3741u16),
            (8, 0x1f83),
            (10, 0x1d84),
            (12, 0x1e09),
            (0x40, 0x000c),
            (0x44, 0x00a0),
            (0x48, 0x17),
            (0x4a, 0x37),
            (0x50, 0x1044),
            (0x52, 0x1f00),
        ] {
            bus.io[o..o + 2].copy_from_slice(&v.to_le_bytes());
        }
        bus.vram[..64].fill(1);
        bus.vram[0x8000..0x8040].fill(3);
        bus.vram[0x10000..0x10020].fill(0x44);
        for (i, c) in [(1, 0x7c00u16), (3, 0x03e0), (260, 0x001f)] {
            bus.palette[i * 2..i * 2 + 2].copy_from_slice(&c.to_le_bytes());
        }
        bus.oam[..2].copy_from_slice(&0x408u16.to_le_bytes());
        bus.oam[2..4].copy_from_slice(&8u16.to_le_bytes());
        let mut sprite = Sprite::read(&bus, 0).unwrap().unwrap();
        sprite.hires = true;
        sprite.slope = [1.0; 2];
        let compose = |sprite: Sprite, bus: &MemMap| {
            Snapshot::new(vec![sprite], bus).compose_shift(
                bus,
                &vec![1; 38400],
                &bus.palette,
                [0; 2],
                1,
            )
        };
        sprite.priority = 2; // deck in front
        let behind = compose(sprite.clone(), &bus);
        sprite.priority = 0; // OBJ in front
        let front = compose(sprite.clone(), &bus);
        assert_eq!(behind[10 * 240 + 10], 0x01ef); // half red, half green
        assert_eq!(front[10 * 240 + 10], behind[10 * 240 + 10]);
        assert_eq!(front[10 * 240 + 14], 0x001f); // outside deck stays opaque
        assert_eq!(behind[10 * 240 + 14], 0x001f);
        sprite.hires = false; // original PPU oracle stays unchanged
        assert_eq!(compose(sprite.clone(), &bus)[10 * 240 + 10], 0x001f);
        sprite.hires = true;
        sprite.priority = 2;
        bus.io[8..10].copy_from_slice(&0x1f80u16.to_le_bytes()); // building above OBJ
        assert_eq!(compose(sprite.clone(), &bus)[10 * 240 + 10], 0x7c00);
        bus.io[8..10].copy_from_slice(&0x1f83u16.to_le_bytes());
        bus.io[0..2].copy_from_slice(&0x1741u16.to_le_bytes()); // not a bridge
        assert_eq!(compose(sprite, &bus)[10 * 240 + 10], 0x001f);
    }
    #[test]
    fn temporal_objects_require_same_identity_and_keep_screen_coordinates_fixed() {
        let mut bus = MemMap::new(Vec::new());
        bus.oam[2..4].copy_from_slice(&10u16.to_le_bytes());
        let mut sprite = Sprite::read(&bus, 0).unwrap().unwrap();
        sprite.node = 0x0201cec0;
        sprite.owner = 0x02025284;
        sprite.generation = 7;
        sprite.slope = [0.5; 2];
        sprite.correction = [0.25, 0.5];
        sprite.x = 10;
        sprite.y = 20;
        sprite.world_screen = Some([10.25, 20.5]);
        let previous = Snapshot::new(vec![sprite.clone()], &bus);
        sprite.x = 14;
        sprite.y = 24;
        sprite.world_screen = Some([14.25, 24.5]);
        let current = Snapshot::new(vec![sprite.clone()], &bus);
        let blended = current.interpolate(&previous, 0.5, [-65536; 2]);
        let s = &blended.sprites[0];
        assert!((s.x as f64 + s.correction[0] + 0.5 - 12.25).abs() < 1e-12);
        assert!((s.y as f64 + s.correction[1] + 0.5 - 22.5).abs() < 1e-12);
        sprite.generation = 8;
        let reused =
            Snapshot::new(vec![sprite.clone()], &bus).interpolate(&previous, 0.5, [-65536; 2]);
        assert_eq!(reused.sprites[0].correction, sprite.correction);
        sprite.generation = 7;
        sprite.slope = [0.0; 2];
        let screen =
            Snapshot::new(vec![sprite.clone()], &bus).interpolate(&previous, 0.5, [-65536; 2]);
        assert_eq!(screen.sprites[0].correction, sprite.correction);
    }
    #[test]
    fn animation_pivot_changes_snap_with_pose_while_world_motion_still_interpolates() {
        let bus = MemMap::new(Vec::new());
        let mut sprite = Sprite::read(&bus, 0).unwrap().unwrap();
        sprite.node = 0x0201cf10;
        sprite.owner = 0x02002390;
        sprite.generation = 3;
        sprite.slope = [1.0; 2];
        sprite.x = 90;
        sprite.y = 80;
        sprite.world_screen = Some([100.0, 100.0]);
        let old = Snapshot::new(vec![sprite.clone()], &bus);
        sprite.x = 82;
        sprite.y = 85;
        // A larger animation cell changes pose atomically.
        sprite.width = 16;
        sprite.height = 32;
        sprite.pixels = Arc::new(vec![None; 16 * 32]);
        let pose = Snapshot::new(vec![sprite.clone()], &bus);
        for alpha in [0.0, 0.25, 0.5, 1.0] {
            let result = pose.interpolate(&old, alpha, [0; 2]);
            let s = &result.sprites[0];
            assert_eq!(
                [s.x as f64 + s.correction[0], s.y as f64 + s.correction[1]],
                [82.0, 85.0]
            );
        }
        sprite.x = 86;
        sprite.world_screen = Some([104.0, 100.0]);
        let moved = Snapshot::new(vec![sprite], &bus).interpolate(&pose, 0.5, [0; 2]);
        assert_eq!(
            moved.sprites[0].x as f64 + moved.sprites[0].correction[0],
            84.0
        );
    }
    #[test]
    fn temporary_animation_world_point_and_bitmap_change_atomically() {
        let mut bus = MemMap::new(Vec::new());
        bus.io[..2].copy_from_slice(&0x1001u16.to_le_bytes());
        let mut sprite = Sprite::read(&bus, 0).unwrap().unwrap();
        sprite.node = 0x0201cf00;
        sprite.owner = 0x02025284;
        sprite.generation = 3;
        sprite.slope = [1.0; 2];
        sprite.x = 100;
        sprite.y = 70;
        sprite.world_screen = Some([135.54931640625, 101.13534450531006]);
        let old = Snapshot::new(vec![sprite.clone()], &bus);
        sprite.x = 94;
        sprite.y = 44;
        sprite.world_screen = Some([128.990478515625, 75.62875270843506]);
        // Keep the dimensions and local pivot identical: bitmap identity
        // must still prevent treating a pose-world-point change as motion.
        sprite.correction = [
            128.990478515625 - 135.54931640625 + 6.0,
            75.62875270843506 - 101.13534450531006 + 26.0,
        ];
        Arc::make_mut(&mut sprite.pixels)[0] = Some(0x1234);
        let pose = Snapshot::new(vec![sprite.clone()], &bus);
        for alpha in [0.0, 0.25, 0.5, 1.0] {
            for shift in [[0; 2], [65536, -32768]] {
                let result = pose.interpolate(&old, alpha, shift);
                let expected = pose.compose_shift(&bus, &vec![0; 38400], &bus.palette, shift, 1);
                assert!(expected.contains(&0x1234));
                assert_eq!(
                    result.compose_shift(&bus, &vec![0; 38400], &bus.palette, shift, 1),
                    expected
                );
                assert_eq!(result.sprites[0].correction, sprite.correction);
            }
        }
    }
    #[test]
    fn observed_actor_root_keeps_body_and_feet_together_across_pose_changes() {
        let mut m = Machine::new(Vec::new());
        let parent = 0x02001000u32;
        let position = 0x02002000u32;
        let base = 0x02003000u32;
        let body = base + 4;
        let feet = 0x02004000u32;
        let put = |m: &mut Machine, address: u32, value: u32| {
            let i = (address - 0x02000000) as usize;
            m.bus.ewram[i..i + 4].copy_from_slice(&value.to_le_bytes());
        };
        put(&mut m, parent + 24, position);
        put(&mut m, body, base + 76);
        put(&mut m, feet, position);
        put(&mut m, position, 100 * 65536);
        put(&mut m, position + 4, 80 * 65536);
        m.cpu.regs[0] = position + 12;
        m.cpu.regs[1] = base + 76;
        m.cpu.regs[3] = 100 * 65536;
        m.cpu.regs[4] = 80 * 65536;
        m.cpu.regs[6] = base;
        m.cpu.regs[13] = 0x03007000;
        m.bus.iwram[0x7010..0x7014].copy_from_slice(&parent.to_le_bytes());
        for i in [0x60, 0x64] {
            m.bus.iwram[i..i + 4].copy_from_slice(&65536i32.to_le_bytes());
        }
        let memory = m.bus.ewram.clone();
        let registers = m.cpu.regs;
        let mut roots = MotionRoots::default();
        roots.observe(&m);
        assert_eq!(m.bus.ewram, memory);
        assert_eq!(m.cpu.regs, registers);
        let mut torso = Sprite::read(&m.bus, 0).unwrap().unwrap();
        torso.owner = body;
        torso.node = 0x02005000;
        torso.generation = 1;
        torso.slope = [1.0; 2];
        torso.x = 90;
        torso.y = 70;
        torso.world_screen = Some([120.0, 100.0]);
        let mut legs = torso.clone();
        legs.owner = feet;
        legs.node += 16;
        legs.x = 96;
        legs.y = 78;
        legs.world_screen = Some([100.0, 80.0]);
        let mut old = Snapshot::new(vec![torso.clone(), legs.clone()], &m.bus);
        old.bind_motion_roots(&roots, &m.bus);
        assert!(old.sprites.iter().all(|s| s.motion.is_some()));
        // Physics advances after pose construction; both parts must use
        // the captured physical root, not the temporary animation point.
        put(&mut m, position, 104 * 65536);
        m.cpu.regs[3] = 104 * 65536;
        roots.observe(&m); // this pose was built at 104
        put(&mut m, position, 108 * 65536); // later physics must not move this pose's root
        torso.x = 86;
        torso.y = 76;
        torso.world_screen = Some([128.0, 75.0]);
        torso.width = 16;
        torso.height = 16;
        torso.pixels = Arc::new(vec![Some(0x1234); 256]);
        legs.x = 100;
        Arc::make_mut(&mut legs.pixels)[0] = Some(0x4321);
        let mut current = Snapshot::new(vec![torso, legs], &m.bus);
        current.bind_motion_roots(&roots, &m.bus);
        assert_eq!(current.sprites[0].motion.unwrap().screen, [104.0, 80.0]);
        for alpha in [0.0, 0.25, 0.5, 1.0] {
            let blended = current.interpolate(&old, alpha, [0; 2]);
            let xy = |s: &Sprite| [s.x as f64 + s.correction[0], s.y as f64 + s.correction[1]];
            let body = xy(&blended.sprites[0]);
            let feet = xy(&blended.sprites[1]);
            assert_eq!(body, [82.0 + 4.0 * alpha, 76.0]);
            assert_eq!(feet, [96.0 + 4.0 * alpha, 78.0]);
            assert_eq!([body[0] - feet[0], body[1] - feet[1]], [-14.0, -2.0]);
        }
        // A parent pointer changed by reuse cannot retain this binding.
        put(&mut m, parent + 24, position + 16);
        current.bind_motion_roots(&roots, &m.bus);
        assert!(current.sprites.iter().all(|s| s.motion.is_none()));
        put(&mut m, parent + 24, position);
        m.bus.frames = 121;
        current.bind_motion_roots(&roots, &m.bus);
        assert!(current.sprites.iter().all(|s| s.motion.is_none()));
    }
    #[test]
    fn camera_zoom_normalizes_old_roots_scales_world_objects_and_keeps_hud_fixed() {
        let mut old_graphics = MemMap::new(Vec::new());
        let mut graphics = MemMap::new(Vec::new());
        graphics.io[..2].copy_from_slice(&0x1001u16.to_le_bytes());
        for (bus, bound, scale) in [
            (&mut old_graphics, -120i32, 65536i32),
            (&mut graphics, -240i32, 32768i32),
        ] {
            for (o, v) in [(0x50, bound), (0x54, bound), (0x60, scale), (0x64, scale)] {
                bus.iwram[o..o + 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        let mut world = Sprite::read(&graphics, 0).unwrap().unwrap();
        world.node = 0x02001000;
        world.owner = 0x02002000;
        world.generation = 1;
        world.slope = [0.5; 2];
        world.x = 115;
        world.y = 115;
        world.pixels = Arc::new(vec![Some(0x1234); 64]);
        world.motion = Some(MotionAnchor {
            identity: [1, 2],
            screen: [130.0; 2],
            vehicle: false,
        });
        let mut old_world = world.clone();
        old_world.motion.as_mut().unwrap().screen = [140.0; 2];
        let previous = Snapshot::new(vec![old_world], &old_graphics);
        let mut hud = world.clone();
        hud.node = 0;
        hud.owner = 0;
        hud.slope = [0.0; 2];
        hud.x = 10;
        hud.y = 10;
        hud.pixels = Arc::new(vec![Some(0x4321); 64]);
        let current = Snapshot::new(vec![world, hud], &graphics);
        for alpha in [0.0, 0.5, 1.0] {
            let objects = current
                .interpolate_projected(&previous, &old_graphics, &graphics, alpha, [0; 2])
                .zoomed(2.0);
            let image = objects.compose_camera(
                &graphics,
                &vec![0; 38400],
                &graphics.palette,
                [0; 2],
                1,
                2.0,
            );
            assert_eq!(image.iter().filter(|&&p| p == 0x1234).count(), 256);
            assert_eq!(image.iter().filter(|&&p| p == 0x4321).count(), 64);
            for y in 110..126 {
                for x in 110..126 {
                    assert_eq!(image[y * 240 + x], 0x1234);
                }
            }
            for y in 10..18 {
                for x in 10..18 {
                    assert_eq!(image[y * 240 + x], 0x4321);
                }
            }
        }
    }
    #[test]
    fn high_resolution_affine_sampling_preserves_texels_lost_at_gba_resolution() {
        let mut bus = MemMap::new(Vec::new());
        bus.io[..2].copy_from_slice(&0x1001u16.to_le_bytes());
        for i in 0..128 {
            bus.oam[i * 8 + 1] = 2;
        }
        bus.oam[..2].copy_from_slice(&0x030au16.to_le_bytes());
        bus.oam[2..4].copy_from_slice(&10u16.to_le_bytes());
        bus.oam[6..8].copy_from_slice(&512i16.to_le_bytes());
        bus.oam[30..32].copy_from_slice(&512i16.to_le_bytes());
        // A single source texel at (3,3) is completely missed by the
        // 2:1 GBA affine minification grid, but fits the enhanced 3x grid.
        bus.vram[0x10000 + 3 * 4 + 1] = 0x10;
        bus.palette[514..516].copy_from_slice(&0x1234u16.to_le_bytes());
        let mut sprite = Sprite::read(&bus, 0).unwrap().unwrap();
        assert!(sprite.pixels.iter().all(Option::is_none));
        sprite.hires = true;
        sprite.slope = [1.0; 2];
        assert_eq!(
            sprite.sample(10.0 + 23.0 / 3.0, 10.0 + 23.0 / 3.0, 0, true),
            Some(0x1234)
        );
        let snapshot = Snapshot::new(vec![sprite], &bus);
        let image =
            snapshot.compose_camera(&bus, &vec![0; 38400 * 9], &bus.palette, [0; 2], 3, 1.0);
        assert!(image.contains(&0x1234));
        assert_eq!(
            snapshot.sprites[0].sample(10.0 + 23.0 / 3.0, 10.0 + 23.0 / 3.0, 0, false),
            None
        );
    }
    #[test]
    fn checked_vehicle_position_and_rotation_blend_independently_of_legacy_bitmap() {
        let mut m = Machine::new(Vec::new());
        let (controller, parent, position, owner) =
            (0x02001000u32, 0x02002000u32, 0x02003000u32, 0x02004000u32);
        let put = |m: &mut Machine, a: u32, v: u32| {
            let i = (a - 0x02000000) as usize;
            m.bus.ewram[i..i + 4].copy_from_slice(&v.to_le_bytes());
        };
        for (a, v) in [
            (controller, position),
            (controller + 48, parent),
            (controller + 52, VEHICLE_UPDATE),
            (controller + 60, owner),
            (parent + 24, position),
            (parent + 28, controller),
            (owner, position),
            (position, 120 * 65536),
            (position + 4, 120 * 65536),
        ] {
            put(&mut m, a, v);
        }
        for o in [0x60, 0x64] {
            m.bus.iwram[o..o + 4].copy_from_slice(&65536i32.to_le_bytes());
        }
        m.cpu.regs[0] = controller;
        let mut roots = MotionRoots::default();
        roots.observe_vehicle(&m);
        let mut sprite = Sprite::read(&m.bus, 0).unwrap().unwrap();
        sprite.node = 0x02005000;
        sprite.owner = owner;
        sprite.generation = 1;
        sprite.slope = [1.0; 2];
        sprite.x = 112;
        sprite.y = 112;
        sprite.world_screen = Some([120.0; 2]);
        sprite.affine = Some([1.0, 0.0, 0.0, 1.0]);
        let mut old = Snapshot::new(vec![sprite.clone()], &m.bus);
        old.bind_motion_roots(&roots, &m.bus);
        assert!(old.sprites[0].motion.unwrap().vehicle);
        put(&mut m, position, 124 * 65536);
        sprite.x = 116;
        sprite.world_screen = Some([124.0, 120.0]);
        let c = std::f64::consts::FRAC_1_SQRT_2;
        sprite.affine = Some([c, -c, c, c]);
        // Quantized legacy images differ on every turn: this used to
        // stop translation interpolation even though it is the same car.
        Arc::make_mut(&mut sprite.pixels)[0] = Some(0x1234);
        let mut current = Snapshot::new(vec![sprite], &m.bus);
        current.bind_motion_roots(&roots, &m.bus);
        let blended = current.interpolate(&old, 0.5, [0; 2]);
        assert_eq!(
            blended.sprites[0].x as f64 + blended.sprites[0].correction[0],
            114.0
        );
        let a = blended.sprites[0].affine.unwrap();
        assert!((a[0] - (std::f64::consts::PI / 8.0).cos()).abs() < 1e-12);
        assert!((a[1] + (std::f64::consts::PI / 8.0).sin()).abs() < 1e-12);
        assert!((a[0] * a[3] - a[1] * a[2] - 1.0).abs() < 1e-12);
        let angle = 175.0f64.to_radians();
        let (sin, cos) = angle.sin_cos();
        old.sprites[0].affine = Some([cos, sin, -sin, cos]);
        current.sprites[0].affine = Some([cos, -sin, sin, cos]);
        let wrapped = current.interpolate(&old, 0.5, [0; 2]).sprites[0]
            .affine
            .unwrap();
        assert!((wrapped[0] + 1.0).abs() < 1e-12 && wrapped[1].abs() < 1e-12);
        put(&mut m, parent + 28, controller + 16);
        roots.observe_vehicle(&m);
        current.bind_motion_roots(&roots, &m.bus);
        assert!(current.sprites[0].motion.is_none());
    }
    #[test]
    fn pool_reuse_discards_old_world_and_cached_placements() {
        let mut observer = Observer::default();
        let node = 0x0201cec0;
        observer.cached.insert(node, (0x02025284, [12, 34]));
        observer.placements.insert(
            node,
            Placement {
                owner: 0x02025284,
                integer: [12, 34],
                anchor: Anchor {
                    correction: [0.25; 2],
                    slope: [1.0; 2],
                    world_screen: [0.0; 2],
                },
            },
        );
        let mut machine = Machine::new(Vec::new());
        machine.cpu.regs[0] = node;
        observer.observe(&machine, 0x08012897).unwrap();
        assert!(!observer.cached.contains_key(&node));
        assert!(!observer.placements.contains_key(&node));
    }
    #[test]
    fn signed_world_projection_retains_camera_and_object_fractions() {
        let (i, f) = axis(-65536 - 32768, -2, -5, 32768, 16384);
        assert_eq!(i, 0);
        assert!((f - 0.625).abs() < 1e-12);
    }
    #[test]
    fn sampler_matches_ppu_for_modes_flips_affine_double_and_negative_placement() {
        for one_dim in [false, true] {
            for eight in [false, true] {
                for variant in 0..6 {
                    let mut bus = MemMap::new(Vec::new());
                    for i in 0..128 {
                        bus.oam[i * 8 + 1] = 2;
                    }
                    bus.io[..2].copy_from_slice(
                        &(0x1001u16 | if one_dim { 0x40 } else { 0 }).to_le_bytes(),
                    );
                    for (i, b) in bus.vram[0x10000..].iter_mut().enumerate() {
                        *b = (i * 37) as u8;
                    }
                    for i in 0..256 {
                        bus.palette[512 + i * 2..514 + i * 2]
                            .copy_from_slice(&(((i * 113) & 0x7fff) as u16).to_le_bytes());
                    }
                    let affine = variant >= 4;
                    let a0 = 252u16
                        | if eight { 0x2000 } else { 0 }
                        | if affine {
                            if variant == 5 {
                                0x300
                            } else {
                                0x100
                            }
                        } else {
                            0
                        };
                    let a1 = 508u16
                        | 0x4000
                        | if !affine {
                            ((variant & 1) << 12) | ((variant & 2) << 12)
                        } else {
                            0
                        };
                    bus.oam[..2].copy_from_slice(&a0.to_le_bytes());
                    bus.oam[2..4].copy_from_slice(&a1.to_le_bytes());
                    for (o, v) in [(6, 224i16), (14, 64), (22, -64), (30, 224)] {
                        bus.oam[o..o + 2].copy_from_slice(&v.to_le_bytes());
                    }
                    let mut sprite = Sprite::read(&bus, 0).unwrap().unwrap();
                    sprite.hires = true;
                    let black = raw_layers(&bus, 0x1000, 0);
                    let white = raw_layers(&bus, 0x1000, 0x7fff);
                    for y in 0..160 {
                        for x in 0..240 {
                            assert_eq!(
                                sprite.sample(x as f64, y as f64, 0, false),
                                (black[y * 240 + x] == white[y * 240 + x])
                                    .then_some(black[y * 240 + x])
                            );
                            assert_eq!(
                                sprite.sample(x as f64, y as f64, 0, true),
                                sprite.sample(x as f64, y as f64, 0, false)
                            );
                        }
                    }
                }
            }
        }
    }
}
