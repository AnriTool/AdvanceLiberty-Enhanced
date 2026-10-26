//! Portable EXE entry point. The same executable runs the native worker;
//! no shell, script interpreter, compiler or development checkout is needed.
use serde::Deserialize;
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    process::Command,
};

pub const TITLE: &str = "AdvanceLiberty: Enhanced";
#[derive(Default, Debug)]
struct Options {
    rom: Option<PathBuf>,
    original_audio: bool,
    no_record: bool,
    frames: u64,
    drop_lifetime: u32,
}
#[derive(Deserialize)]
struct Manifest {
    format: u32,
    rom_sha256: String,
    libraries: Vec<Library>,
}
#[derive(Deserialize)]
struct Library {
    path: String,
    sha256: String,
}

pub fn is_portable(args: &[String]) -> bool {
    // Preserve the complete developer/replay CLI, including all existing tools.
    !(args.len() >= 5 && matches!(args[4].as_str(), "interp" | "single" | "multi" | "play"))
}
fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        drop_lifetime: 30,
        ..Default::default()
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--original-audio" => options.original_audio = true,
            "--no-record" => options.no_record = true,
            "--rom" | "--frames" | "--drop-lifetime" => {
                let key = &args[i];
                i += 1;
                let value = args
                    .get(i)
                    .ok_or_else(|| format!("{key} requires a value"))?;
                match key.as_str() {
                    "--rom" => options.rom = Some(PathBuf::from(value)),
                    "--frames" => {
                        options.frames = value.parse().map_err(|_| "Invalid frame count")?
                    }
                    _ => {
                        options.drop_lifetime =
                            value.parse().map_err(|_| "Invalid drop lifetime")?;
                        if options.drop_lifetime > 300 {
                            return Err("Drop lifetime must be 0..300 seconds".into());
                        }
                    }
                }
            }
            value
                if args.len() == 1
                    && Path::new(value)
                        .extension()
                        .is_some_and(|s| s.eq_ignore_ascii_case("gba")) =>
            {
                options.rom = Some(PathBuf::from(value)); // Drag and drop a ROM onto the EXE.
            }
            other => {
                return Err(format!(
                    "Unknown option: {other}\nUse --help for launch options."
                ))
            }
        }
        i += 1;
    }
    Ok(options)
}
fn profile(root: &Path) -> Result<Vec<PathBuf>, String> {
    let path = root.join("native/usa/manifest.json");
    let bytes = fs::read(&path).map_err(|e| {
        format!(
            "Cannot read {}: {e}\nExtract the complete release folder.",
            path.display()
        )
    })?;
    let manifest: Manifest =
        serde_json::from_slice(&bytes).map_err(|e| format!("Invalid native manifest: {e}"))?;
    if manifest.format != 1
        || manifest.rom_sha256 != crate::rom_profile::USA
        || manifest.libraries.is_empty()
    {
        return Err("The native profile does not match this release.".into());
    }
    manifest
        .libraries
        .into_iter()
        .map(|library| {
            let relative = Path::new(&library.path);
            if !relative
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
                || !relative.starts_with("native/usa")
                || relative.extension() != Some(std::ffi::OsStr::new("dll"))
            {
                return Err("Invalid native library path".into());
            }
            let path = root.join(relative);
            let bytes = fs::read(&path)
                .map_err(|e| format!("Missing native library {}: {e}", path.display()))?;
            if crate::rom_profile::digest(&bytes) != library.sha256 {
                return Err(format!(
                    "Native library checksum mismatch: {}\nExtract the complete release again.",
                    path.display()
                ));
            }
            Ok(path)
        })
        .collect()
}
fn find_rom(root: &Path, explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        let path = fs::canonicalize(path)
            .map_err(|e| format!("Cannot open ROM {}: {e}", path.display()))?;
        crate::rom_profile::verify(&fs::read(&path).map_err(|e| e.to_string())?)?;
        return Ok(path);
    }
    let mut paths = Vec::new();
    for dir in [root.join("roms"), root.to_path_buf()] {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries {
                let path = entry.map_err(|e| e.to_string())?.path();
                if path.is_file()
                    && path
                        .extension()
                        .is_some_and(|s| s.eq_ignore_ascii_case("gba"))
                {
                    paths.push(path);
                }
            }
        }
    }
    paths.sort();
    let mut unsupported = Vec::new();
    for path in paths {
        let bytes = fs::read(&path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
        if crate::rom_profile::verify(&bytes).is_ok() {
            return Ok(path);
        }
        unsupported.push(path.file_name().unwrap().to_string_lossy().into_owned());
    }
    if !unsupported.is_empty() {
        return Err(format!("No supported ROM found.\nThese files are incompatible: {}\nUse your unmodified USA dump.\nExpected SHA-256: {}", unsupported.join(", "), crate::rom_profile::USA));
    }
    Err(format!("Your cartridge ROM is required.\n\nPlace your legally obtained, unmodified USA .gba dump in:\n{}\n\nKeep any filename, then start AdvanceLiberty.exe again.\nYou can also drag a .gba file onto the EXE.\n\nNo ROM is included or downloaded by this application.", root.join("roms").display()))
}
pub fn launch(args: &[String]) -> Result<(), String> {
    if args == ["--version"] {
        println!("{TITLE} {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args == ["--help"] {
        let help = format!("{TITLE}\n\nPlace your USA .gba dump in roms/ and run AdvanceLiberty.exe.\n\nOptional arguments:\n--rom FILE\n--original-audio\n--drop-lifetime 0..300 (default: 30)\n--no-record\n--frames N (diagnostics)\n\nEnhanced Audio and session recording are enabled by default.");
        println!("{help}");
        let _ = sdl2::messagebox::show_simple_message_box(
            sdl2::messagebox::MessageBoxFlag::INFORMATION,
            TITLE,
            &help,
            None,
        );
        return Ok(());
    }
    let options = parse(args)?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let root = exe
        .parent()
        .ok_or("Cannot locate the installation directory")?;
    let rom = find_rom(root, options.rom.as_deref())?;
    let libraries = profile(root)?;
    for dir in [
        "data/saves",
        "data/config",
        "data/logs",
        "data/sessions",
        "languages",
    ] {
        fs::create_dir_all(root.join(dir)).map_err(|e| {
            format!("Cannot write to the game folder: {e}\nExtract it to a writable location.")
        })?;
    }
    let id = format!(
        "{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos(),
        std::process::id()
    );
    let log_path = root.join("data/logs").join(format!("launch-{id}.log"));
    let mut log = fs::File::create_new(&log_path).map_err(|e| e.to_string())?;
    writeln!(
        log,
        "{TITLE} {}\nROM: {}",
        env!("CARGO_PKG_VERSION"),
        rom.display()
    )
    .map_err(|e| e.to_string())?;
    let mut child = Command::new(&exe);
    child
        .current_dir(root)
        .arg(rom)
        .arg("-")
        .arg(options.frames.to_string())
        .arg("-")
        .arg("play");
    child
        .args(libraries)
        .args(["--camera-preview", "--save"])
        .arg(root.join("data/saves/usa.sav"))
        .arg("--controls")
        .arg(root.join("data/config/controls.cfg"));
    if !options.original_audio {
        child.arg("--enhanced-audio");
    }
    if options.drop_lifetime != 0 {
        child
            .arg("--drop-lifetime")
            .arg(options.drop_lifetime.to_string());
    }
    if !options.no_record {
        child
            .arg("--record-session")
            .arg(root.join("data/sessions").join(format!("session-{id}")));
    }
    child
        .stdout(log.try_clone().map_err(|e| e.to_string())?)
        .stderr(log);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        child.creation_flags(0x08000000);
    }
    let status = child
        .status()
        .map_err(|e| format!("Cannot start the game: {e}"))?;
    if !status.success() {
        let text = fs::read_to_string(&log_path).unwrap_or_default();
        let tail: Vec<_> = text.lines().rev().take(8).collect();
        return Err(format!(
            "The game exited with {status}.\n\n{}\n\nLog: {}",
            tail.into_iter().rev().collect::<Vec<_>>().join("\n"),
            log_path.display()
        ));
    }
    Ok(())
}
pub fn show_error(message: &str) {
    if std::env::var_os("ADVANCE_LIBERTY_NO_DIALOGS").is_some() {
        return;
    }
    let _ = sdl2::messagebox::show_simple_message_box(
        sdl2::messagebox::MessageBoxFlag::ERROR,
        TITLE,
        message,
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }
    #[test]
    fn launch_defaults_overrides_and_worker_detection() {
        let defaults = parse(&[]).unwrap();
        assert_eq!(defaults.drop_lifetime, 30);
        assert!(!defaults.original_audio && !defaults.no_record);
        let options = parse(&args(&[
            "--rom",
            "my copy.gba",
            "--original-audio",
            "--drop-lifetime",
            "0",
            "--no-record",
            "--frames",
            "120",
        ]))
        .unwrap();
        assert_eq!(options.drop_lifetime, 0);
        assert!(options.original_audio && options.no_record);
        assert_eq!(options.frames, 120);
        assert!(is_portable(&[]));
        assert!(!is_portable(&args(&["rom", "-", "0", "-", "play", "lib"])));
        assert!(parse(&args(&["--drop-lifetime", "301"])).is_err());
        assert!(parse(&args(&["--rom"])).is_err());
        assert!(parse(&args(&["--bad"])).is_err());
        assert_eq!(
            parse(&args(&["dropped.gba"])).unwrap().rom.unwrap(),
            Path::new("dropped.gba")
        );
    }
    #[test]
    fn missing_or_modified_rom_is_rejected_without_creating_a_save() {
        let root =
            std::env::temp_dir().join(format!("advance-liberty-launch-{}", std::process::id()));
        fs::create_dir_all(root.join("roms")).unwrap();
        assert!(find_rom(&root, None)
            .unwrap_err()
            .contains("No ROM is included"));
        fs::write(root.join("roms/modified.gba"), b"not a cartridge").unwrap();
        assert!(find_rom(&root, None).unwrap_err().contains("incompatible"));
        assert!(!root.join("data/saves").exists());
    }
    #[test]
    fn native_profile_validates_hashes_and_rejects_path_escape() {
        let root =
            std::env::temp_dir().join(format!("advance-liberty-profile-{}", std::process::id()));
        fs::create_dir_all(root.join("native/usa")).unwrap();
        let dll = root.join("native/usa/test.dll");
        fs::write(&dll, b"fixture").unwrap();
        let mut manifest = serde_json::json!({"format":1,"rom_sha256":crate::rom_profile::USA,"libraries":[{"path":"native/usa/test.dll","sha256":crate::rom_profile::digest(b"fixture")}]});
        let write = |value: &serde_json::Value| {
            fs::write(
                root.join("native/usa/manifest.json"),
                serde_json::to_vec(value).unwrap(),
            )
            .unwrap()
        };
        write(&manifest);
        assert_eq!(profile(&root).unwrap(), vec![dll.clone()]);
        fs::write(&dll, b"changed").unwrap();
        assert!(profile(&root).unwrap_err().contains("checksum"));
        manifest["libraries"][0]["path"] = "native/usa/../../../other.dll".into();
        write(&manifest);
        assert!(profile(&root).unwrap_err().contains("path"));
    }
}
