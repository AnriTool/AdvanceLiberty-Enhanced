//! Opt-in presentation only. All observation borrows live state immutably.
use crate::{building_material, building_projection, ground_projection, object_projection};
use gba_core::{Machine, MemMap};
use std::{
    collections::{HashMap, VecDeque},
    io::Write,
    path::PathBuf,
};
const ENTRY: u32 = 0x0800d2b1; // start of the actual building render pass
                               // Both VBlank branches perform the same object preparation/upload. The early
                               // branch is common while driving; observing only the late return freezes the
                               // presentation even though all four capture stages have completed.
const UPLOAD_RETURNS: [u32; 2] = [0x08000293, 0x08000327];
// Loading and vehicle transitions can keep the same draw call open for more
// than twelve VBlanks. Completion is checked by END, stage counts and camera.
const MAX_CAPTURE_FRAMES: u64 = 120;
// Recorded normal captures arrive 2..6 GBA frames apart. A single presentation
// clock avoids restarting unfinished motion when the next interval is shorter.
const PRESENTATION_DELAY: u64 = 6;
fn camera(bus: &MemMap) -> [i32; 3] {
    [0x14, 0x18, 0x1c].map(|o| i32::from_le_bytes(bus.iwram[o..o + 4].try_into().unwrap()))
}
fn display_compatible(control: u16) -> bool {
    // Forced blank is also used briefly while the next scene is uploaded.
    // The completed presentation copy remains usable during that upload.
    control & 0x8007 == 1
}
fn stale_scene(age: u64, unchanged: bool, capture_in_progress: bool) -> bool {
    // Guest camera/VRAM can already describe the next upload. Never discard
    // that upload just because the last completed scene has grown old.
    // Pending captures have their own bounded timeout in observe().
    age > 12 && !unchanged && !capture_in_progress
}
fn capture_can_hold_scene(valid_display_frame: u64, capture_frame: u64) -> bool {
    // Repeatedly replacing an unfinished capture must not renew the lifetime
    // of the same old presentation indefinitely.
    capture_frame <= valid_display_frame.saturating_add(12)
}
fn unchanged_display_layout(scene: &Scene, bus: &MemMap) -> bool {
    // Physics/camera RAM can advance before the next PPU upload. The last
    // completed presentation still describes the displayed GPU state then.
    scene.graphics.io[8..0x50] == bus.io[8..0x50]
        && (u16::from_le_bytes(scene.graphics.io[..2].try_into().unwrap()) & !0x80)
            == (u16::from_le_bytes(bus.io[..2].try_into().unwrap()) & !0x80)
        && scene.graphics.vram == bus.vram
        && scene.graphics.oam == bus.oam
}
fn unchanged_graphics(scene: &Scene, bus: &MemMap) -> bool {
    unchanged_display_layout(scene, bus) && scene.graphics.palette == bus.palette
}
fn scales(bus: &MemMap) -> [i32; 4] {
    [0x60, 0x64, 0x68, 0x6c].map(|o| i32::from_le_bytes(bus.iwram[o..o + 4].try_into().unwrap()))
}
fn delta(a: [i32; 3], b: [i32; 3], alpha: f64) -> [i32; 2] {
    std::array::from_fn(|i| ((a[i] as f64 - b[i] as f64) * (1.0 - alpha)).round() as i32)
}
fn compatible(old: &Scene, new: &Scene) -> bool {
    let span = new.frame.saturating_sub(old.frame);
    let scale_compatible = if old.camera[2] == new.camera[2] {
        scales(&old.graphics) == scales(&new.graphics)
    } else if old.camera[2] > 0
        && new.camera[2] > 0
        && (old.camera[2] as i64 - new.camera[2] as i64).abs() <= 16 * 65536
    {
        let ratio = new.camera[2] as f64 / old.camera[2] as f64;
        scales(&old.graphics)
            .into_iter()
            .zip(scales(&new.graphics))
            .enumerate()
            .all(|(i, (a, b))| {
                let expected = if i < 2 { 1.0 / ratio } else { ratio };
                a > 0 && b > 0 && (b as f64 / a as f64 / expected - 1.0).abs() < 0.02
            })
    } else {
        false
    };
    (1..=8).contains(&span)
        && scale_compatible
        // Camera updates are several GBA frames apart. Bound velocity rather
        // than a fixed distance that rejects ordinary fast vehicle motion.
        && (0..2).all(|i| (old.camera[i] as i64 - new.camera[i] as i64).abs() <= 16 * span as i64 * 65536)
}
struct Pending {
    observer: building_projection::Observer,
    ground: ground_projection::Observer,
    frame: u64,
    camera: [i32; 3],
    stages: [u32; 4],
    rebuilding: bool,
}
struct Scene {
    ground_world: Option<ground_projection::WorldGround>,
    materials: building_material::Capture,
    objects: object_projection::Snapshot,
    graphics: MemMap,
    camera: [i32; 3],
    frame: u64,
    captured_effects: [u8; 6],
}
fn historical_effects(captured: [u8; 6], live: &[u8]) -> [u8; 6] {
    let mode = (u16::from_le_bytes(live[..2].try_into().unwrap()) >> 6) & 3;
    // Brightness fades are global. Bridge/OBJ alpha belongs to the sampled
    // world's windows and poses, not to a newer scene ahead of that clock.
    if mode >= 2 {
        live.try_into().unwrap()
    } else {
        captured
    }
}
fn presentation_pair<'a>(
    history: &'a VecDeque<Scene>,
    previous: Option<&'a Scene>,
    current: Option<&'a Scene>,
    time: f64,
) -> Option<(&'a Scene, Option<&'a Scene>, f64)> {
    let mut older: Option<&Scene> = None;
    for scene in history.iter().chain(previous).chain(current) {
        if scene.frame as f64 >= time {
            let alpha = older.map_or(1.0, |old| {
                ((time - old.frame as f64).max(0.0) / (scene.frame - old.frame).max(1) as f64)
                    .clamp(0.0, 1.0)
            });
            return Some((scene, older, alpha));
        }
        older = Some(scene);
    }
    older.map(|scene| (scene, None, 1.0))
}
fn closest_supported_alpha(requested: f64, minimum: f64, supports: impl Fn(f64) -> bool) -> f64 {
    let (mut invalid, mut right) = (requested, 1.0);
    for _ in 0..16 {
        let mid = (invalid + right) * 0.5;
        if supports(mid) {
            right = mid;
        } else {
            invalid = mid;
        }
    }
    if minimum <= requested && supports(minimum) {
        let (mut left, mut invalid) = (minimum, requested);
        for _ in 0..16 {
            let mid = (left + invalid) * 0.5;
            if supports(mid) {
                left = mid;
            } else {
                invalid = mid;
            }
        }
        if requested - left <= right - requested {
            return left;
        }
    }
    right
}
pub struct Preview {
    wide: bool,
    ground_rom: Option<std::sync::Arc<Vec<u8>>>,
    material_bank: building_material::MaterialBank,
    motion_roots: object_projection::MotionRoots,
    pending: Option<Pending>,
    current: Option<Scene>,
    valid_display_frame: u64,
    previous: Option<Scene>,
    history: VecDeque<Scene>,
    presented_scene: Option<u64>,
    aim_projection: Option<crate::mouse_aim::Projection>,
    presentation_floor: f64,
    epochs: HashMap<u32, u64>,
    epoch: u64,
    output: Option<PathBuf>,
    events: Option<std::fs::File>,
    session_events: Option<std::fs::File>,
    session_summary: Option<PathBuf>,
    session_recovering: bool,
    last_settings: Option<crate::render_settings::Settings>,
    last: Option<Vec<u16>>,
    pub accepted: u64,
    pub rejected: u64,
    pub rendered: u64,
    pub original: u64,
    pub blended: u64,
    transition_resets: u64,
    material_clamps: u64,
    status: &'static str,
    last_reason: String,
    render_ns: u128,
    validate_ns: u128,
    graphics_ns: u128,
    geometry_ns: u128,
    compose_ns: u128,
    coverage: Option<std::fs::File>,
    worst_holes: usize,
    timings: Option<std::fs::File>,
    sprite_anchors: Option<std::fs::File>,
    sprite_presented: Option<std::fs::File>,
    camera_frames: Option<std::fs::File>,
}
impl Preview {
    pub fn aim_projection(&self) -> Option<crate::mouse_aim::Projection> {
        self.aim_projection
    }
    pub fn diagnostic_graphics(&self) -> Option<&MemMap> {
        self.history
            .iter()
            .chain(self.previous.iter())
            .chain(self.current.iter())
            .find(|s| Some(s.frame) == self.presented_scene)
            .map(|s| &s.graphics)
    }
    #[inline]
    pub fn interested(&self, key: u32) -> bool {
        matches!(
            key,
            ENTRY
                | 0x08012771
                | 0x08012897
                | 0x080128a5
                | object_projection::ACTOR_POSITION_COPY
                | object_projection::VEHICLE_UPDATE
        ) || UPLOAD_RETURNS.contains(&key)
            || self.pending.as_ref().is_some_and(|p| {
                matches!(key, 0x0800d301 | 0x08010545 | 0x080127ed | 0x0800fe29)
                    || ground_projection::Observer::interested(key)
                    || p.observer.interested(key)
            })
    }
    pub fn new(output: Option<PathBuf>) -> Result<Self, Box<dyn std::error::Error>> {
        let events = if let Some(path) = &output {
            std::fs::create_dir(path)?;
            let mut f = std::fs::File::create_new(path.join("events.csv"))?;
            writeln!(f, "frame,event,reason")?;
            Some(f)
        } else {
            None
        };
        let coverage = output
            .as_ref()
            .map(|path| {
                let mut f = std::fs::File::create_new(path.join("geometry.csv"))?;
                writeln!(f, "frame,missing_pixels,extra_pixels")?;
                Ok::<_, std::io::Error>(f)
            })
            .transpose()?;
        let timings = output
            .as_ref()
            .map(|path| {
                let mut f = std::fs::File::create_new(path.join("timings.csv"))?;
                writeln!(f, "frame,render_ns,scene_frame,scene_age,image_hash")?;
                Ok::<_, std::io::Error>(f)
            })
            .transpose()?;
        let sprite_anchors = output.as_ref().map(|path| {
            let mut f = std::fs::File::create_new(path.join("sprite-anchors.csv"))?;
            writeln!(f,"frame,node,owner,x,y,correction_x,correction_y,world_x,world_y,pivot_x,pivot_y,bitmap_hash,root_parent,root_position,motion_x,motion_y,affine_pa,affine_pb,affine_pc,affine_pd,hires,vehicle,zoom")?;
            Ok::<_,std::io::Error>(f)
        }).transpose()?;
        let sprite_presented = output.as_ref().map(|path| {
            let mut f = std::fs::File::create_new(path.join("sprite-presented.csv"))?;
            writeln!(f,"frame,node,owner,x,y,correction_x,correction_y,world_x,world_y,pivot_x,pivot_y,bitmap_hash,root_parent,root_position,motion_x,motion_y,affine_pa,affine_pb,affine_pc,affine_pd,hires,vehicle,zoom")?;
            Ok::<_,std::io::Error>(f)
        }).transpose()?;
        let camera_frames = output
            .as_ref()
            .map(|path| {
                let mut f = std::fs::File::create_new(path.join("camera-frames.csv"))?;
                writeln!(
                    f,
                "frame,scene_frame,alpha,source_z,target_z,zoom,shift_x,shift_y,source_x,source_y"
                )?;
                Ok::<_, std::io::Error>(f)
            })
            .transpose()?;
        Ok(Self {
            wide: false,
            ground_rom: None,
            material_bank: Default::default(),
            motion_roots: object_projection::MotionRoots::default(),
            pending: None,
            current: None,
            valid_display_frame: 0,
            previous: None,
            history: VecDeque::new(),
            presented_scene: None,
            aim_projection: None,
            presentation_floor: 0.0,
            epochs: HashMap::new(),
            epoch: 0,
            output,
            events,
            session_events: None,
            session_summary: None,
            session_recovering: false,
            last_settings: None,
            last: None,
            accepted: 0,
            rejected: 0,
            rendered: 0,
            original: 0,
            blended: 0,
            transition_resets: 0,
            material_clamps: 0,
            status: "Camera preview: waiting",
            last_reason: String::new(),
            render_ns: 0,
            validate_ns: 0,
            graphics_ns: 0,
            geometry_ns: 0,
            compose_ns: 0,
            coverage,
            worst_holes: 0,
            timings,
            sprite_anchors,
            sprite_presented,
            camera_frames,
        })
    }
    pub fn log_session(&mut self, directory: &std::path::Path) -> std::io::Result<()> {
        let mut file = std::fs::File::create_new(directory.join("camera-events.csv"))?;
        writeln!(file, "frame,event,reason")?;
        self.session_events = Some(file);
        self.session_summary = Some(directory.join("camera-summary.txt"));
        Ok(())
    }
    fn discard_scene(&mut self) {
        self.aim_projection = None;
        self.pending = None;
        self.current = None;
        self.previous = None;
        self.history.clear();
        self.presented_scene = None;
        self.presentation_floor = 0.0;
    }
    fn event(&mut self, frame: u64, event: &str, reason: &str) {
        if let Some(f) = &mut self.session_events {
            let label = match event {
                "fallback" => {
                    self.session_recovering = true;
                    Some(event)
                }
                "scene" if self.session_recovering => {
                    self.session_recovering = false;
                    Some("recovered")
                }
                "menu" => {
                    self.session_recovering = false;
                    Some(event)
                }
                "settings" | "material_clamp" | "wide_object_failure" => Some(event),
                _ => None,
            };
            if let Some(label) = label {
                let _ = writeln!(
                    f,
                    "{frame},{label},{}",
                    reason.replace([',', '\n', '\r'], " ")
                );
                if label == "fallback" {
                    let _ = f.flush();
                }
            }
        }
        if let Some(f) = &mut self.events {
            let _ = writeln!(
                f,
                "{frame},{event},{}",
                reason.replace([',', '\n', '\r'], " ")
            );
        }
    }
    fn reject(&mut self, frame: u64, reason: String) {
        self.discard_scene();
        self.rejected += 1;
        self.status = "Camera preview: original scene";
        self.event(frame, "fallback", &reason);
        if self.rejected <= 10 && self.last_reason != reason {
            eprintln!("Camera preview fallback: {reason}");
        }
        self.last_reason = reason;
    }
    pub fn observe(&mut self, m: &Machine, key: u32) {
        if key == object_projection::VEHICLE_UPDATE {
            self.motion_roots.observe_vehicle(m);
            return;
        }
        if key == object_projection::ACTOR_POSITION_COPY {
            self.motion_roots.observe(m);
            return;
        }
        if key == 0x08012771 {
            self.epochs.clear();
            self.epoch += 1;
        }
        if key == 0x08012897 || key == 0x080128a5 {
            self.epoch += 1;
            self.epochs.insert(m.cpu.regs[0], self.epoch);
        }
        if key == ENTRY {
            if let Some(p) = &self.pending {
                let reason = format!(
                    "started={} stages={:?} rebuilding={}",
                    p.frame, p.stages, p.rebuilding
                );
                self.event(m.bus.frames, "capture_replaced", &reason);
            }
            self.pending = Some(Pending {
                observer: {
                    let mut observer = building_projection::Observer::default();
                    observer.set_wide(self.wide);
                    observer
                },
                ground: ground_projection::Observer::default(),
                frame: m.bus.frames,
                camera: camera(&m.bus),
                stages: [0; 4],
                rebuilding: false,
            });
        }
        if key == 0x0800fe29 {
            self.event(
                m.bus.frames,
                "upload_call",
                &format!(
                    "lr={:08x} pending={}",
                    m.cpu.regs[14],
                    self.pending.is_some()
                ),
            );
        }
        if UPLOAD_RETURNS.contains(&key) {
            if let Some(p) = self.pending.take() {
                if p.stages != [1; 4] || p.camera != camera(&m.bus) {
                    self.reject(m.bus.frames, "Scene changed during capture".into());
                    return;
                }
                let validation_start = std::time::Instant::now();
                for reason in p.observer.side_failures() {
                    self.event(m.bus.frames, "wide_object_failure", reason);
                }
                let capture = p.observer.into_live(&m.bus);
                self.validate_ns += validation_start.elapsed().as_nanos();
                match capture {
                    Ok((mut materials, mut objects)) => {
                        if !p.rebuilding {
                            let Some(old) = &self.current else {
                                self.reject(
                                    m.bus.frames,
                                    "Cached building scene has no captured predecessor".into(),
                                );
                                return;
                            };
                            // 0800d2fe skips the building draw when its integer
                            // camera and redraw counter permit reuse. Empty
                            // observation here does not mean an empty world.
                            materials = old.materials.translated(std::array::from_fn(|i| {
                                p.camera[i].wrapping_sub(old.camera[i])
                            }));
                            self.event(
                                m.bus.frames,
                                "geometry_reuse",
                                "Guest reused building framebuffer",
                            );
                        }
                        let mut restored = self.material_bank.restore(&mut materials);
                        for old in self.current.iter() {
                            restored += materials.restore_edge_materials(
                                &old.materials,
                                std::array::from_fn(|i| p.camera[i].wrapping_sub(old.camera[i])),
                            );
                        }
                        if restored != 0 {
                            self.event(
                                m.bus.frames,
                                "material_reuse",
                                &format!("restored {restored} verified edge faces"),
                            );
                        }
                        if let Some(f) = self.coverage.as_mut() {
                            let layer = materials.live_layer([0; 2], 1).unwrap();
                            let reference = object_projection::raw_layers(&m.bus, 0x100, 0);
                            let mask = object_projection::raw_layers(&m.bus, 0x100, 0x7fff);
                            let missing = (0..38400)
                                .filter(|&i| reference[i] == mask[i] && layer[i] == 0)
                                .count();
                            let extra = (0..38400)
                                .filter(|&i| reference[i] != mask[i] && layer[i] != 0)
                                .count();
                            let _ = writeln!(f, "{},{missing},{extra}", m.bus.frames);
                            if missing > self.worst_holes {
                                self.worst_holes = missing;
                                if let Some(path) = &self.output {
                                    let image: Vec<_> = layer
                                        .iter()
                                        .map(|&p| {
                                            u16::from_le_bytes(
                                                m.bus.palette[p as usize * 2..p as usize * 2 + 2]
                                                    .try_into()
                                                    .unwrap(),
                                            )
                                        })
                                        .collect();
                                    let _ = crate::scene_replay::ppm(
                                        &path.join(format!("worst-geometry-{}.ppm", m.bus.frames)),
                                        &image,
                                        240,
                                        160,
                                    );
                                    let _ = crate::scene_replay::ppm(
                                        &path.join(format!("worst-reference-{}.ppm", m.bus.frames)),
                                        &reference,
                                        240,
                                        160,
                                    );
                                }
                            }
                        }
                        for node in objects.nodes() {
                            if !self.epochs.contains_key(&node) {
                                self.epoch += 1;
                                self.epochs.insert(node, self.epoch);
                            }
                        }
                        objects.stamp(&self.epochs);
                        objects.bind_motion_roots(&self.motion_roots, &m.bus);
                        if let Some(f) = self.sprite_anchors.as_mut() {
                            objects.trace_anchors(m.bus.frames, [0; 2], f);
                        }
                        let graphics_start = std::time::Instant::now();
                        let mut graphics = MemMap::new(Vec::new());
                        graphics.io.clone_from(&m.bus.io);
                        graphics.iwram.clone_from(&m.bus.iwram);
                        graphics.vram.clone_from(&m.bus.vram);
                        graphics.palette.clone_from(&m.bus.palette);
                        graphics.oam.clone_from(&m.bus.oam);
                        self.graphics_ns += graphics_start.elapsed().as_nanos();
                        let ground_world = if self.wide {
                            let rom = self
                                .ground_rom
                                .get_or_insert_with(|| std::sync::Arc::new(m.bus.rom.clone()))
                                .clone();
                            ground_projection::WorldGround::capture(&m.bus, rom)
                        } else {
                            None
                        };
                        if materials.side_faces > 0 {
                            self.event(
                                m.bus.frames,
                                "wide_geometry",
                                &format!("added_faces={}", materials.side_faces),
                            );
                        }
                        let scene = Scene {
                            ground_world,
                            captured_effects: m.bus.io[0x50..0x56].try_into().unwrap(),
                            materials,
                            objects,
                            graphics,
                            camera: camera(&m.bus),
                            frame: m.bus.frames,
                        };
                        let prior = self.current.take().filter(|old| compatible(old, &scene));
                        if prior.is_some() {
                            if let Some(older) = self.previous.take() {
                                self.history.push_back(older);
                            }
                            while self
                                .history
                                .front()
                                .is_some_and(|s| s.frame.saturating_add(24) < scene.frame)
                                || self.history.len() > 16
                            {
                                self.history.pop_front();
                            }
                        } else {
                            self.history.clear();
                            self.presentation_floor = scene.frame as f64;
                        }
                        self.previous = prior;
                        self.current = Some(scene);
                        self.valid_display_frame = m.bus.frames;
                        self.accepted += 1;
                        self.status = "Camera preview";
                        self.event(
                            m.bus.frames,
                            "scene",
                            &format!(
                                "accepted z={} scales={:?}",
                                camera(&m.bus)[2],
                                scales(&m.bus)
                            ),
                        );
                    }
                    Err(reason) => self.reject(m.bus.frames, reason),
                }
            }
            return;
        }
        if let Some(p) = &mut self.pending {
            if key == 0x0800d301 {
                p.rebuilding = true;
            }
            if m.bus.frames > p.frame + MAX_CAPTURE_FRAMES {
                self.reject(m.bus.frames, "Capture timeout".into());
                return;
            }
            for (i, entry) in [0x0800d2b1, 0x08010545, 0x080127ed, 0x0800fe29]
                .iter()
                .enumerate()
            {
                if key == *entry {
                    p.stages[i] += 1;
                }
            }
            if let Err(reason) = p
                .ground
                .observe(m, key)
                .and_then(|_| p.observer.observe(m, key))
            {
                self.reject(m.bus.frames, reason);
            }
        }
    }
    #[cfg(test)]
    pub fn frame(&mut self, bus: &MemMap) -> Option<Vec<u16>> {
        self.frame_configured(bus, crate::render_settings::Settings::default())
    }
    pub fn frame_configured(
        &mut self,
        bus: &MemMap,
        settings: crate::render_settings::Settings,
    ) -> Option<Vec<u16>> {
        self.aim_projection = None;
        self.wide = settings.aspect == crate::viewport::Aspect::Wide;
        if self.last_settings != Some(settings) {
            self.event(bus.frames, "settings", &format!("{settings:?}"));
            self.last_settings = Some(settings);
        }
        let display = u16::from_le_bytes(bus.io[..2].try_into().unwrap());
        // The ordinary pause menu intentionally disables BG2 and OBJ. It is
        // not a failed world capture, and no stale world should cover it.
        if display & 0x9587 == 0x0101 {
            if self.status != "Original menu" {
                self.event(bus.frames, "menu", "guest menu display");
            }
            self.discard_scene();
            self.status = "Original menu";
            self.original += 1;
            return None;
        }
        let mut palette_animation = false;
        // Palette uploads need not redraw geometry or OAM. Re-decode colors
        // only when the displayed layout is unchanged; never pair a partial
        // new upload with old geometry. All changes stay in presentation copies.
        if let Some(scene) = self.current.as_mut() {
            if scene.graphics.palette != bus.palette && unchanged_display_layout(scene, bus) {
                scene.graphics.palette.clone_from(&bus.palette);
                palette_animation = true;
                if let Err(reason) = scene.objects.refresh_palette(&scene.graphics) {
                    self.reject(bus.frames, reason);
                }
            }
        }
        // Ordinary instructions no longer enter observe(). Expiry must also
        // run at frame boundaries, including a CPU loop without capture hooks.
        if self
            .pending
            .as_ref()
            .is_some_and(|p| bus.frames > p.frame + MAX_CAPTURE_FRAMES)
        {
            self.reject(bus.frames, "Capture timeout".into());
        }
        let unchanged = self
            .current
            .as_ref()
            .is_some_and(|s| unchanged_graphics(s, bus));
        if unchanged {
            self.valid_display_frame = bus.frames;
        }
        if self.current.as_ref().is_some_and(|_| {
            stale_scene(
                bus.frames.saturating_sub(self.valid_display_frame),
                unchanged,
                self.pending
                    .as_ref()
                    .is_some_and(|p| capture_can_hold_scene(self.valid_display_frame, p.frame)),
            )
        }) {
            let s = self.current.as_ref().unwrap();
            let details = format!(
                "camera={} bounds={} controls={} vram={} oam={} palette={} pending={}",
                camera(&s.graphics) == camera(bus),
                s.graphics.iwram[0x50..0x70] == bus.iwram[0x50..0x70],
                s.graphics.io[8..0x50] == bus.io[8..0x50],
                s.graphics.vram == bus.vram,
                s.graphics.oam == bus.oam,
                s.graphics.palette == bus.palette,
                self.pending.is_some()
            );
            self.event(bus.frames, "stale_state", &details);
            self.reject(bus.frames, "Scene refresh timeout".into());
        } else if self.current.is_some()
            && !display_compatible(u16::from_le_bytes(bus.io[..2].try_into().unwrap()))
        {
            self.reject(bus.frames, "Display mode/OBJ-window transition".into());
        }
        if settings.renderer == crate::render_settings::Renderer::Original {
            self.original += 1;
            self.status = "Original output selected";
            return None;
        }
        if self.status == "Original output selected" {
            self.status = "Camera preview: waiting";
        }
        let start = std::time::Instant::now();
        // Fade registers can update every GBA frame between scene uploads.
        // Update the presentation copy only; guest IO remains untouched.
        if let Some(scene) = self.current.as_mut() {
            scene.graphics.io[0x50..0x56].copy_from_slice(&bus.io[0x50..0x56]);
        }
        if let Some(palette) = self.current.as_ref().map(|s| s.graphics.palette.clone()) {
            let mut failure = None;
            for scene in self.history.iter_mut().chain(self.previous.iter_mut()) {
                let fx = historical_effects(scene.captured_effects, &bus.io[0x50..0x56]);
                scene.graphics.io[0x50..0x56].copy_from_slice(&fx);
                if palette_animation && scene.graphics.palette != palette {
                    scene.graphics.palette.clone_from(&palette);
                    if let Err(reason) = scene.objects.refresh_palette(&scene.graphics) {
                        failure = Some(reason);
                        break;
                    }
                }
            }
            if let Some(reason) = failure {
                self.reject(bus.frames, reason);
            }
        }
        let (mut geometry_ns, mut compose_ns) = (0, 0);
        let time =
            (bus.frames.saturating_sub(PRESENTATION_DELAY) as f64).max(self.presentation_floor);
        let pair = presentation_pair(
            &self.history,
            self.previous.as_ref(),
            self.current.as_ref(),
            time,
        );
        let scene_frame = pair.map_or(0, |p| p.0.frame);
        let mut rendered_time = time;
        let mut aim_projection = None;
        let result = pair.map(|(scene, old_scene, mut alpha)| {
            let mut shift = if let Some(old) = old_scene {
                delta(old.camera, scene.camera, alpha)
            } else {
                [0; 2]
            };
            let mut dz = old_scene.map_or(0, |old| {
                ((old.camera[2] as f64 - scene.camera[2] as f64) * (1.0 - alpha)).round() as i32
            });
            let mut reset = false;
            let geometry_start = std::time::Instant::now();
            let viewport = crate::viewport::View::new(3, settings.aspect);
            let layer = match scene.materials.live_layer_view(
                [shift[0], shift[1], dz],
                viewport,
                settings.textures,
            ) {
                Ok(layer) => layer,
                Err(_) if shift != [0; 2] || dz != 0 => {
                    // Clamp only to the closest fully captured camera, rather
                    // than skipping the rest of this entire interpolation.
                    if let Some(old) = old_scene {
                        let span = (scene.frame - old.frame).max(1) as f64;
                        let minimum =
                            ((self.presentation_floor - old.frame as f64) / span).clamp(0.0, 1.0);
                        let view = |a: f64| {
                            let xy = delta(old.camera, scene.camera, a);
                            [
                                xy[0],
                                xy[1],
                                ((old.camera[2] as f64 - scene.camera[2] as f64) * (1.0 - a))
                                    .round() as i32,
                            ]
                        };
                        alpha = closest_supported_alpha(alpha, minimum, |a| {
                            scene.materials.supports_view(view(a), viewport)
                        });
                        let chosen = view(alpha);
                        shift = [chosen[0], chosen[1]];
                        dz = chosen[2];
                    } else {
                        shift = [0; 2];
                        dz = 0;
                        alpha = 1.0;
                    }
                    reset = true;
                    scene.materials.live_layer_view(
                        [shift[0], shift[1], dz],
                        viewport,
                        settings.textures,
                    )?
                }
                Err(reason) => return Err(reason),
            };
            geometry_ns = geometry_start.elapsed().as_nanos();
            rendered_time = old_scene.map_or(scene.frame as f64, |old| {
                old.frame as f64 + (scene.frame - old.frame) as f64 * alpha
            });
            let zoom = if dz == 0 {
                1.0
            } else {
                scene.camera[2] as f64 / (scene.camera[2] as f64 + dz as f64)
            };
            if let Some(f) = self.camera_frames.as_mut() {
                let _ = writeln!(
                    f,
                    "{},{},{alpha},{},{},{zoom},{},{},{},{}",
                    bus.frames,
                    scene.frame,
                    scene.camera[2],
                    scene.camera[2] as i64 + dz as i64,
                    shift[0],
                    shift[1],
                    scene.camera[0],
                    scene.camera[1]
                );
            }
            let compose_start = std::time::Instant::now();
            let mut objects = if let Some(old) = old_scene {
                scene.objects.interpolate_projected(
                    &old.objects,
                    &old.graphics,
                    &scene.graphics,
                    alpha,
                    shift,
                )
            } else {
                scene.objects.clone()
            }
            .zoomed(zoom);
            if let Some(latest) = &self.current {
                objects.use_hud_from(&latest.objects);
            }
            if let Some((actor, _, _)) = crate::mouse_aim::player(bus) {
                aim_projection = objects.aim_projection(actor, shift, viewport);
            }
            if let Some(f) = self.sprite_presented.as_mut() {
                objects.trace_anchors(bus.frames, shift, f);
            }
            let image = objects.compose_view(
                &scene.graphics,
                &layer,
                &scene.graphics.palette,
                shift,
                viewport,
                zoom,
                settings.hud,
                scene.ground_world.as_ref(),
            );
            compose_ns = compose_start.elapsed().as_nanos();
            Ok::<_, String>((image, alpha < 1.0, reset))
        });
        let elapsed = start.elapsed().as_nanos();
        self.render_ns += elapsed;
        self.geometry_ns += geometry_ns;
        self.compose_ns += compose_ns;
        if result.is_some() {
            if let Some(f) = &mut self.timings {
                let hash = match &result {
                    Some(Ok((image, _, _))) => image.iter().fold(0xcbf29ce484222325u64, |h, p| {
                        p.to_le_bytes()
                            .iter()
                            .fold(h, |h, b| (h ^ *b as u64).wrapping_mul(0x1_0000_01b3))
                    }),
                    _ => 0,
                };
                let _ = writeln!(
                    f,
                    "{},{elapsed},{scene_frame},{},{hash:016x}",
                    bus.frames,
                    bus.frames.saturating_sub(scene_frame)
                );
            }
        }
        match result {
            Some(Ok((image, blended, reset))) => {
                self.aim_projection = aim_projection;
                if reset {
                    // Never move the presentation clock backwards after an
                    // emergency clamp to a fully captured source camera.
                    self.material_clamps += 1;
                    self.event(
                        bus.frames,
                        "material_clamp",
                        &format!("nearest valid presentation time={rendered_time:.6}"),
                    );
                }
                self.rendered += 1;
                self.presentation_floor = self.presentation_floor.max(rendered_time);
                self.presented_scene = Some(scene_frame);
                self.blended += u64::from(blended);
                self.status = "Camera preview";
                if self.output.is_some() {
                    self.last = Some(image.clone());
                }
                Some(image)
            }
            Some(Err(reason)) => {
                self.reject(bus.frames, reason);
                self.original += 1;
                None
            }
            None => {
                self.original += 1;
                None
            }
        }
    }
    pub fn status(&self) -> &'static str {
        self.status
    }
    pub fn finish(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        eprintln!("camera_preview_stages validate_ms={:.3} graphics_ms={:.3} geometry_ms={:.3} compose_ms={:.3}",
            self.validate_ns as f64/1e6,self.graphics_ns as f64/1e6,self.geometry_ns as f64/1e6,self.compose_ns as f64/1e6);
        eprintln!("camera_preview scenes={} rejected={} rendered={} original={} blended={} render_ms={:.3}",self.accepted,self.rejected,self.rendered,self.original,self.blended,self.render_ns as f64/1e6);
        if let Some(path) = &self.output {
            if let Some(image) = &self.last {
                crate::scene_replay::ppm(
                    &path.join("last-preview.ppm"),
                    image,
                    image.len() / 480,
                    480,
                )?;
            }
            std::fs::write(path.join("summary.txt"),format!("accepted_scenes={}\nrejected_scenes={}\npreview_frames={}\noriginal_frames={}\nblended_frames={}\ntransition_resets={}\nmaterial_clamps={}\nrender_ms={}\nlast_fallback={}\n",self.accepted,self.rejected,self.rendered,self.original,self.blended,self.transition_resets,self.material_clamps,self.render_ns as f64/1e6,self.last_reason))?;
        }
        if let Some(f) = &mut self.events {
            f.flush()?;
        }
        if let Some(f) = &mut self.session_events {
            f.flush()?;
        }
        if let Some(path) = &self.session_summary {
            std::fs::write(path,format!("accepted_scenes={}\nrejected_scenes={}\npreview_frames={}\noriginal_frames={}\nmaterial_clamps={}\nlast_fallback={}\n",self.accepted,self.rejected,self.rendered,self.original,self.material_clamps,self.last_reason))?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn guest_pause_clears_scene_without_reporting_failure_but_forced_blank_still_times_out() {
        let mut preview = super::Preview::new(None).unwrap();
        let mut machine = gba_core::Machine::new(Vec::new());
        preview.observe(&machine, super::ENTRY);
        assert!(preview.pending.is_some());
        machine.bus.io[..2].copy_from_slice(&0x0101u16.to_le_bytes());
        machine.bus.frames = 200;
        assert!(preview.frame(&machine.bus).is_none());
        assert!(preview.pending.is_none());
        assert_eq!(preview.rejected, 0);
        assert_eq!(preview.status, "Original menu");
        preview.observe(&machine, super::ENTRY);
        machine.bus.io[..2].copy_from_slice(&0x17c1u16.to_le_bytes());
        machine.bus.frames = 400;
        assert!(preview.frame(&machine.bus).is_none());
        assert_eq!(preview.rejected, 1);
    }
    #[test]
    fn camera_session_log_filters_frame_noise_and_records_recovery() {
        let directory = std::env::temp_dir().join(format!(
            "advance-camera-log-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        {
            let mut preview = super::Preview::new(None).unwrap();
            preview.log_session(&directory).unwrap();
            preview.event(1, "scene", "accepted");
            preview.event(2, "upload_call", "ignored");
            preview.reject(3, "missing, material\nroof".into());
            preview.event(4, "scene", "accepted");
            preview.event(5, "scene", "accepted");
            preview.event(6, "menu", "guest menu display");
            preview.finish().unwrap();
        }
        assert_eq!(std::fs::read_to_string(directory.join("camera-events.csv")).unwrap(),
            "frame,event,reason\n3,fallback,missing  material roof\n4,recovered,accepted\n6,menu,guest menu display\n");
        assert!(
            std::fs::read_to_string(directory.join("camera-summary.txt"))
                .unwrap()
                .contains("rejected_scenes=1")
        );
        std::fs::remove_file(directory.join("camera-events.csv")).unwrap();
        std::fs::remove_file(directory.join("camera-summary.txt")).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }

    use super::*;
    #[test]
    fn missing_edge_clamps_to_nearest_valid_time_without_rewinding() {
        let supports = |a: f64| a <= 0.2 || a >= 0.4;
        let near_left = closest_supported_alpha(0.25, 0.1, supports);
        assert!((near_left - 0.2).abs() < 1e-5);
        assert!(supports(near_left));
        let no_rewind = closest_supported_alpha(0.25, 0.22, supports);
        assert!((no_rewind - 0.4).abs() < 2e-5);
        assert!(supports(no_rewind));
        assert!(no_rewind >= 0.22);
    }
    #[test]
    fn historical_bridge_alpha_keeps_its_world_phase_but_global_fades_are_live() {
        let bridge = [0x44, 0x10, 0, 0x1f, 0, 0];
        let ordinary = [0u8; 6];
        let fade = [0xbf, 0, 0, 0, 12, 0];
        assert_eq!(historical_effects(ordinary, &bridge), ordinary);
        assert_eq!(historical_effects(bridge, &ordinary), bridge);
        assert_eq!(historical_effects(bridge, &fade), fade);
        assert_eq!(historical_effects(bridge, &ordinary), bridge);
    }
    #[test]
    fn presentation_clock_has_no_jump_when_capture_intervals_shorten() {
        fn scene(frame: u64) -> Scene {
            let mut graphics = MemMap::new(Vec::new());
            graphics.io[..2].copy_from_slice(&1u16.to_le_bytes());
            for i in 0..128 {
                graphics.oam[i * 8 + 1] = 2;
            }
            let objects = object_projection::Observer::default()
                .live_snapshot(&graphics)
                .unwrap();
            Scene {
                ground_world: None,
                captured_effects: [0; 6],
                materials: Default::default(),
                objects,
                graphics,
                camera: [frame as i32 * 65536, 0, 100 * 65536],
                frame,
            }
        }
        let mut last = None;
        for frame in 18..=40 {
            let mut history: VecDeque<_> = [10, 14, 19, 22, 24, 28, 34]
                .into_iter()
                .filter(|t| *t <= frame)
                .map(scene)
                .collect();
            let current = history.pop_back();
            let previous = history.pop_back();
            let time = frame - PRESENTATION_DELAY;
            let (b, a, alpha) =
                presentation_pair(&history, previous.as_ref(), current.as_ref(), time as f64)
                    .unwrap();
            let x = b.camera[0] + a.map_or(0, |a| delta(a.camera, b.camera, alpha)[0]);
            assert_eq!(x, time as i32 * 65536, "frame={frame}");
            if let Some(prior) = last {
                assert_eq!(x - prior, 65536);
            }
            last = Some(x);
        }
        let cut = scene(100);
        let empty = VecDeque::new();
        let (_, prior, alpha) = presentation_pair(&empty, None, Some(&cut), 94.0).unwrap();
        assert!(prior.is_none());
        assert_eq!(alpha, 1.0);
    }
    #[test]
    fn both_vblank_upload_returns_publish_a_completed_scene() {
        let mut m = Machine::new(Vec::new());
        m.bus.io[..2].copy_from_slice(&0x0401u16.to_le_bytes());
        for slot in 0..128 {
            m.bus.oam[slot * 8 + 1] = 2;
        }
        let mut preview = Preview::new(None).unwrap();
        for (frame, key) in [(10, 0x08000293), (14, 0x08000327), (18, 0x08000293)] {
            m.bus.frames = frame;
            preview.pending = Some(Pending {
                observer: building_projection::Observer::default(),
                ground: ground_projection::Observer::default(),
                frame: frame - 1,
                camera: [0; 3],
                stages: [1; 4],
                rebuilding: true,
            });
            preview.observe(&m, key);
            assert!(preview.pending.is_none());
            assert_eq!(preview.current.as_ref().unwrap().frame, frame);
        }
        assert_eq!(preview.accepted, 3);
        assert_eq!(preview.rejected, 0);
        let original = crate::render_settings::Settings {
            renderer: crate::render_settings::Renderer::Original,
            ..Default::default()
        };
        assert!(preview.frame_configured(&m.bus, original).is_none());
        assert!(preview.current.is_some());
        assert_eq!(preview.rendered, 0);
        assert!(preview
            .frame_configured(&m.bus, Default::default())
            .is_some());
        assert_eq!(preview.rendered, 1);
    }
    #[test]
    fn palette_upload_and_future_camera_do_not_expire_unchanged_gpu_scene() {
        let mut m = Machine::new(Vec::new());
        m.bus.io[..2].copy_from_slice(&0x0401u16.to_le_bytes());
        m.bus.io[12..14].copy_from_slice(&0x1f00u16.to_le_bytes());
        m.bus.vram[..64].fill(1);
        m.bus.palette[2..4].copy_from_slice(&0x001fu16.to_le_bytes());
        for i in 0..128 {
            m.bus.oam[i * 8 + 1] = 2;
        }
        let mut preview = Preview::new(None).unwrap();
        m.bus.frames = 10;
        preview.pending = Some(Pending {
            observer: Default::default(),
            ground: Default::default(),
            frame: 9,
            camera: [0; 3],
            stages: [1; 4],
            rebuilding: true,
        });
        preview.observe(&m, 0x08000293);
        assert_eq!(preview.frame(&m.bus).unwrap()[0], 0x001f);
        m.bus.frames = 100;
        m.bus.palette[2..4].copy_from_slice(&0x7c00u16.to_le_bytes());
        m.bus.iwram[0x14..0x18].copy_from_slice(&123456i32.to_le_bytes());
        assert_eq!(preview.frame(&m.bus).unwrap()[0], 0x7c00);
        assert_eq!(preview.rejected, 0);
        assert_eq!(
            preview.current.as_ref().unwrap().graphics.iwram[0x14..0x18],
            [0; 4]
        );
        assert_eq!(m.bus.iwram[0x14..0x18], 123456i32.to_le_bytes());
        m.bus.vram[100] = 2;
        m.bus.frames = 101;
        preview.pending = Some(Pending {
            observer: Default::default(),
            ground: Default::default(),
            frame: 101,
            camera: [0; 3],
            stages: [0; 4],
            rebuilding: true,
        });
        m.bus.frames = 180;
        assert!(preview.frame(&m.bus).is_some()); // fresh upload after a long static scene
        preview.pending.as_mut().unwrap().frame = 180; // replacing it cannot renew the hold
        m.bus.frames = 181;
        assert!(preview.frame(&m.bus).is_none());
        assert_eq!(preview.last_reason, "Scene refresh timeout");
    }
    #[test]
    fn stale_presentation_does_not_cancel_the_upload_replacing_it() {
        assert!(!stale_scene(12, false, false));
        assert!(!stale_scene(13, false, true));
        assert!(!stale_scene(80, true, false));
        assert!(stale_scene(13, false, false));
        assert!(capture_can_hold_scene(100, 104));
        assert!(capture_can_hold_scene(100, 112));
        assert!(!capture_can_hold_scene(100, 113));
        assert!(stale_scene(20, false, capture_can_hold_scene(100, 116)));
    }
    #[test]
    fn hook_filter_keeps_upload_stages_and_expires_capture_without_further_hooks() {
        let mut m = Machine::new(Vec::new());
        let mut preview = Preview::new(None).unwrap();
        assert!(preview.interested(ENTRY));
        assert!(UPLOAD_RETURNS.iter().all(|&key| preview.interested(key)));
        assert!(!preview.interested(0x03001000));
        preview.observe(&m, ENTRY);
        for key in [
            0x0800d301, 0x08010545, 0x080127ed, 0x0800fe29, 0x080105c5, 0x08010681, 0x0800e045,
            0x030018a4, 0x03001720, 0x0800db65, 0x0800e249, 0x0800dfc9, 0x080140ab, 0x080145c3,
            0x08014a77,
        ] {
            assert!(preview.interested(key), "lost capture hook {key:08x}");
        }
        m.bus.frames = MAX_CAPTURE_FRAMES + 1;
        assert!(preview.frame(&m.bus).is_none());
        assert!(preview.pending.is_none());
        assert_eq!(preview.last_reason, "Capture timeout");
    }
    #[test]
    fn brief_upload_blank_keeps_the_completed_renderer_but_display_changes_do_not() {
        assert!(display_compatible(0x1741));
        assert!(display_compatible(0x17c1));
        assert!(!display_compatible(0x1743));
        assert!(display_compatible(0x3741));
        assert!(display_compatible(0x7741));
        assert!(!display_compatible(0x9741));
    }
    #[test]
    fn camera_transitions_and_static_reuse_require_matching_state() {
        fn scene(frame: u64) -> Scene {
            let bus = MemMap::new(Vec::new());
            let objects = object_projection::Observer::default()
                .live_snapshot(&{
                    let mut b = MemMap::new(Vec::new());
                    b.io[..2].copy_from_slice(&1u16.to_le_bytes());
                    for i in 0..128 {
                        b.oam[i * 8 + 1] = 2;
                    }
                    b
                })
                .unwrap();
            Scene {
                ground_world: None,
                captured_effects: [0; 6],
                materials: building_material::Capture::default(),
                objects,
                graphics: bus,
                camera: [0; 3],
                frame,
            }
        }
        let mut old = scene(10);
        let mut new = scene(14);
        assert!(compatible(&old, &new));
        new.frame = 15;
        assert!(compatible(&old, &new));
        new.frame = 19;
        assert!(!compatible(&old, &new));
        new.frame = 14;
        new.camera[0] = 100 * 65536;
        assert!(!compatible(&old, &new));
        new.camera[0] = 0;
        new.camera[2] = 1;
        assert!(!compatible(&old, &new));
        new.camera[2] = 0;
        new.graphics.iwram[0x60] = 1;
        assert!(!compatible(&old, &new));
        let mut bus = MemMap::new(Vec::new());
        assert!(unchanged_graphics(&old, &bus));
        bus.io[0x50] = 0x40;
        assert!(unchanged_graphics(&old, &bus));
        bus.vram[0] = 1;
        assert!(!unchanged_graphics(&old, &bus));
        bus.vram[0] = 0;
        bus.oam[0] = 1;
        assert!(!unchanged_graphics(&old, &bus));
        bus.oam[0] = 0;
        bus.palette[0] = 1;
        assert!(!unchanged_graphics(&old, &bus));
        bus.palette[0] = 0;
        bus.iwram[0x14] = 1;
        assert!(unchanged_graphics(&old, &bus));
        bus.iwram[0x60] = 1;
        assert!(unchanged_graphics(&old, &bus));
        bus.io[0x28] = 1;
        assert!(!unchanged_graphics(&old, &bus));
        old.camera[2] = 164 * 65536;
        new.camera[2] = 10957616;
        for (scene, values) in [
            (&mut old, [47760i32, 47760, 89544, 89544]),
            (&mut new, [47040i32, 47040, 91182, 91182]),
        ] {
            for (i, v) in values.into_iter().enumerate() {
                scene.graphics.iwram[0x60 + i * 4..0x64 + i * 4].copy_from_slice(&v.to_le_bytes());
            }
        }
        assert!(compatible(&old, &new));
        new.camera[2] = 220 * 65536;
        assert!(!compatible(&old, &new));
        new.camera[2] = 10957616;
        new.graphics.iwram[0x60..0x64].copy_from_slice(&24000i32.to_le_bytes());
        assert!(!compatible(&old, &new));
    }
    #[test]
    fn temporal_camera_moves_on_both_axes_without_changing_endpoints() {
        let a = [65536, 3 * 65536, 0];
        let b = [3 * 65536, 65536, 0];
        assert_eq!(delta(a, b, 0.0), [-2 * 65536, 2 * 65536]);
        assert_eq!(delta(a, b, 0.5), [-65536, 65536]);
        assert_eq!(delta(a, b, 1.0), [0, 0]);
    }
}
