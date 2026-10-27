#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! AdvanceLiberty: Enhanced runtime and deterministic replay tools.
//! Reuses upstream CPU/hardware semantics; no game code or assets are included.
use gba_core::{
    capi::{RcgBlock, RtApi, RT_API},
    Machine, Mode,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    ffi::c_void,
    fs,
};
mod block_check;
mod building_material;
mod building_projection;
mod camera_mode;
mod camera_preview;
mod camera_trace;
mod cash_protection;
#[cfg(feature = "desktop")]
mod cheats;
mod color_effects;
mod control_context;
#[cfg(feature = "desktop")]
mod controls;
#[cfg(any(feature = "desktop", test))]
mod coverage;
#[cfg(feature = "desktop")]
mod desktop;
mod drop_lifetime;
mod audio_probe;
mod enhanced_audio;
mod polyphonic_audio;
mod frame_capture;
#[cfg(feature = "desktop")]
mod game_menu;
#[cfg(feature = "desktop")]
mod gamepad;
mod ground_projection;
mod hud_layout;
mod language_font;
#[cfg(feature = "desktop")]
mod launcher;
mod localization;
#[cfg(feature = "desktop")]
mod menu_audio;
mod menu_text;
mod mouse_aim;
mod native_audit;
mod native_host;
mod object_projection;
#[cfg(feature = "desktop")]
mod options;
mod pickup_probe;
mod rect_windows;
mod render_settings;
mod replay;
#[cfg(feature = "desktop")]
mod replay_panel;
mod rom_profile;
mod save;
mod scene_replay;
mod session;
mod viewport;
const FRAME_PROFILE_HEADER: &str = "frame,total_ns,native_ns,render_ns,audio_ns,accepted_scenes,audio_samples,fifo_underruns_a,fifo_underruns_b,fifo_refills_a,fifo_refills_b,queued_audio_bytes,audio_underruns";
type BlockFn = extern "C" fn(*const RtApi, *mut c_void) -> u32;
#[derive(Default)]
struct Counters {
    calls: u64,
    internal: u64,
    rescued: u64,
    halt_wake: u64,
    guard_checks: u64,
    guard_failures: u64,
    rejected: bool,
    retry: bool,
    failed: HashMap<u32, u64>,
}
thread_local! { static COUNTERS: RefCell<Counters> = RefCell::new(Counters::default()); }
thread_local! { static VIEWING_REPLAY: Cell<bool> = const { Cell::new(false) }; }
thread_local! { static AUDIT: RefCell<native_audit::Audit> = RefCell::new(native_audit::Audit::default()); }
thread_local! { static RESCUE: RefCell<HashMap<u32, Vec<BlockFn>>> = RefCell::new(HashMap::new()); }
thread_local! { static ACTIVE_API: Cell<*const RtApi> = const { Cell::new(std::ptr::null()) }; }
struct ApiScope<'a> {
    previous: *const RtApi,
    _api: &'a RtApi,
}
impl<'a> ApiScope<'a> {
    fn enter(api: &'a RtApi) -> Self {
        Self {
            previous: ACTIVE_API.with(|p| p.replace(api)),
            _api: api,
        }
    }
}
impl Drop for ApiScope<'_> {
    fn drop(&mut self) {
        ACTIVE_API.with(|p| p.set(self.previous));
    }
}
fn audit(reason: &'static str, m: &Machine) {
    if !VIEWING_REPLAY.with(Cell::get) {
        AUDIT.with(|a| a.borrow_mut().record(reason, m));
    }
}
extern "C" fn guard(m: *mut c_void, addr: u32, len: u32, hash: u64) -> u32 {
    let result = (RT_API.guard)(m, addr, len, hash);
    COUNTERS.with(|c| {
        let mut c = c.borrow_mut();
        c.guard_checks += 1;
        if result == 0 {
            c.guard_failures += 1;
            c.rejected = true;
            *c.failed.entry(addr).or_default() += 1;
        }
    });
    result
}
extern "C" fn interp(m: *mut c_void) -> u32 {
    let skip = COUNTERS.with(|c| {
        let c = c.borrow();
        c.retry && c.rejected
    });
    if skip {
        // The emitter's RAM entry guard runs before any guest instruction. On
        // rejection it restores PC and calls interp_one. Defer that one step so
        // the dispatcher can try another guarded version of this same address.
        let m = unsafe { &*(m as *const Machine) };
        m.cpu.regs[15] | m.cpu.thumb() as u32
    } else {
        let key = unsafe { &*(m as *const Machine) };
        let key = key.cpu.regs[15] | key.cpu.thumb() as u32;
        let candidates = RESCUE.with(|r| r.borrow().get(&key).cloned().unwrap_or_default());
        // Only a marked, generated supplement may service an internal exit.
        // Its bodies execute exactly one native instruction; the only call back
        // to interp is a RAM entry rejection, which is deferred above.
        let api = native_host::Api::new(RtApi {
            guard,
            interp_one: interp,
            ..RT_API
        });
        let active_api = ACTIVE_API.with(|p| p.get());
        let api_ptr = if active_api.is_null() {
            &api.base as *const RtApi
        } else {
            active_api
        };
        for candidate in candidates {
            let old = COUNTERS.with(|c| {
                let mut c = c.borrow_mut();
                let old = (c.retry, c.rejected);
                c.retry = true;
                c.rejected = false;
                old
            });
            let result = candidate(api_ptr, m);
            let rejected = COUNTERS.with(|c| {
                let mut c = c.borrow_mut();
                let rejected = c.rejected;
                c.retry = old.0;
                c.rejected = old.1;
                if !rejected {
                    c.rescued += 1;
                }
                rejected
            });
            if !rejected {
                return result;
            }
        }
        COUNTERS.with(|c| c.borrow_mut().internal += 1);
        audit("internal", unsafe { &*(m as *const Machine) });
        (RT_API.interp_one)(m)
    }
}
fn hash(pixels: &[u16]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for p in pixels {
        for b in p.to_le_bytes() {
            // Match upstream fb_hash, including its nonstandard FNV multiplier.
            h = (h ^ b as u64).wrapping_mul(0x1_0000_01b3);
        }
    }
    h
}
fn input(path: &str) -> Result<Vec<(u64, u16)>, Box<dyn std::error::Error>> {
    let s = fs::read_to_string(path)?;
    let mut lines = s.lines();
    if lines.next() != Some("gba-input v1") {
        return Err("Invalid input header".into());
    }
    let mut points = Vec::new();
    for line in lines.filter(|l| {
        !l.trim().is_empty() && !l.starts_with("# language ") && !l.starts_with("# camera ")
    }) {
        let (f, k) = line.split_once(' ').ok_or("Invalid input line")?;
        let frame = f.parse()?;
        let mask = u16::from_str_radix(k, 16)?;
        if mask > 0x3ff || points.last().is_some_and(|&(last, _)| last > frame) {
            return Err("Invalid input ordering or mask".into());
        }
        points.push((frame, mask));
    }
    Ok(points)
}
#[cfg(feature = "desktop")]
fn coverage(native_dispatches: u64, misses: u64, rejected: u64) -> coverage::Coverage {
    COUNTERS.with(|c| {
        let c = c.borrow();
        coverage::Coverage {
            native_dispatches: native_dispatches + c.rescued,
            interpreted_steps: misses + rejected + c.internal + c.halt_wake,
        }
    })
}
#[cfg(all(test, feature = "desktop"))]
thread_local! { static REPLAY_TEST_DRIVER: RefCell<Option<Box<dyn FnMut(&Machine, &mut desktop::Desktop)>>> = RefCell::new(None); }
#[cfg(all(test, feature = "desktop"))]
thread_local! { static REPLAY_TEST_FAST: Cell<bool> = const { Cell::new(false) }; }
fn run() -> Result<(), Box<dyn std::error::Error>> {
    run_with_args(std::env::args().skip(1).collect())
}
fn run_with_args(mut args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let polyphonic = if let Some(index) = args.iter().position(|s| s == "--polyphonic-audio") {
        args.remove(index);
        true
    } else {
        false
    };
    let enhanced_sound = if let Some(index) = args.iter().position(|s| s == "--enhanced-audio") {
        args.remove(index);
        true
    } else {
        false
    };
    let _original_menus = if let Some(index) = args.iter().position(|s| s == "--original-menus") {
        args.remove(index);
        true
    } else {
        false
    };
    let language_path = if let Some(index) = args.iter().position(|s| s == "--language-pack") {
        if index + 1 >= args.len() {
            return Err("--language-pack requires FILE.json".into());
        }
        let path = std::path::PathBuf::from(args.remove(index + 1));
        args.remove(index);
        Some(path)
    } else {
        None
    };
    let text_trace = if let Some(index) = args.iter().position(|s| s == "--text-trace") {
        if index + 1 >= args.len() {
            return Err("--text-trace requires NEW_FILE.csv".into());
        }
        let path = std::path::PathBuf::from(args.remove(index + 1));
        args.remove(index);
        Some(path)
    } else {
        None
    };
    let aim_path = if let Some(i) = args.iter().position(|s| s == "--aim-input") {
        if i + 1 >= args.len() {
            return Err("--aim-input requires FILE.gba-aim".into());
        }
        let path = std::path::PathBuf::from(args.remove(i + 1));
        args.remove(i);
        Some(path)
    } else {
        None
    };
    let mut control_trace = if let Some(i) = args.iter().position(|s| s == "--control-state") {
        if i + 1 >= args.len() {
            return Err("--control-state requires NEW_FILE.csv".into());
        }
        let path = args.remove(i + 1);
        args.remove(i);
        use std::io::Write;
        let mut trace = std::io::BufWriter::new(fs::File::create_new(path)?);
        writeln!(trace, "frame,context,actor,display,gate")?;
        Some(trace)
    } else {
        None
    };
    #[cfg(feature = "desktop")]
    let override_renderer = args.iter().any(|s| s == "--original-renderer");
    #[cfg(feature = "desktop")]
    let override_textures = args.iter().any(|s| s == "--affine-textures");
    let mut render_settings = render_settings::Settings::default();
    if let Some(index) = args.iter().position(|s| s == "--hud-scale") {
        let size = hud_layout::Size::parse(
            args.get(index + 1)
                .ok_or("--hud-scale requires 50,65,75,85 or 100")?,
        )?;
        render_settings.hud = hud_layout::Settings {
            radar: size,
            status: size,
            messages: size,
        };
        args.drain(index..=index + 1);
    }
    if let Some(i) = args.iter().position(|s| s == "--original-renderer") {
        args.remove(i);
        render_settings.renderer = render_settings::Renderer::Original;
    }
    if let Some(i) = args.iter().position(|s| s == "--affine-textures") {
        args.remove(i);
        render_settings.textures = render_settings::TextureMapping::Affine;
    }
    let override_aspect = if let Some(i) = args.iter().position(|s| s == "--widescreen") {
        args.remove(i);
        render_settings.aspect = viewport::Aspect::Wide;
        true
    } else {
        false
    };
    let preview_enabled = if let Some(i) = args.iter().position(|s| s == "--camera-preview") {
        args.remove(i);
        true
    } else {
        false
    };
    let preview_output = if let Some(i) = args.iter().position(|s| s == "--preview-output") {
        if i + 1 >= args.len() {
            return Err("--preview-output requires NEW_DIRECTORY".into());
        }
        let path = std::path::PathBuf::from(args.remove(i + 1));
        args.remove(i);
        Some(path)
    } else {
        None
    };
    if preview_output.is_some() && !preview_enabled {
        return Err("--preview-output requires --camera-preview".into());
    }
    let scene_offsets = if let Some(i) = args.iter().position(|s| s == "--scene-camera-offsets") {
        args.remove(i);
        true
    } else {
        false
    };
    let scene_options = if let Some(index) = args.iter().position(|s| s == "--scene-replay") {
        if index + 3 >= args.len() {
            return Err("--scene-replay requires NEW_DIRECTORY START_FRAME TRIAL_COUNT".into());
        }
        let path = std::path::PathBuf::from(args.remove(index + 1));
        let start = args.remove(index + 1).parse::<u64>()?;
        let count = args.remove(index + 1).parse::<u32>()?;
        if count == 0 {
            return Err("Scene trial count must be positive".into());
        }
        args.remove(index);
        Some((path, start, count))
    } else {
        None
    };
    if scene_offsets && scene_options.is_none() {
        return Err("--scene-camera-offsets requires --scene-replay".into());
    }
    let camera_options = if let Some(index) = args.iter().position(|s| s == "--camera-trace") {
        if index + 3 >= args.len() {
            return Err("--camera-trace requires NEW_DIRECTORY START_FRAME END_FRAME".into());
        }
        let path = std::path::PathBuf::from(args.remove(index + 1));
        let start = args.remove(index + 1).parse::<u64>()?;
        let end = args.remove(index + 1).parse::<u64>()?;
        if end < start {
            return Err("Camera trace frame range is reversed".into());
        }
        args.remove(index);
        Some((path, start, end))
    } else {
        None
    };
    let drop_lifetime_requested =
        if let Some(index) = args.iter().position(|s| s == "--drop-lifetime") {
            if index + 1 >= args.len() {
                return Err("--drop-lifetime requires seconds (1..300)".into());
            }
            let value = args.remove(index + 1).parse::<u32>()?;
            args.remove(index);
            if !(1..=300).contains(&value) {
                return Err("Drop lifetime must be 1..300 seconds".into());
            }
            Some(value)
        } else {
            None
        };
    // Research switch: replay an unchanged lead-in before enabling the policy.
    let drop_start_requested =
        if let Some(index) = args.iter().position(|s| s == "--drop-lifetime-after-frame") {
            if index + 1 >= args.len() {
                return Err("--drop-lifetime-after-frame requires FRAME".into());
            }
            let value = args.remove(index + 1).parse::<u64>()?;
            args.remove(index);
            Some(value)
        } else {
            None
        };
    let preserve_cash_requested =
        if let Some(index) = args.iter().position(|s| s == "--preserve-cash") {
            args.remove(index);
            true
        } else {
            false
        };
    let mut cash_retained = 0u64;
    let pickup_probe_path = if let Some(index) = args.iter().position(|s| s == "--pickup-probe") {
        if index + 1 >= args.len() {
            return Err("--pickup-probe requires NEW_DIRECTORY".into());
        }
        let path = std::path::PathBuf::from(args.remove(index + 1));
        args.remove(index);
        Some(path)
    } else {
        None
    };
    let mut state_trace = if let Some(index) = args.iter().position(|s| s == "--state-trace") {
        use std::io::Write;
        if index + 1 >= args.len() {
            return Err("--state-trace requires NEW_CSV".into());
        }
        let path = args.remove(index + 1);
        args.remove(index);
        let mut file = std::io::BufWriter::new(fs::File::create_new(path)?);
        writeln!(file, "frame,clock,pc,cpsr,ie,if,ime,ewram,iwram,regs")?;
        Some(file)
    } else {
        None
    };
    let check_options = if let Some(index) = args.iter().position(|s| s == "--check-blocks") {
        if index + 3 >= args.len() {
            return Err("--check-blocks requires NEW_REPORT START_FRAME COUNT".into());
        }
        let path = args.remove(index + 1);
        let start = args.remove(index + 1).parse::<u64>()?;
        let count = args.remove(index + 1).parse::<u64>()?;
        if count == 0 {
            return Err("Block comparison count must be positive".into());
        }
        args.remove(index);
        Some((path, start, count))
    } else {
        None
    };
    let capture_options = if let Some(index) = args.iter().position(|s| s == "--capture-frames") {
        if index + 2 >= args.len() {
            return Err("--capture-frames requires NEW_DIRECTORY FRAME,FRAME,...".into());
        }
        let directory = std::path::PathBuf::from(args.remove(index + 1));
        let frames = args
            .remove(index + 1)
            .split(',')
            .map(str::parse::<u64>)
            .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
        args.remove(index);
        if frames.is_empty() || frames.contains(&0) {
            return Err("Capture frames must be positive".into());
        }
        Some((directory, frames))
    } else {
        None
    };
    // Offline diagnostics reuse the exact gameplay replay format.
    let offline_play = if let Some(index) = args.iter().position(|s| s == "--replay-play") {
        let path = args
            .get(index + 1)
            .ok_or("--replay-play requires a replay directory")?
            .clone();
        args.drain(index..=index + 1);
        Some(std::path::PathBuf::from(path))
    } else {
        None
    };
    let offline_record = if let Some(index) = args.iter().position(|s| s == "--replay-record") {
        let directory = args
            .get(index + 1)
            .ok_or("--replay-record requires DIRECTORY START_FRAME")?
            .clone();
        let start = args
            .get(index + 2)
            .ok_or("--replay-record requires DIRECTORY START_FRAME")?
            .parse::<u64>()?;
        args.drain(index..=index + 2);
        Some((std::path::PathBuf::from(directory), start))
    } else {
        None
    };
    if offline_play.is_some() && offline_record.is_some() {
        return Err("Choose recording or playing an offline replay".into());
    }
    let mut replay_directory = if let Some(index) = args.iter().position(|a| a == "--replays") {
        if index + 1 >= args.len() {
            return Err("--replays requires a directory".into());
        }
        let path = std::path::PathBuf::from(args.remove(index + 1));
        args.remove(index);
        path
    } else {
        std::env::current_dir()?.join("replays")
    };
    if let Some((directory, _)) = &offline_record {
        replay_directory = directory.clone();
    }
    let session_path = if let Some(index) = args.iter().position(|s| s == "--record-session") {
        if index + 1 >= args.len() {
            return Err("--record-session requires a new directory".into());
        }
        let path = std::path::PathBuf::from(args.remove(index + 1));
        args.remove(index);
        if path.exists() {
            return Err("Session directory already exists; choose a new directory".into());
        }
        Some(path)
    } else {
        None
    };
    let force_interpreter = if let Some(index) = args.iter().position(|s| s == "--interpreter") {
        args.remove(index);
        true
    } else {
        false
    };
    let audio_hashes = if let Some(index) = args.iter().position(|s| s == "--audio-hashes") {
        args.remove(index);
        true
    } else {
        false
    };
    let mut frame_profile = if let Some(index) = args.iter().position(|s| s == "--frame-profile") {
        let path = args
            .get(index + 1)
            .ok_or("--frame-profile requires a new CSV path")?
            .clone();
        args.drain(index..=index + 1);
        let mut file = std::io::BufWriter::new(fs::File::create_new(path)?);
        use std::io::Write;
        writeln!(file, "{FRAME_PROFILE_HEADER}")?;
        Some(file)
    } else {
        None
    };
    if let Some(index) = args.iter().position(|s| s == "--roof-trace") {
        if index + 3 >= args.len() {
            return Err("--roof-trace requires NEW_CSV START_FRAME END_FRAME".into());
        }
        let start = args[index + 2].parse()?;
        let end = args[index + 3].parse()?;
        if end < start {
            return Err("Reversed roof trace range".into());
        }
        building_material::open_roof_trace(std::path::Path::new(&args[index + 1]), start, end)?;
        args.drain(index..=index + 3);
    }
    let sound_probe = if let Some(index) = args.iter().position(|s| s == "--sound-probe") {
        let path = args
            .get(index + 1)
            .ok_or("--sound-probe requires NEW_DIRECTORY")?
            .clone();
        args.drain(index..=index + 1);
        Some(std::path::PathBuf::from(path))
    } else {
        None
    };
    let mut audio_output = if let Some(index) = args.iter().position(|s| s == "--audio-out") {
        if index + 1 >= args.len() {
            return Err("--audio-out requires a new PCM file path".into());
        }
        let path = args.remove(index + 1);
        args.remove(index);
        Some(std::io::BufWriter::new(fs::File::create_new(path)?))
    } else {
        None
    };
    let strict_native = if let Some(index) = args.iter().position(|s| s == "--require-native") {
        args.remove(index);
        true
    } else {
        false
    };
    let audit_path = if let Some(index) = args.iter().position(|s| s == "--audit") {
        if session_path.is_some() {
            return Err("--record-session already includes an audit; omit --audit".into());
        }
        if index + 1 >= args.len() {
            return Err("--audit requires a new CSV path".into());
        }
        let path = std::path::PathBuf::from(args.remove(index + 1));
        args.remove(index);
        if path.exists() {
            return Err("Audit output already exists; choose a new path".into());
        }
        Some(path)
    } else {
        session_path.as_ref().map(|path| path.join("fallbacks.csv"))
    };
    if strict_native && audit_path.is_none() {
        return Err("--require-native requires --audit PATH".into());
    }
    AUDIT.with(|a| {
        let mut a = a.borrow_mut();
        *a = native_audit::Audit::default();
        a.enabled = audit_path.is_some();
    });
    let controls_path = if let Some(index) = args.iter().position(|s| s == "--controls") {
        if index + 1 >= args.len() {
            return Err("--controls requires a path".into());
        }
        let path = std::path::PathBuf::from(args.remove(index + 1));
        args.remove(index);
        Some(path)
    } else {
        None
    };
    #[cfg(not(feature = "desktop"))]
    if controls_path.is_some() {
        return Err("--controls requires --features desktop".into());
    }
    let save_path = if let Some(index) = args.iter().position(|s| s == "--save") {
        if index + 1 >= args.len() {
            return Err("--save requires a path".into());
        }
        let path = std::path::PathBuf::from(args.remove(index + 1));
        args.remove(index);
        Some(path)
    } else {
        None
    };
    if args.len() < 5 {
        return Err(
            "Usage: AdvanceLiberty ROM INPUT|- FRAMES HASHES.csv|- interp|single|multi|play [LIBRARY ...] [--save PATH] [--controls PATH]"
                .into(),
        );
    }
    let mode = args[4].as_str();
    if (offline_play.is_some() || offline_record.is_some())
        && (mode != "multi" || force_interpreter)
    {
        return Err("Offline replay tools require native multi mode".into());
    }
    if offline_play.is_some()
        && (save_path.is_some() || session_path.is_some() || check_options.is_some())
    {
        return Err(
            "Offline playback cannot write cartridge saves/sessions or run block checks".into(),
        );
    }
    if preview_enabled
        && (!matches!(mode, "multi" | "play")
            || force_interpreter
            || scene_options.is_some()
            || camera_options.is_some())
    {
        return Err(
            "Camera preview requires native multi/play without other scene observers".into(),
        );
    }
    if scene_options.is_some() && (mode != "multi" || force_interpreter || camera_options.is_some())
    {
        return Err("Scene replay requires headless multi without camera tracing".into());
    }
    if camera_options.is_some() && (mode == "interp" || mode == "play" || force_interpreter) {
        return Err("Camera tracing requires headless native mode".into());
    }
    if audit_path.is_some() && (mode == "interp" || force_interpreter) {
        return Err(
            "Native audit/session requires recompiled single/multi/play without --interpreter"
                .into(),
        );
    }
    let mut frames: u64 = args[2].parse()?;
    if frames == 0 && mode == "play" {
        frames = u64::MAX;
    }
    if frames == 0 {
        return Err("FRAMES must be positive".into());
    }
    if !matches!(mode, "interp" | "single" | "multi" | "play") {
        return Err("Invalid mode".into());
    }
    if !force_interpreter
        && (mode == "single" && args.len() != 6
            || matches!(mode, "multi" | "play") && args.len() < 6)
    {
        return Err("single needs one library, multi/play need at least one".into());
    }
    #[cfg(not(feature = "desktop"))]
    if mode == "play" {
        return Err("Build with --features desktop for play".into());
    }
    let keys = if args[1] == "-" && (mode == "play" || offline_play.is_some()) {
        Vec::new()
    } else {
        input(&args[1])?
    };
    let recorded_language_changes = if args[1] == "-" {
        Vec::new()
    } else {
        localization::changes(std::path::Path::new(&args[1]))?
    };
    let mut camera_changes = if args[1] == "-" {
        Vec::new()
    } else {
        camera_mode::changes(&fs::read_to_string(&args[1])?)?
    }
    .into_iter()
    .peekable();
    if !recorded_language_changes.is_empty()
        && (mode == "interp" || force_interpreter || check_options.is_some())
    {
        return Err("Recorded language changes require native mode".into());
    }
    let aim_path = aim_path.or_else(|| {
        if args[1] == "-" {
            return None;
        }
        let path = std::path::Path::new(&args[1]).with_file_name("aim.gba-aim");
        path.exists().then_some(path)
    });
    let aim_keys = aim_path.as_ref().map(|p| mouse_aim::load(p)).transpose()?;
    if (force_interpreter || mode == "interp") && aim_keys.is_some() {
        return Err("Mouse aim replay requires native multi/single mode".into());
    }
    let recorded_drop_seconds = if args[1] != "-" {
        fs::read_to_string(std::path::Path::new(&args[1]).with_file_name("info.txt"))
            .ok()
            .and_then(|s| {
                s.lines().find_map(|l| {
                    l.strip_prefix("drop_lifetime_seconds=")
                        .and_then(|n| n.parse::<u32>().ok())
                })
            })
    } else {
        None
    };
    let drop_seconds = drop_lifetime_requested.or(recorded_drop_seconds);
    if drop_seconds.is_some_and(|s| !(1..=300).contains(&s)) {
        return Err("Recorded drop lifetime must be 1..300 seconds".into());
    }
    let mut drop_policy = drop_seconds.map(drop_lifetime::Policy::new);
    let mut drop_start = drop_start_requested
        .or_else(|| {
            if args[1] == "-" {
                return None;
            }
            fs::read_to_string(std::path::Path::new(&args[1]).with_file_name("info.txt"))
                .ok()
                .and_then(|s| {
                    s.lines().find_map(|l| {
                        l.strip_prefix("drop_lifetime_after_frame=")
                            .and_then(|n| n.parse::<u64>().ok())
                    })
                })
        })
        .unwrap_or(0);
    if drop_start != 0 && drop_seconds.is_none() {
        return Err("--drop-lifetime-after-frame requires --drop-lifetime".into());
    }
    let mut preserve_cash = drop_seconds.is_some()
        || preserve_cash_requested
        || args[1] != "-"
            && fs::read_to_string(std::path::Path::new(&args[1]).with_file_name("info.txt"))
                .is_ok_and(|text| text.lines().any(|line| line == "preserve_cash=1"));
    if preserve_cash && (mode == "interp" || force_interpreter) {
        return Err("--preserve-cash requires native dispatch mode".into());
    }
    if pickup_probe_path.is_some() && (mode == "interp" || force_interpreter) {
        return Err("--pickup-probe requires native dispatch mode".into());
    }
    let mut pickup_probe = pickup_probe_path
        .as_deref()
        .map(pickup_probe::Probe::open)
        .transpose()?;
    let rom = fs::read(&args[0])?;
    rom_profile::verify(&rom)?;
    let language_path = language_path.or_else(|| {
        if args[1] == "-" {
            return None;
        }
        let path = std::path::Path::new(&args[1]).with_file_name("language-pack.json");
        path.exists().then_some(path)
    });
    if language_path.is_none() && args[1] != "-" {
        let info = std::path::Path::new(&args[1]).with_file_name("info.txt");
        if fs::read_to_string(info).is_ok_and(|text| {
            text.lines()
                .any(|line| line == "language_pack=language-pack.json")
        }) {
            return Err(
                "Recorded language-pack.json is missing; restore the session's package".into(),
            );
        }
    }
    #[cfg(feature = "desktop")]
    let language_executable = std::env::current_exe()?;
    #[cfg(feature = "desktop")]
    let language_directory = localization::directory(&language_executable);
    #[cfg(feature = "desktop")]
    let language_choices = if mode == "play" {
        if args[1] == "-" {
            match localization::ensure_english(&language_directory, &rom) {
                Ok(Some(path)) => {
                    eprintln!("Editable English language pack created: {}", path.display())
                }
                Ok(None) => {}
                Err(error) => eprintln!("English language export: {error}"),
            }
        }
        localization::discover(&language_directory, &rom)
    } else {
        Vec::new()
    };
    #[cfg(feature = "desktop")]
    let language_path = language_path.or_else(|| {
        // Interactive launches use user preferences. Replays use only their
        // own snapshots or an explicit override, regardless of local settings.
        if mode != "play" || args[1] != "-" {
            return None;
        }
        let preferences_path = controls_path
            .clone()
            .unwrap_or_else(|| language_executable.with_file_name("controls.cfg"))
            .with_file_name("enhanced.cfg");
        let selected = options::Preferences::load(&preferences_path)
            .ok()?
            .language?;
        if language_choices
            .iter()
            .any(|choice| choice.file == selected)
        {
            Some(language_directory.join(selected))
        } else {
            eprintln!("Saved language pack unavailable: {selected}; using original game language");
            None
        }
    });
    let mut language = language_path
        .as_ref()
        .map(|path| localization::Language::load(path, &rom))
        .transpose()?;
    if language.is_some() && (mode == "interp" || force_interpreter || check_options.is_some()) {
        return Err(
            "Language packs require native mode without differential block checking".into(),
        );
    }
    if let Some(path) = text_trace.as_ref() {
        if mode == "interp" || force_interpreter {
            return Err("Text trace requires native mode".into());
        }
        localization::open_trace(path, &rom)?;
    }
    // Libraries must outlive every copied function pointer.
    let mut libraries = Vec::new();
    let mut table: HashMap<u32, Vec<BlockFn>> = HashMap::new();
    if mode != "interp" && !force_interpreter {
        for path in &args[5..] {
            let lib = unsafe { libloading::Library::new(path) }?;
            unsafe {
                if let Ok(version) = lib.get::<*const u32>(b"advance_native_api_version") {
                    if **version != 1 {
                        return Err("Unsupported native supplement API version".into());
                    }
                }
                let rescue = lib
                    .get::<*const u32>(b"advance_native_rescue_v1")
                    .is_ok_and(|version| **version == 1);
                if (scene_options.is_some()
                    || preview_enabled
                    || language.is_some()
                    || mode == "play"
                    || !recorded_language_changes.is_empty())
                    && !rescue
                {
                    return Err(
                        "Scene replay/language packs require one-instruction AOT libraries".into(),
                    );
                }
                let ptr: libloading::Symbol<*const RcgBlock> = lib.get(b"rcg_blocks")?;
                let count: libloading::Symbol<*const u64> = lib.get(b"rcg_block_count")?;
                if **count > 1_000_000 {
                    return Err("Invalid block count".into());
                }
                for b in std::slice::from_raw_parts(*ptr, **count as usize) {
                    if rescue {
                        RESCUE.with(|r| r.borrow_mut().entry(b.key).or_default().push(b.func));
                    }
                    let entries = table.entry(b.key).or_default();
                    if entries.is_empty() || b.key >> 24 == 3 || b.key >> 24 == 2 {
                        entries.push(b.func);
                    }
                }
            }
            libraries.push(lib);
        }
    }
    let mut language_changes = recorded_language_changes.into_iter().peekable();
    let mut active_language_file = language_path
        .as_ref()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .map(str::to_owned);
    let mut m = Machine::new(rom.clone());
    m.bus.tap_channels = enhanced_sound;
    let mut enhanced_audio = enhanced_sound.then(enhanced_audio::Output::new);
    if polyphonic {
        if !enhanced_sound {
            return Err("Polyphonic audio requires --enhanced-audio".into());
        }
        m.bus.pcm_overlay = Some(Default::default());
        eprintln!("Audio: experimental simultaneous PCM effects enabled");
    }
    if enhanced_sound {
        eprintln!(
            "Audio: Enhanced Output (per-channel sinc / DC blocker / soft clip), engine: {}",
            gba_core::engine::classify(&rom).describe()
        );
    }
    if let Some(language) = language.as_mut() {
        language.install(&mut m);
        println!(
            "Language pack: {} | translated={} | CustomFont={}",
            language.label, language.translated, language.extended
        );
    }
    let mut offline_playback = None;
    if let Some(path) = &offline_play {
        let (initial, player) = replay::load(path, &rom)?;
        m = initial.machine;
        enhanced_audio = initial.audio;
        drop_policy = initial.drops;
        language = initial.language;
        render_settings = initial.settings;
        preserve_cash = initial.preserve_cash;
        drop_start = initial.drop_start;
        frames = m
            .bus
            .frames
            .checked_add(player.frames.len() as u64)
            .ok_or("Replay frame overflow")?;
        offline_playback = Some(player);
    }
    if offline_record
        .as_ref()
        .is_some_and(|(_, start)| *start >= frames)
    {
        return Err("Offline recording start must precede the final frame".into());
    }
    let mut camera_preview = if preview_enabled {
        Some(camera_preview::Preview::new(preview_output)?)
    } else {
        None
    };
    let mut scene_replay = scene_options
        .map(|(path, start, count)| scene_replay::Replay::open(path, start, count, scene_offsets))
        .transpose()?;
    if check_options.is_some() && (mode == "interp" || mode == "play" || force_interpreter) {
        return Err("Block checking requires headless native mode".into());
    }
    let mut block_check = check_options
        .map(|(path, start, count)| block_check::Check::open(&path, start, count, &m.bus.rom))
        .transpose()?;
    let capture = capture_options
        .map(|(directory, frames)| frame_capture::Capture::open(directory, frames))
        .transpose()?;
    let save_path = save_path.or_else(|| {
        (mode == "play" && args[1] == "-")
            .then(|| std::path::Path::new(&args[0]).with_extension("recomp.sav"))
    });
    let mut save = save_path
        .map(|path| save::Save::open(path, &mut m))
        .transpose()?;
    let mut recording = session_path
        .map(|path| session::Session::open(path, &m, &args[0], &args[5..]))
        .transpose()?;
    let mut profile_is_session = false;
    if frame_profile.is_none() {
        if let Some(session) = recording.as_ref() {
            use std::io::Write;
            let mut file = std::io::BufWriter::new(fs::File::create_new(
                session.directory().join("frame-profile.csv"),
            )?);
            writeln!(file, "{FRAME_PROFILE_HEADER}")?;
            frame_profile = Some(file);
            profile_is_session = true;
        }
    }
    if let (Some(recording), Some(language)) = (recording.as_ref(), language.as_ref()) {
        recording.language_pack(&language.bytes)?;
    }
    if enhanced_sound {
        if let Some(recording) = recording.as_ref() {
            use std::io::Write;
            let mut info = fs::OpenOptions::new()
                .append(true)
                .open(recording.directory().join("info.txt"))?;
            writeln!(info, "audio_output=EnhancedOutput")?;
            if m.bus.pcm_overlay.is_some() {
                writeln!(info, "polyphonic_audio=1")?;
            }
        }
    }
    if preserve_cash {
        if let Some(recording) = recording.as_ref() {
            use std::io::Write;
            let mut info = fs::OpenOptions::new()
                .append(true)
                .open(recording.directory().join("info.txt"))?;
            writeln!(info, "preserve_cash=1")?;
            if let Some(seconds) = drop_seconds {
                writeln!(info, "drop_lifetime_seconds={seconds}")?;
                writeln!(info, "drop_lifetime_after_frame={drop_start}")?;
            }
        }
    }
    #[cfg(feature = "desktop")]
    let mut desktop = if mode == "play" {
        Some(desktop::Desktop::new(controls_path.unwrap_or(
            std::env::current_exe()?.with_file_name("controls.cfg"),
        ))?)
    } else {
        None
    };
    #[cfg(feature = "desktop")]
    if let Some(ui) = desktop.as_mut() {
        ui.set_languages(language_choices);
        if !_original_menus {
            ui.set_menu_skin(&m.bus.rom, language.as_ref())?;
        }
    }
    #[cfg(feature = "desktop")]
    let texture_creator = desktop.as_ref().map(|ui| ui.texture_creator());
    #[cfg(feature = "desktop")]
    if let Some(ui) = desktop.as_ref() {
        let saved = ui.initial_render_settings();
        render_settings.hud = saved.hud;
        if !override_aspect {
            render_settings.aspect = saved.aspect;
        }
        if !override_renderer {
            render_settings.renderer = saved.renderer;
        }
        if !override_textures {
            render_settings.textures = saved.textures;
        }
    }
    #[cfg(feature = "desktop")]
    let mut textures = texture_creator.as_ref().map(desktop::Textures::new);
    if let (Some(preview), Some(recording)) = (camera_preview.as_mut(), recording.as_ref()) {
        preview.log_session(recording.directory())?;
    }
    let mut api = RtApi {
        guard,
        interp_one: interp,
        read16: mouse_aim::read16,
        read32: polyphonic_audio::read32,
        ..RT_API
    };
    let camera_enabled = camera_options.is_some();
    if text_trace.is_some() {
        api.read32 = localization::read32;
    }
    if let Some((path, start, end)) = camera_options {
        camera_trace::open(&path, start, end)?;
        api.write8 = camera_trace::write8;
        api.write16 = camera_trace::write16;
        api.write32 = camera_trace::write32;
    }
    if let Some(path) = &sound_probe {
        if camera_enabled || mode != "multi" || force_interpreter {
            return Err("Sound probe requires native multi mode without camera write tracing".into());
        }
        audio_probe::open(path)?;
        api.write8 = audio_probe::write8;
        api.write16 = audio_probe::write16;
        api.write32 = audio_probe::write32;
    }
    let native_api = native_host::Api::new(api);
    let api = &native_api.base;
    let _api_scope = ApiScope::enter(api);
    let mut misses = 0u64;
    let mut rejected_fallback = 0u64;
    // A supplement may contain several guarded versions of one RAM address.
    // Candidate indices are per address, not library indices.
    let mut selected = vec![0u64; table.values().map(Vec::len).max().unwrap_or(0)];
    let mut output = String::from(if audio_hashes {
        "frame,framebuffer_upstream_hash,audio_samples_hash\n"
    } else {
        "frame,framebuffer_upstream_hash\n"
    });
    let mut replay_recording: Option<replay::Recording> = None;
    let mut playback: Option<replay::Playback> = offline_playback;
    let mut live: Option<Box<replay::Live>> = None;
    let mut suspended_session = None;
    let mut replay_counts = None;
    let mut replay_language_changes = Vec::new();
    macro_rules! restore_game {
        () => {
            if let Some(state) = live.take() {
                let replay::Live {
                    machine,
                    audio,
                    drops,
                    language: old_language,
                    preview,
                    settings,
                    preserve_cash: cash,
                    drop_start: start,
                } = *state;
                m = machine;
                enhanced_audio = audio;
                drop_policy = drops;
                language = old_language;
                camera_preview = preview;
                render_settings = settings;
                preserve_cash = cash;
                drop_start = start;
                playback = None;
                recording = suspended_session.take();
                if let Some((counts, entries, old_misses, rejected, cash)) = replay_counts.take() {
                    COUNTERS.with(|c| {
                        c.replace(counts);
                    });
                    selected = entries;
                    misses = old_misses;
                    rejected_fallback = rejected;
                    cash_retained = cash;
                }
                VIEWING_REPLAY.with(|flag| flag.set(false));
                #[cfg(feature = "desktop")]
                if let Some(ui) = desktop.as_mut() {
                    ui.replay_mode(false);
                    if !_original_menus {
                        ui.set_menu_skin(&m.bus.rom, language.as_ref())?;
                    }
                }
            }
        };
    }
    let start = std::time::Instant::now();
    while m.bus.frames < frames || playback.is_some() {
        if playback
            .as_ref()
            .is_some_and(|p| p.cursor == p.frames.len())
        {
            if offline_play.is_some() {
                break;
            }
            restore_game!();
            continue;
        }
        if offline_record
            .as_ref()
            .is_some_and(|(_, start)| *start == m.bus.frames)
        {
            replay_recording = Some(replay::Recording::start(
                &m,
                &rom,
                &enhanced_audio,
                &drop_policy,
                &language,
                render_settings,
                preserve_cash,
                drop_start,
            )?);
        }
        let profile_start = frame_profile.as_ref().map(|_| std::time::Instant::now());
        mouse_aim::set(&m.bus, None);
        let before = m.bus.frames;
        while camera_changes
            .peek()
            .is_some_and(|(frame, _)| *frame == before)
        {
            let (_, mode) = camera_changes.next().unwrap();
            camera_mode::set(&mut m.bus, mode)?;
            if let Some(recording) = recording.as_mut() {
                recording.camera_change(before, mode)?;
            }
        }
        while language_changes
            .peek()
            .is_some_and(|(frame, _)| *frame == before)
        {
            let (_, bytes) = language_changes.next().unwrap();
            localization::change_live(&mut m, &rom, &mut language, bytes.clone())?;
            if replay_recording.is_some() {
                replay_language_changes.push(match bytes.clone() {
                    Some(bytes) => replay::LanguageChange::Pack(bytes),
                    None => replay::LanguageChange::Original,
                });
            }
            if let Some(recording) = recording.as_mut() {
                recording.language_change(before, bytes.as_deref())?;
            }
        }
        let i = keys.partition_point(|&(f, _)| f <= before);
        m.bus.keys = if i == 0 { 0x3ff } else { keys[i - 1].1 };
        let control_context = control_context::detect(&m.bus);
        let mut aim_angle = aim_keys.as_ref().and_then(|points| {
            let i = points.partition_point(|&(f, _)| f <= before);
            i.checked_sub(1).and_then(|i| points[i].1)
        });
        let mut aim_presentation = aim_keys.is_some();
        if let Some(trace) = control_trace.as_mut() {
            use std::io::Write;
            writeln!(
                trace,
                "{before},{control_context:?},{:08x},{:04x},{:02x}",
                u32::from_le_bytes(m.bus.ewram[0xf10..0xf14].try_into().unwrap()),
                u16::from_le_bytes(m.bus.io[..2].try_into().unwrap()),
                m.bus.ewram[0x494]
            )?;
        }
        #[cfg(feature = "desktop")]
        if let Some(ui) = desktop.as_mut() {
            ui.sync_cheats(&m.bus);
            ui.sync_menu_region(&m.bus);
            #[cfg(test)]
            REPLAY_TEST_DRIVER.with(|driver| {
                if let Some(driver) = driver.borrow_mut().as_mut() {
                    driver(&m, ui);
                }
            });
            let Some(keys) = ui.poll(&mut render_settings, preview_enabled, control_context)?
            else {
                break;
            };
            let cycles = ui.take_camera_cycles();
            if args[1] == "-" && playback.is_none() {
                for _ in 0..cycles {
                    if let Some(mode) = camera_mode::cycle(&mut m.bus) {
                        ui.play_menu_click();
                        if let Some(recording) = recording.as_mut() {
                            recording.camera_change(before, mode)?;
                        }
                        eprintln!("Camera: {}", camera_mode::NAMES[mode as usize]);
                    }
                }
            }
            if ui.take_language_scan() {
                ui.set_languages(localization::discover(&language_directory, &rom));
            }
            if let Some(selected) = ui.take_language_change() {
                let bytes = selected
                    .as_ref()
                    .map(|name| fs::read(language_directory.join(name)))
                    .transpose();
                let result = bytes
                    .map_err(|error| -> Box<dyn std::error::Error> { error.into() })
                    .and_then(|bytes| {
                        if force_interpreter {
                            return Err("Live language changes require native mode".into());
                        }
                        localization::change_live(&mut m, &rom, &mut language, bytes.clone())?;
                        if replay_recording.is_some() {
                            replay_language_changes.push(match bytes.clone() {
                                Some(bytes) => replay::LanguageChange::Pack(bytes),
                                None => replay::LanguageChange::Original,
                            });
                        }
                        if let Some(recording) = recording.as_mut() {
                            recording.language_change(before, bytes.as_deref())?;
                        }
                        Ok(())
                    });
                match result {
                    Ok(()) => {
                        active_language_file = selected;
                        if !_original_menus {
                            ui.set_menu_skin(&m.bus.rom, language.as_ref())?;
                        }
                        eprintln!("Language applied live at frame {before}");
                    }
                    Err(error) => {
                        ui.language_error(active_language_file.clone(), &error.to_string())
                    }
                }
            }
            if args[1] == "-" {
                m.bus.keys = keys;
            }
            if let Some(command) = ui.replays.command.take() {
                use replay::Command;
                if matches!(command, Command::Stop) {
                    restore_game!();
                    continue;
                }
                if matches!(
                    command,
                    Command::Record | Command::Latest | Command::Play(_)
                ) {
                    let was_recording = replay_recording.is_some();
                    if let Some(clip) = replay_recording.take() {
                        match clip.save(&replay_directory, &rom) {
                            Ok(Some(path)) => {
                                eprintln!("Replay saved: {}", path.display());
                                ui.replays.notice("Replay saved".into());
                            }
                            Ok(None) => ui.replays.notice("Empty replay discarded".into()),
                            Err(error) => {
                                eprintln!("Cannot save replay: {error}");
                                ui.replays.notice(format!("Replay save failed: {error}"));
                            }
                        }
                        ui.replays.recording = false;
                    }
                    if matches!(command, Command::Record) {
                        if !was_recording {
                            match replay::Recording::start(
                                &m,
                                &rom,
                                &enhanced_audio,
                                &drop_policy,
                                &language,
                                render_settings,
                                preserve_cash,
                                drop_start,
                            ) {
                                Ok(clip) => {
                                    replay_language_changes.clear();
                                    replay_recording = Some(clip);
                                    ui.replays.recording = true;
                                }
                                Err(error) => {
                                    ui.replays.notice(format!("Cannot record replay: {error}"))
                                }
                            }
                        }
                        continue;
                    }
                }
                if matches!(command, Command::Browse) {
                    match replay::list(&replay_directory) {
                        Ok(entries) => ui.replays.entries(entries),
                        Err(error) => ui.replays.notice(format!("Cannot list replays: {error}")),
                    }
                    continue;
                }
                let path = match command {
                    Command::Play(path) => Some(path),
                    Command::Latest => match replay::list(&replay_directory) {
                        Ok(entries) => entries.first().map(|e| e.path.clone()),
                        Err(error) => {
                            ui.replays.notice(error.to_string());
                            None
                        }
                    },
                    _ => None,
                };
                if let Some(path) = path {
                    // Validate every file and prepare all fallible objects before moving live state.
                    let prepared = (|| -> Result<_, Box<dyn std::error::Error>> {
                        let (mut initial, playback) = replay::load(&path, &rom)?;
                        let clip_language = initial.language.take();
                        let preview = if preview_enabled {
                            Some(camera_preview::Preview::new(None)?)
                        } else {
                            None
                        };
                        Ok((initial, playback, clip_language, preview))
                    })();
                    match prepared {
                        Ok((initial, clip, clip_language, preview)) => {
                            let mut presentation = initial.settings;
                            // Viewing uses the user's current renderer, independent of the clip.
                            presentation.renderer = render_settings.renderer;
                            let state = replay::Live {
                                machine: std::mem::replace(&mut m, initial.machine),
                                audio: std::mem::replace(&mut enhanced_audio, initial.audio),
                                drops: std::mem::replace(&mut drop_policy, initial.drops),
                                language: std::mem::replace(&mut language, clip_language),
                                preview: std::mem::replace(&mut camera_preview, preview),
                                settings: std::mem::replace(&mut render_settings, presentation),
                                preserve_cash: std::mem::replace(
                                    &mut preserve_cash,
                                    initial.preserve_cash,
                                ),
                                drop_start: std::mem::replace(&mut drop_start, initial.drop_start),
                            };
                            live = Some(Box::new(state));
                            playback = Some(clip);
                            suspended_session = recording.take();
                            let empty = vec![0; selected.len()];
                            replay_counts = Some((
                                COUNTERS.with(|c| std::mem::take(&mut *c.borrow_mut())),
                                std::mem::replace(&mut selected, empty),
                                std::mem::take(&mut misses),
                                std::mem::take(&mut rejected_fallback),
                                std::mem::take(&mut cash_retained),
                            ));
                            VIEWING_REPLAY.with(|flag| flag.set(true));
                            ui.replay_mode(true);
                            if !_original_menus {
                                ui.set_menu_skin(&m.bus.rom, language.as_ref())?;
                            }
                            eprintln!("Replay started: {}", path.display());
                        }
                        Err(error) => {
                            eprintln!("Cannot play replay: {error}");
                            ui.replays.notice(format!("Cannot play replay: {error}"));
                        }
                    }
                } else {
                    ui.replays
                        .notice("No saved replay. Press F2 to record.".into());
                }
                continue;
            }
            if ui.paused() {
                ui.present_paused(textures.as_mut().unwrap())?;
                ui.update_status(
                    coverage(selected.iter().sum(), misses, rejected_fallback),
                    force_interpreter,
                )?;
                continue;
            }
            if args[1] == "-" && !force_interpreter {
                aim_angle = ui.aim_angle(&m.bus);
                aim_presentation = ui.mouse_aim_enabled();
            }
        }
        if let Some(player) = playback.as_ref() {
            let packet = &player.frames[player.cursor];
            m.bus.keys = packet.keys;
            aim_angle = packet.aim;
            aim_presentation = packet.aim_presentation;
            let renderer = render_settings.renderer;
            render_settings = packet.settings;
            render_settings.renderer = renderer;
            camera_mode::set(&mut m.bus, packet.camera)?;
            for change in &packet.language {
                let bytes = match change {
                    replay::LanguageChange::Original => None,
                    replay::LanguageChange::Pack(bytes) => Some(bytes.clone()),
                };
                localization::change_live(&mut m, &rom, &mut language, bytes)?;
                #[cfg(feature = "desktop")]
                if !_original_menus {
                    if let Some(ui) = desktop.as_mut() {
                        ui.set_menu_skin(&m.bus.rom, language.as_ref())?;
                    }
                }
            }
        }
        let mut replay_packet = replay::Frame {
            keys: m.bus.keys,
            aim: aim_angle,
            aim_presentation,
            camera: m.bus.ewram[0x10c66].min(3),
            settings: render_settings,
            language: std::mem::take(&mut replay_language_changes),
            checksum: 0,
        };
        if let Some(policy) = drop_policy.as_mut().filter(|_| m.bus.frames >= drop_start) {
            policy.update(&mut m);
        }
        // Resolve and validate the current actor on every host frame. Hooks
        // cannot affect vehicle physics or any other object's direction.
        mouse_aim::set(&m.bus, aim_angle);
        mouse_aim::present(&m.bus, aim_presentation);
        if let Some(recording) = recording.as_mut() {
            recording.input(before, m.bus.keys)?;
            recording.aim(before, aim_angle)?;
        }
        if mode == "interp" || force_interpreter {
            m.run_frame(5_000_000);
        } else {
            m.bus.frame_ready = false;
            let mut steps = 0;
            while !m.bus.frame_ready {
                steps += 1;
                if steps > 200_000_000 {
                    return Err("Frame step guard exceeded".into());
                }
                if m.bus.halted
                    || (m.cpu.regs[15] == gba_core::machine::IRQ_RETURN_ADDR
                        && m.cpu.mode() == Mode::Irq)
                    || (m.bus.irq_pending() && !m.cpu.flag(gba_core::cpu::FLAG_I))
                {
                    // A halted CPU can wake and execute an instruction in this same
                    // step. Count it too; IRQ/idle bookkeeping itself is native HLE.
                    if m.bus.halted
                        && m.bus.irq_line()
                        && !(m.cpu.regs[15] == gba_core::machine::IRQ_RETURN_ADDR
                            && m.cpu.mode() == Mode::Irq)
                        && !(m.bus.irq_pending() && !m.cpu.flag(gba_core::cpu::FLAG_I))
                    {
                        COUNTERS.with(|c| c.borrow_mut().halt_wake += 1);
                        audit("halt_wake", &m);
                    }
                    m.step();
                    continue;
                }
                let key = m.cpu.regs[15] | m.cpu.thumb() as u32;
                if let Some(policy) = drop_policy.as_mut() {
                    if drop_lifetime::Policy::interested(key) && m.bus.frames >= drop_start {
                        policy.observe(&mut m, key);
                    }
                }
                if preserve_cash && key == cash_protection::CHECK && cash_protection::apply(&mut m)
                {
                    cash_retained += 1;
                }
                if let Some(probe) = pickup_probe.as_mut() {
                    if pickup_probe::Probe::interested(key) {
                        probe.observe(&m, key)?;
                    }
                }
                #[cfg(feature = "desktop")]
                if key == cheats::MAIN_CHECK {
                    if let Some(ui) = desktop.as_mut() {
                        ui.observe_cheat_menu(m.bus.frames);
                    }
                }
                #[cfg(feature = "desktop")]
                if game_menu::Frontend::interested(key) {
                    if let Some(ui) = desktop.as_mut() {
                        ui.observe_game_menu(&m, key);
                    }
                }
                if let Some(language) = language.as_mut() {
                    if key == 0x0802de67
                        || text_trace.is_some()
                            && matches!(key, 0x0802e4f9 | 0x0802e191 | 0x0802e383)
                    {
                        language.observe(&mut m, key);
                    }
                }
                if let Some(preview) = camera_preview.as_mut() {
                    if preview.interested(key) {
                        preview.observe(&m, key);
                    }
                }
                if let Some(replay) = scene_replay.as_mut() {
                    replay.observe(&m, &table, &keys)?;
                }
                if let Some(entries) = table.get(&key) {
                    let snapshot = block_check
                        .as_mut()
                        .map(|c| c.snapshot(&mut m))
                        .transpose()?
                        .flatten();
                    let mut accepted = false;
                    for (index, f) in entries.iter().enumerate() {
                        COUNTERS.with(|c| {
                            let mut c = c.borrow_mut();
                            c.calls += 1;
                            c.rejected = false;
                            c.retry = mode != "single";
                        });
                        let ptr = &mut m as *mut Machine as *mut c_void;
                        f(api, ptr);
                        let rejected = COUNTERS.with(|c| c.borrow().rejected);
                        if mode == "single" || !rejected {
                            selected[index] += 1;
                            accepted = true;
                            break;
                        }
                        // No instruction may execute before a rejected RAM entry guard.
                        if m.cpu.regs[15] | m.cpu.thumb() as u32 != key {
                            return Err("Guard rejection changed guest PC".into());
                        }
                    }
                    if !accepted {
                        rejected_fallback += 1;
                        audit("guard", &m);
                        (RT_API.interp_one)(&mut m as *mut Machine as *mut c_void);
                    }
                    if let Some(snapshot) = snapshot {
                        block_check.as_mut().unwrap().compare(snapshot, &m)?;
                    }
                } else {
                    misses += 1;
                    audit("dispatch", &m);
                    m.step();
                }
            }
        }
        let profile_native_ns = profile_start.map_or(0, |start| start.elapsed().as_nanos());
        if m.bus.frames == before {
            return Err("No frame progress".into());
        }
        if let Some(probe) = pickup_probe.as_mut() {
            probe.frame(&m)?;
        }
        if camera_enabled {
            camera_trace::frame(&m)?;
        }
        if let Some(replay) = scene_replay.as_mut() {
            replay.frame(&m)?;
        }
        if let Some(trace) = state_trace.as_mut() {
            use std::io::Write;
            let bytes_hash = |bytes: &[u8]| {
                bytes.iter().fold(0xcbf29ce484222325u64, |h, b| {
                    (h ^ *b as u64).wrapping_mul(0x1_0000_01b3)
                })
            };
            writeln!(
                trace,
                "{},{},{:08x},{:08x},{:04x},{:04x},{},{:016x},{:016x},\"{:08x?}\"",
                m.bus.frames,
                m.bus.clock,
                m.cpu.regs[15],
                m.cpu.cpsr,
                m.bus.reg_ie,
                m.bus.reg_if,
                m.bus.ime,
                bytes_hash(&m.bus.ewram),
                bytes_hash(&m.bus.iwram),
                m.cpu.regs
            )?;
        }
        if let Some(capture) = &capture {
            capture.frame(&m)?;
        }
        if let Some(recording) = recording.as_mut() {
            recording.frame(&m)?;
            if m.bus.frames % 300 == 0 {
                AUDIT.with(|a| {
                    a.borrow()
                        .checkpoint(&recording.directory().join("fallbacks.partial.csv"))
                })?;
            }
        }
        #[cfg(feature = "desktop")]
        let defer_save_write = desktop.as_ref().is_some_and(|ui| ui.saving_game_for_exit());
        #[cfg(not(feature = "desktop"))]
        let defer_save_write = false;
        if !defer_save_write && playback.is_none() {
            if let Some(save) = save.as_mut() {
                save.update(&m, false)?;
            }
        }
        #[cfg(feature = "desktop")]
        if let Some(ui) = desktop.as_mut() {
            if ui.save_exit_needs_commit() {
                let result = save
                    .as_mut()
                    .ok_or_else(|| "No save file is configured".to_owned())
                    .and_then(|save| save.update(&m, true).map_err(|e| e.to_string()));
                ui.finish_save_exit(result);
            }
        }
        let profile_render_start = frame_profile.as_ref().map(|_| std::time::Instant::now());
        #[cfg(feature = "desktop")]
        let preview_image = camera_preview
            .as_mut()
            .and_then(|p| p.frame_configured(&m.bus, render_settings));
        let profile_render_ns = profile_render_start.map_or(0, |start| start.elapsed().as_nanos());
        let profile_audio_start = frame_profile.as_ref().map(|_| std::time::Instant::now());
        let enhanced_samples = enhanced_audio.as_mut().map(|audio| audio.frame(&mut m.bus));
        let profile_audio_ns = profile_audio_start.map_or(0, |start| start.elapsed().as_nanos());
        #[cfg(feature = "desktop")]
        if let (Some(capture), Some(image)) = (&capture, &preview_image) {
            capture.presentation(
                m.bus.frames,
                image,
                camera_preview
                    .as_ref()
                    .and_then(|p| p.diagnostic_graphics()),
            )?;
        }
        #[cfg(not(feature = "desktop"))]
        if let Some(preview) = camera_preview.as_mut() {
            let _ = preview.frame_configured(&m.bus, render_settings);
        }
        #[cfg(feature = "desktop")]
        if let Some(ui) = desktop.as_mut() {
            ui.set_preview_status(camera_preview.as_ref().map(|p| p.status()).unwrap_or(""));
            ui.set_render_settings(render_settings, preview_enabled);
            if let Some(capture) = &capture {
                capture.menu(m.bus.frames, ui)?;
            }
            ui.present_with_audio(
                &m.bus,
                preview_image.as_deref(),
                textures.as_mut().unwrap(),
                enhanced_samples,
            )?;
            ui.set_aim_projection(if preview_image.is_some() {
                camera_preview.as_ref().and_then(|p| p.aim_projection())
            } else {
                mouse_aim::Projection::original(&m.bus)
            });
            ui.update_status(
                coverage(selected.iter().sum(), misses, rejected_fallback),
                force_interpreter,
            )?;
        }
        if args[3] != "-" {
            output.push_str(&format!(
                "{},{:016x}",
                m.bus.frames,
                hash(&m.bus.framebuffer)
            ));
            if audio_hashes {
                let samples: Vec<u16> = m.bus.audio_buf.iter().map(|&x| x as u16).collect();
                output.push_str(&format!(",{:016x}", hash(&samples)));
            }
            output.push('\n');
        }
        if let Some(audio) = audio_output.as_mut() {
            use std::io::Write;
            for sample in enhanced_samples.unwrap_or(&m.bus.audio_buf) {
                audio.write_all(&sample.to_le_bytes())?;
            }
        }
        if let Some(file) = frame_profile
            .as_mut()
            .filter(|_| !profile_is_session || playback.is_none())
        {
            use std::io::Write;
            #[cfg(feature = "desktop")]
            let (queued, underruns) = desktop.as_mut().map_or((0, 0), |ui| ui.audio_health());
            #[cfg(not(feature = "desktop"))]
            let (queued, underruns) = (0, 0);
            writeln!(file,"{},{},{profile_native_ns},{profile_render_ns},{profile_audio_ns},{},{},{},{},{},{},{queued},{underruns}",
                m.bus.frames,profile_start.unwrap().elapsed().as_nanos(), camera_preview.as_ref().map_or(0, |p| p.accepted),
                m.bus.audio_buf.len(),m.bus.fifo_underruns[0],m.bus.fifo_underruns[1],m.bus.fifo_refills[0],m.bus.fifo_refills[1])?;
        }
        if sound_probe.is_some() {
            audio_probe::frame(&m)?;
        }
        m.bus.audio_buf.clear();
        if replay_recording.is_some() || playback.is_some() {
            replay_packet.checksum = replay::checksum(&m);
        }
        if let Some(clip) = replay_recording.as_mut() {
            clip.push(replay_packet.clone());
        }
        if replay_recording.as_ref().is_some_and(|clip| clip.full()) {
            if let Some(clip) = replay_recording.take() {
                clip.save(&replay_directory, &rom)?;
            }
            #[cfg(feature = "desktop")]
            if let Some(ui) = desktop.as_mut() {
                ui.replays.recording = false;
                ui.replays.notice("Replay saved (one-hour limit)".into());
            }
        }
        if let Some(player) = playback.as_mut() {
            if player.frames[player.cursor].checksum != replay_packet.checksum {
                eprintln!("Replay diverged at frame {}", player.cursor);
                #[cfg(feature = "desktop")]
                if let Some(ui) = desktop.as_mut() {
                    ui.replays
                        .notice("Replay stopped: simulation mismatch".into());
                }
                player.cursor = player.frames.len();
            } else {
                player.cursor += 1;
            }
        }
    }
    // Window close during playback must commit only the suspended live save.
    #[allow(unused_assignments)]
    {
        restore_game!();
    }
    let replay_save_result = if let Some(clip) = replay_recording.take() {
        clip.save(&replay_directory, &rom).map(|_| ())
    } else {
        Ok(())
    };
    audio_probe::close();
    building_material::close_roof_trace()?;
    if let Some(file) = frame_profile.as_mut() {
        use std::io::Write;
        file.flush()?;
    }
    if let Some(audio) = audio_output.as_mut() {
        use std::io::Write;
        audio.flush()?;
    }
    if let Some(trace) = state_trace.as_mut() {
        use std::io::Write;
        trace.flush()?;
    }
    if let Some(probe) = pickup_probe.as_mut() {
        probe.finish()?;
    }
    if let Some(check) = block_check.as_mut() {
        check.finish()?;
    }
    if camera_enabled {
        camera_trace::finish()?;
    }
    if let Some(preview) = camera_preview.as_mut() {
        preview.finish()?;
    }
    #[cfg(feature = "desktop")]
    if let (Some(ui), Some(textures)) = (&desktop, &textures) {
        ui.finish(textures);
    }
    if let Some(replay) = scene_replay.as_mut() {
        replay.finish()?;
    }
    if let Some(save) = save.as_mut() {
        save.update(&m, true)?;
    }
    if let Some(recording) = recording.as_mut() {
        let (native, interpreted) = COUNTERS.with(|c| {
            let c = c.borrow();
            (
                selected.iter().sum::<u64>() + c.rescued,
                misses + rejected_fallback + c.internal + c.halt_wake,
            )
        });
        recording.finish(m.bus.frames, native, interpreted)?;
    }
    // Commit live progress and finish diagnostics even if writing the replay failed.
    replay_save_result?;
    #[cfg(feature = "desktop")]
    if let Some(ui) = desktop.as_mut() {
        // Force a final title update even for short diagnostic replays.
        ui.final_status(
            coverage(selected.iter().sum(), misses, rejected_fallback),
            force_interpreter,
        )?;
        println!("window_title={}", ui.title());
    }
    if args[3] != "-" {
        fs::write(&args[3], output)?;
    }
    if let Some(language) = language.as_ref() {
        println!(
            "language_pack_translated={} extended_glyph_calls={}",
            language.translated, language.glyph_calls
        );
    }
    localization::close_trace()?;
    if preserve_cash {
        println!("preserve_cash retained_checks={cash_retained}");
    }
    if let Some(policy) = drop_policy.as_ref() {
        println!(
            "drop_lifetime seconds={} held={} emergency_corpses={} expired={}",
            drop_seconds.unwrap_or_else(|| policy.seconds()),
            policy.held,
            policy.emergency_bodies,
            policy.expired
        );
    }
    println!("mode={mode} frames={} hash={:016x} seconds={:.3} dispatch_miss_steps={misses} rejected_fallback_steps={rejected_fallback} selected={selected:?}",m.bus.frames,hash(&m.bus.framebuffer),start.elapsed().as_secs_f64());
    if mode == "interp" || force_interpreter {
        return Ok(());
    }
    COUNTERS.with(|c| {
        let c=c.borrow();
        println!("native_calls={} rescued_native_instructions={} internal_interp_steps={} guard_checks={} guard_failures={} tracked_interpreter_steps={}",c.calls,c.rescued,c.internal,c.guard_checks,c.guard_failures,misses+c.internal+rejected_fallback+c.halt_wake);
        let mut hot:Vec<_>=c.failed.iter().collect();hot.sort_by_key(|&(_,v)|std::cmp::Reverse(*v));
        for (addr,count) in hot.into_iter().take(16) {println!("guard_fail {addr:08x} {count}");}
    });
    if let Some(path) = audit_path {
        let total = AUDIT.with(|a| -> std::io::Result<u64> {
            let a = a.borrow();
            a.write(&path)?;
            Ok(a.total())
        })?;
        println!(
            "audit_interpreted_instructions={total} report={}",
            path.display()
        );
        if strict_native && total != 0 {
            return Err(
                "Native-only check failed: guest instructions were interpreted (see audit CSV)"
                    .into(),
            );
        }
    }
    Ok(())
}
fn main() {
    #[cfg(feature = "desktop")]
    {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if launcher::is_portable(&args) {
            if let Err(error) = launcher::launch(&args) {
                eprintln!("{error}");
                launcher::show_error(&error);
                std::process::exit(1);
            }
            return;
        }
    }
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> Machine {
        let mut m = Machine::new(0xe3a00007u32.to_le_bytes().to_vec()); // ARM MOV r0, #7
        m.cpu.regs[15] = 0x08000000;
        m.cpu.cpsr = 0x1f;
        m
    }

    extern "C" fn rejecting_candidate(api: *const RtApi, m: *mut c_void) -> u32 {
        let api = unsafe { &*api };
        assert_eq!((api.guard)(m, 0x03000000, 4, 0), 0);
        (api.interp_one)(m)
    }

    extern "C" fn accepting_candidate(_: *const RtApi, m: *mut c_void) -> u32 {
        let m = unsafe { &mut *(m as *mut Machine) };
        m.cpu.regs[0] = 123;
        m.cpu.regs[15] += 4;
        m.cpu.regs[15]
    }

    #[test]
    fn internal_exit_tries_native_variants_without_interpreting_or_leaking_rejection() {
        COUNTERS.with(|c| *c.borrow_mut() = Counters::default());
        RESCUE.with(|r| {
            r.borrow_mut()
                .insert(0x08000000, vec![rejecting_candidate, accepting_candidate])
        });
        let mut m = machine();
        assert_eq!(interp(&mut m as *mut _ as *mut c_void), 0x08000004);
        assert_eq!(m.cpu.regs[0], 123);
        COUNTERS.with(|c| {
            let c = c.borrow();
            assert_eq!(c.internal, 0);
            assert_eq!(c.rescued, 1);
            assert!(!c.rejected);
        });
        RESCUE.with(|r| r.borrow_mut().clear());
    }

    #[test]
    fn rejection_defers_instruction_until_dispatcher_fallback() {
        COUNTERS.with(|c| {
            *c.borrow_mut() = Counters {
                retry: true,
                ..Counters::default()
            }
        });
        let mut m = machine();
        let ptr = &mut m as *mut Machine as *mut c_void;
        assert_eq!(guard(ptr, 0x03000000, 4, 0), 0);
        let regs = m.cpu.regs;
        assert_eq!(interp(ptr), 0x08000000);
        assert_eq!(m.cpu.regs, regs);
        COUNTERS.with(|c| assert_eq!(c.borrow().internal, 0));
        (RT_API.interp_one)(ptr);
        assert_eq!(m.cpu.regs[0], 7);
        assert_eq!(m.cpu.regs[15], 0x08000004);
    }

    #[test]
    fn single_mode_executes_rejected_instruction() {
        COUNTERS.with(|c| *c.borrow_mut() = Counters::default());
        let mut m = machine();
        let ptr = &mut m as *mut Machine as *mut c_void;
        assert_eq!(guard(ptr, 0x03000000, 4, 0), 0);
        assert_eq!(interp(ptr), 0x08000004);
        assert_eq!(m.cpu.regs[0], 7);
        COUNTERS.with(|c| assert_eq!(c.borrow().internal, 1));
    }

    #[test]
    fn matching_guard_does_not_suppress_instruction() {
        COUNTERS.with(|c| {
            *c.borrow_mut() = Counters {
                retry: true,
                ..Counters::default()
            }
        });
        let mut m = machine();
        let ptr = &mut m as *mut Machine as *mut c_void;
        let expected = hash(&[0, 0]); // Four zero bytes in IWRAM.
        assert_eq!(guard(ptr, 0x03000000, 4, expected), 1);
        assert_eq!(interp(ptr), 0x08000004);
        COUNTERS.with(|c| assert_eq!(c.borrow().internal, 1));
    }
}

#[cfg(all(test, feature = "desktop"))]
mod replay_runtime_tests {
    use super::*;
    #[test]
    #[ignore = "requires local USA ROM, native libraries and recorded input"]
    fn native_record_play_browser_abort_and_live_save_restoration() {
        use sdl2::{
            event::Event,
            keyboard::{Keycode, Mod, Scancode},
        };
        use std::{path::PathBuf, rc::Rc};
        let root = PathBuf::from("artifacts/replay-integration").join(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
                .to_string(),
        );
        fs::create_dir_all(&root).unwrap();
        let fixture = PathBuf::from("data/sessions/session-b9d955bed15e4c618fda627f01edc5aa");
        let save = root.join("test.sav");
        fs::copy(fixture.join("initial.sav"), &save).unwrap();
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read("native/usa/manifest.json").unwrap()).unwrap();
        let mut args = vec![
            "roms/usa.gba".into(),
            fixture.join("input.gba-input").to_str().unwrap().into(),
            "2500".into(),
            "-".into(),
            "play".into(),
        ];
        args.extend(
            manifest["libraries"]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| l["path"].as_str().unwrap().into()),
        );
        args.extend([
            "--camera-preview".into(),
            "--enhanced-audio".into(),
            "--polyphonic-audio".into(),
            "--drop-lifetime".into(),
            "30".into(),
            "--save".into(),
            save.to_str().unwrap().into(),
            "--controls".into(),
            root.join("controls.cfg").to_str().unwrap().into(),
            "--replays".into(),
            root.join("replays").to_str().unwrap().into(),
            "--record-session".into(),
            root.join("session").to_str().unwrap().into(),
        ]);
        let phases = Rc::new(Cell::new(0));
        let phase = phases.clone();
        let mut saved = Vec::new();
        let mut saved_calls = 0;
        let mut end_frame = 0;
        let mut viewer_switched = false;
        let mut original_files = Vec::new();
        let replay_root = root.join("replays");
        REPLAY_TEST_FAST.with(|fast| fast.set(true));
        REPLAY_TEST_DRIVER.with(|driver| {
            *driver.borrow_mut() = Some(Box::new(move |m, ui| {
                let push = |key| {
                    ui.push_test_event(Event::KeyDown {
                        timestamp: 0,
                        window_id: 0,
                        keycode: Some(key),
                        scancode: Scancode::from_keycode(key),
                        keymod: Mod::NOMOD,
                        repeat: false,
                    })
                };
                let key = match phase.get() {
                    0 if m.bus.frames >= 1450
                        && control_context::detect(&m.bus) != control_context::Context::Menu =>
                    {
                        phase.set(1);
                        end_frame = m.bus.frames + 80;
                        Some(Keycode::F2)
                    }
                    1 if ui.replays.recording && m.bus.frames >= end_frame => {
                        saved = serde_json::to_vec(m).unwrap();
                        saved_calls = COUNTERS.with(|c| c.borrow().calls);
                        phase.set(2);
                        push(Keycode::F6); // Select Original after recording, before viewing.
                        Some(Keycode::F3)
                    }
                    2 if ui.replays.playing => {
                        if original_files.is_empty() {
                            let entry = replay::list(&replay_root).unwrap().remove(0);
                            original_files = ["initial.json", "input.json", "info.json"]
                                .iter()
                                .map(|name| {
                                    (
                                        entry.path.join(name),
                                        fs::read(entry.path.join(name)).unwrap(),
                                    )
                                })
                                .collect();
                        }
                        assert_eq!(
                            ui.initial_render_settings().renderer,
                            if viewer_switched {
                                render_settings::Renderer::Enhanced
                            } else {
                                render_settings::Renderer::Original
                            }
                        );
                        if !viewer_switched && m.bus.frames >= end_frame - 60 {
                            viewer_switched = true;
                            Some(Keycode::F6)
                        } else {
                            None
                        }
                    }
                    2 if !ui.replays.playing && !ui.replays.recording => {
                        assert_eq!(m.bus.frames, end_frame);
                        assert!(viewer_switched);
                        for (path, bytes) in &original_files {
                            assert_eq!(
                                fs::read(path).unwrap(),
                                *bytes,
                                "viewing does not change replay files"
                            );
                        }
                        assert_eq!(
                            COUNTERS.with(|c| c.borrow().calls),
                            saved_calls,
                            "diagnostic native counts restored"
                        );
                        assert_eq!(
                            serde_json::to_vec(m).unwrap(),
                            saved,
                            "natural end restores exact live machine"
                        );
                        phase.set(3);
                        Some(Keycode::F4)
                    }
                    3 if ui.replays.visible => {
                        phase.set(4);
                        Some(Keycode::Return)
                    }
                    4 if ui.replays.playing && m.bus.frames >= end_frame - 60 => {
                        phase.set(5);
                        Some(Keycode::Escape)
                    }
                    5 if !ui.replays.playing => {
                        assert_eq!(
                            serde_json::to_vec(m).unwrap(),
                            saved,
                            "Escape restores exact live machine"
                        );
                        phase.set(6);
                        Some(Keycode::F3)
                    }
                    6 if ui.replays.playing && m.bus.frames >= end_frame - 60 => {
                        phase.set(7);
                        Some(Keycode::Return)
                    }
                    7 if !ui.replays.playing => {
                        assert_eq!(
                            serde_json::to_vec(m).unwrap(),
                            saved,
                            "Enter restores exact live machine"
                        );
                        phase.set(8);
                        Some(Keycode::F3)
                    }
                    8 if ui.replays.playing && m.bus.frames >= end_frame - 60 => {
                        phase.set(9);
                        ui.push_test_quit();
                        None
                    }
                    _ => None,
                };
                if let Some(key) = key {
                    ui.push_test_event(Event::KeyDown {
                        timestamp: 0,
                        window_id: 0,
                        keycode: Some(key),
                        scancode: Scancode::from_keycode(key),
                        keymod: Mod::NOMOD,
                        repeat: false,
                    });
                }
            }))
        });
        let result = run_with_args(args);
        REPLAY_TEST_DRIVER.with(|driver| driver.borrow_mut().take());
        REPLAY_TEST_FAST.with(|fast| fast.set(false));
        result.unwrap();
        assert_eq!(
            phases.get(),
            9,
            "full recording/playback/browser/abort sequence completed"
        );
        assert_eq!(replay::list(&root.join("replays")).unwrap().len(), 1);
        assert_eq!(
            fs::read(&save).unwrap(),
            fs::read(fixture.join("initial.sav")).unwrap(),
            "replay never commits its own save"
        );
        let frames = fs::read_to_string(root.join("session/frames.csv")).unwrap();
        let numbers: Vec<u64> = frames
            .lines()
            .skip(1)
            .map(|l| l.split(',').next().unwrap().parse().unwrap())
            .collect();
        assert!(
            numbers.windows(2).all(|p| p[1] == p[0] + 1),
            "diagnostic session excludes replay frames"
        );
        let profile = fs::read_to_string(root.join("session/frame-profile.csv")).unwrap();
        let profile_frames: Vec<u64> = profile
            .lines()
            .skip(1)
            .map(|row| row.split(',').next().unwrap().parse().unwrap())
            .collect();
        assert_eq!(
            profile_frames, numbers,
            "frame-time diagnostics exclude playback frames too"
        );
        assert!(profile.lines().next().unwrap().contains("audio_underruns"));

        println!("Replay integration evidence: {}", root.display());
    }
}
