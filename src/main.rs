//! Desktop GTA Advance runtime and deterministic replay tools for local AOT libraries.
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
mod camera_preview;
mod camera_trace;
mod color_effects;
mod control_context;
#[cfg(feature = "desktop")]
mod controls;
#[cfg(any(feature = "desktop", test))]
mod coverage;
#[cfg(feature = "desktop")]
mod desktop;
mod frame_capture;
#[cfg(feature = "desktop")]
mod gamepad;
mod ground_projection;
mod hud_layout;
mod language_font;
mod localization;
mod mouse_aim;
mod native_audit;
mod native_host;
mod object_projection;
#[cfg(feature = "desktop")]
mod options;
mod rect_windows;
mod render_settings;
mod save;
mod scene_replay;
mod session;
mod viewport;
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
    AUDIT.with(|a| a.borrow_mut().record(reason, m));
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
    for line in lines.filter(|l| !l.trim().is_empty() && !l.starts_with("# language ")) {
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
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<_> = std::env::args().skip(1).collect();
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
            "Usage: gta-advance ROM INPUT|- FRAMES HASHES.csv|- interp|single|multi|play [LIBRARY ...] [--save PATH] [--controls PATH]"
                .into(),
        );
    }
    let mode = args[4].as_str();
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
    let keys = if args[1] == "-" && mode == "play" {
        Vec::new()
    } else {
        input(&args[1])?
    };
    let recorded_language_changes = if args[1] == "-" {
        Vec::new()
    } else {
        localization::changes(std::path::Path::new(&args[1]))?
    };
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
    let rom = fs::read(&args[0])?;
    if rom.len() != 16 * 1024 * 1024 || &rom[0xac..0xb0] != b"BGTE" {
        return Err("Expected GTA Advance USA ROM".into());
    }
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
    if let Some(language) = language.as_mut() {
        language.install(&mut m);
        println!(
            "Language pack: {} | translated={} | CustomFont={}",
            language.label, language.translated, language.extended
        );
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
    if let (Some(recording), Some(language)) = (recording.as_ref(), language.as_ref()) {
        recording.language_pack(&language.bytes)?;
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
        read32: mouse_aim::read32,
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
    let start = std::time::Instant::now();
    while m.bus.frames < frames {
        mouse_aim::set(&m.bus, None);
        let before = m.bus.frames;
        while language_changes
            .peek()
            .is_some_and(|(frame, _)| *frame == before)
        {
            let (_, bytes) = language_changes.next().unwrap();
            localization::change_live(&mut m, &rom, &mut language, bytes.clone())?;
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
            let Some(keys) = ui.poll(&mut render_settings, preview_enabled, control_context)?
            else {
                break;
            };
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
                        if let Some(recording) = recording.as_mut() {
                            recording.language_change(before, bytes.as_deref())?;
                        }
                        Ok(())
                    });
                match result {
                    Ok(()) => {
                        active_language_file = selected;
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
        if m.bus.frames == before {
            return Err("No frame progress".into());
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
        if let Some(save) = save.as_mut() {
            save.update(&m, false)?;
        }
        #[cfg(feature = "desktop")]
        let preview_image = camera_preview
            .as_mut()
            .and_then(|p| p.frame_configured(&m.bus, render_settings));
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
            ui.present(&m.bus, preview_image.as_deref(), textures.as_mut().unwrap())?;
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
            for sample in &m.bus.audio_buf {
                audio.write_all(&sample.to_le_bytes())?;
            }
        }
        m.bus.audio_buf.clear();
    }
    if let Some(audio) = audio_output.as_mut() {
        use std::io::Write;
        audio.flush()?;
    }
    if let Some(trace) = state_trace.as_mut() {
        use std::io::Write;
        trace.flush()?;
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
