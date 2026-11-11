// Shared modules also contain desktop hooks and offline probes.
#![allow(dead_code)]

use gba_core::capi::RtApi;
use gba_core::Machine;
use std::{cell::RefCell, sync::Arc};
use std::{collections::HashMap, ffi::c_void};

#[path = "../../../src/building_material.rs"]
mod building_material;
#[path = "../../../src/building_projection.rs"]
mod building_projection;
#[path = "../../../src/camera_preview.rs"]
mod camera_preview;
#[path = "../../../src/color_effects.rs"]
mod color_effects;
#[path = "../../../src/ground_projection.rs"]
mod ground_projection;
#[path = "../../../src/hud_layout.rs"]
mod hud_layout;
#[path = "../../../src/native_host.rs"]
mod native_host;
#[path = "../../../src/object_projection.rs"]
mod object_projection;
#[path = "../../../src/rect_windows.rs"]
mod rect_windows;
#[path = "../../../src/render_settings.rs"]
mod render_settings;
#[path = "../../../src/scene_replay.rs"]
mod scene_replay;
#[path = "../../../src/viewport.rs"]
mod viewport;

type BlockFn = extern "C" fn(*const RtApi, *mut c_void) -> u32;
thread_local! {
    static RESCUE: RefCell<HashMap<u32, Vec<BlockFn>>> = RefCell::new(HashMap::new());
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "host")]
extern "C" {
    fn now_ms() -> f64;
}

#[cfg(target_arch = "wasm32")]
struct BrowserInstant(f64);
#[cfg(target_arch = "wasm32")]
impl BrowserInstant {
    fn now() -> Self {
        Self(unsafe { now_ms() })
    }
    fn elapsed(self) -> std::time::Duration {
        std::time::Duration::from_secs_f64((unsafe { now_ms() } - self.0).max(0.0) / 1000.0)
    }
}

#[path = "../../../src/camera_mode.rs"]
mod camera_mode;
#[path = "../../../src/cash_protection.rs"]
mod cash_protection;
#[path = "../../../src/control_context.rs"]
mod control_context;
#[path = "../../../src/drop_lifetime.rs"]
mod drop_lifetime;
#[path = "../../../src/enhanced_audio.rs"]
mod enhanced_audio;
#[path = "../../../src/language_font.rs"]
mod language_font;
#[path = "../../../src/localization.rs"]
mod localization;
#[path = "../../../src/menu_text.rs"]
mod menu_text;
#[path = "../../../src/mouse_aim.rs"]
mod mouse_aim;
#[path = "../../../src/pickup_probe.rs"]
mod pickup_probe;
#[path = "../../../src/polyphonic_audio.rs"]
mod polyphonic_audio;

#[path = "../../../src/rom_profile.rs"]
mod rom_profile;

const ROM_SIZE: usize = 16 * 1024 * 1024;

#[derive(Default)]
struct Runtime {
    upload: Vec<u8>,
    machine: Option<Machine>,
    base_rom: Arc<Vec<u8>>,
    language: Option<localization::Language>,
    error: String,
    audio: Option<enhanced_audio::Output>,
    output: Vec<i16>,
    drops: Option<drop_lifetime::Policy>,
    preview: Option<camera_preview::Preview>,
    render: render_settings::Settings,
    rendered: Option<Vec<u16>>,
}

thread_local! {
    static RUNTIME: RefCell<Runtime> = RefCell::new(Runtime::default());
}

// The browser writes into this buffer before calling start or load_save.
#[no_mangle]
pub extern "C" fn upload(len: usize) -> *mut u8 {
    if !matches!(len, ROM_SIZE | 512 | 8192) {
        return std::ptr::null_mut();
    }
    RUNTIME.with_borrow_mut(|runtime| {
        runtime.upload.resize(len, 0);
        runtime.upload.as_mut_ptr()
    })
}

#[no_mangle]
pub extern "C" fn start() -> bool {
    RUNTIME.with_borrow_mut(|runtime| {
        if rom_profile::verify(&runtime.upload).is_err() {
            runtime.upload = Vec::new();
            return false;
        }
        let machine = Machine::new(std::mem::take(&mut runtime.upload));
        runtime.base_rom = machine.bus.rom.shared();
        runtime.machine = Some(machine);
        runtime.language = None;
        runtime.audio = None;
        runtime.drops = None;
        runtime.preview = camera_preview::Preview::new(None).ok();
        runtime.rendered = None;
        true
    })
}

#[no_mangle]
pub extern "C" fn upload_language(len: usize) -> *mut u8 {
    if len == 0 || len > 4 * 1024 * 1024 {
        return std::ptr::null_mut();
    }
    RUNTIME.with_borrow_mut(|runtime| {
        runtime.upload.resize(len, 0);
        runtime.upload.as_mut_ptr()
    })
}

#[no_mangle]
pub extern "C" fn set_language(original: bool) -> bool {
    RUNTIME.with_borrow_mut(|r| {
        let Some(machine) = &mut r.machine else {
            return false;
        };
        let bytes = (!original).then(|| std::mem::take(&mut r.upload));
        match localization::change_live(machine, &r.base_rom, &mut r.language, bytes) {
            Ok(()) => {
                r.error.clear();
                true
            }
            Err(error) => {
                r.error = error.to_string();
                false
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn error_ptr() -> *const u8 {
    RUNTIME.with_borrow(|r| r.error.as_ptr())
}

#[no_mangle]
pub extern "C" fn error_len() -> usize {
    RUNTIME.with_borrow(|r| r.error.len())
}

#[no_mangle]
pub extern "C" fn configure_audio(enabled: bool) {
    RUNTIME.with_borrow_mut(|r| {
        let Some(machine) = &mut r.machine else {
            return;
        };
        if r.audio.is_some() != enabled {
            r.audio = enabled.then(enhanced_audio::Output::new);
            machine.bus.tap_channels = enabled;
            // At boot both hardware FIFOs are empty.
            machine.bus.pcm_overlay = (enabled && machine.bus.frames == 0).then(Default::default);
            machine.bus.fifo_tap.iter_mut().for_each(Vec::clear);
            machine.bus.psg_tap.clear();
        }
    });
}

#[no_mangle]
pub extern "C" fn configure_drops(seconds: u32) {
    RUNTIME.with_borrow_mut(|r| {
        r.drops = (1..=300)
            .contains(&seconds)
            .then(|| drop_lifetime::Policy::new(seconds));
    });
}

#[no_mangle]
pub extern "C" fn configure_graphics(enhanced: bool, wide: bool, perspective: bool, hud: u8) {
    RUNTIME.with_borrow_mut(|r| {
        use render_settings::{Renderer, TextureMapping};
        r.render.renderer = if enhanced {
            Renderer::Enhanced
        } else {
            Renderer::Original
        };
        r.render.aspect = if wide {
            viewport::Aspect::Wide
        } else {
            viewport::Aspect::Original
        };
        r.render.textures = if perspective {
            TextureMapping::Perspective
        } else {
            TextureMapping::Affine
        };
        let size = hud_layout::Size::parse(&hud.to_string()).unwrap_or_default();
        r.render.hud = hud_layout::Settings {
            radar: size,
            status: size,
            messages: size,
        };
    });
}

#[no_mangle]
pub extern "C" fn control_context() -> u32 {
    RUNTIME.with_borrow(|r| {
        use control_context::Context;
        match r.machine.as_ref().map(|m| control_context::detect(&m.bus)) {
            Some(Context::OnFoot) => 1,
            Some(Context::Vehicle) => 2,
            _ => 0,
        }
    })
}

#[no_mangle]
pub extern "C" fn cycle_camera() -> i32 {
    RUNTIME.with_borrow_mut(|r| {
        r.machine
            .as_mut()
            .and_then(|m| camera_mode::cycle(&mut m.bus))
            .map_or(-1, i32::from)
    })
}

#[no_mangle]
pub extern "C" fn load_save() -> bool {
    RUNTIME.with_borrow_mut(|runtime| {
        let Some(machine) = &mut runtime.machine else {
            return false;
        };
        if !matches!(runtime.upload.len(), 512 | 8192) {
            return false;
        }
        machine.bus.load_save_data(&runtime.upload);
        runtime.upload.clear();
        true
    })
}

#[no_mangle]
pub extern "C" fn frame(keys: u16, aiming: bool, x: f64, y: f64) -> bool {
    RUNTIME.with_borrow_mut(|runtime| {
        let Runtime {
            machine,
            language,
            audio,
            output,
            drops,
            preview,
            render,
            rendered,
            ..
        } = runtime;
        let Some(machine) = machine else {
            return false;
        };
        machine.bus.keys = keys & 0x3ff;
        machine.bus.audio_buf.clear();
        machine.bus.frame_ready = false;
        let angle = aiming
            .then(|| {
                let (actor, _, world) = mouse_aim::player(&machine.bus)?;
                let projection = preview
                    .as_ref()
                    .and_then(camera_preview::Preview::aim_projection)
                    .or_else(|| mouse_aim::Projection::original(&machine.bus))?;
                projection.angle(
                    actor,
                    world,
                    [
                        x * f64::from(projection.size[0]),
                        y * f64::from(projection.size[1]),
                    ],
                )
            })
            .flatten();
        mouse_aim::set(&machine.bus, angle);
        mouse_aim::present(&machine.bus, angle.is_some());
        if let Some(policy) = drops.as_mut() {
            policy.update(machine);
        }
        for _ in 0..1_000_000 {
            #[cfg(target_arch = "wasm32")]
            let mut aim_load = None;
            machine.step_with(|m| {
                let key = m.cpu.regs[15] | m.cpu.thumb() as u32;
                if key == 0x0802de67 {
                    if let Some(language) = language.as_mut() {
                        language.observe(m, key);
                    }
                }
                if drop_lifetime::Policy::interested(key) {
                    if let Some(policy) = drops.as_mut() {
                        policy.observe(m, key);
                    }
                }
                #[cfg(target_arch = "wasm32")]
                if matches!(key, 0x0803242f | 0x0803244f | 0x08032497 | 0x08034d73) {
                    polyphonic_audio::observe_interpreted(m, key);
                }
                if render.renderer == render_settings::Renderer::Enhanced {
                    if let Some(preview) = preview.as_mut().filter(|p| p.interested(key)) {
                        preview.observe(m, key);
                    }
                }
                #[cfg(target_arch = "wasm32")]
                if angle.is_some() && m.cpu.thumb() {
                    aim_load = mouse_aim::interpreted_load(m);
                }
            });
            #[cfg(target_arch = "wasm32")]
            if let Some((register, value)) = aim_load {
                machine.cpu.regs[register] = value;
            }
            if machine.bus.frame_ready {
                *rendered = preview
                    .as_mut()
                    .and_then(|p| p.frame_configured(&machine.bus, *render));
                output.clear();
                if let Some(audio) = audio {
                    output.extend_from_slice(audio.frame(&mut machine.bus));
                } else {
                    output.extend_from_slice(&machine.bus.audio_buf);
                }
                return true;
            }
        }
        false
    })
}

// Read these views immediately, before another call mutates the machine or
// WebAssembly grows its memory. Audio length counts interleaved i16 samples.
#[no_mangle]
pub extern "C" fn pixels() -> *const u16 {
    RUNTIME.with_borrow(|r| {
        if let Some(rendered) = &r.rendered {
            return rendered.as_ptr();
        }
        r.machine
            .as_ref()
            .map_or(std::ptr::null(), |m| m.bus.framebuffer.as_ptr())
    })
}

#[no_mangle]
pub extern "C" fn video_width() -> usize {
    RUNTIME.with_borrow(|r| r.rendered.as_ref().map_or(240, |image| image.len() / 480))
}

#[no_mangle]
pub extern "C" fn video_height() -> usize {
    RUNTIME.with_borrow(|r| if r.rendered.is_some() { 480 } else { 160 })
}

#[no_mangle]
pub extern "C" fn audio() -> *const i16 {
    RUNTIME.with_borrow(|r| r.output.as_ptr())
}

#[no_mangle]
pub extern "C" fn audio_len() -> usize {
    RUNTIME.with_borrow(|r| r.output.len())
}

#[no_mangle]
pub extern "C" fn save() -> *const u8 {
    RUNTIME.with_borrow(|r| {
        r.machine
            .as_ref()
            .and_then(|m| m.bus.save_data())
            .map_or(std::ptr::null(), |s| s.as_ptr())
    })
}

#[no_mangle]
pub extern "C" fn save_len() -> usize {
    RUNTIME.with_borrow(|r| {
        r.machine
            .as_ref()
            .and_then(|m| m.bus.save_data())
            .map_or(0, |s| s.len())
    })
}
