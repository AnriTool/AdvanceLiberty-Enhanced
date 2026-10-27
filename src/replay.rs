//! User replays: a portable machine snapshot plus resolved per-frame controls.
//! Cartridge bytes are excluded; ROM differences contain only runtime patches.
use crate::{
    camera_preview, drop_lifetime, enhanced_audio, localization, render_settings, rom_profile,
};
use gba_core::Machine;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const VERSION: u32 = 1;
const MAX_FRAMES: usize = 3600 * 16_777_216 / 280_896; // One hour, bounded memory and disk usage.
const MAX_FILE: u64 = 128 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
pub struct Initial {
    pub machine: Machine,
    patches: Vec<(usize, Vec<u8>)>,
    tail: Vec<u8>,
    pub audio: Option<enhanced_audio::Output>,
    pub drops: Option<drop_lifetime::Policy>,
    pub language: Option<localization::Language>,
    pub settings: render_settings::Settings,
    pub preserve_cash: bool,
    pub drop_start: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub enum LanguageChange {
    Original,
    Pack(Vec<u8>),
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Frame {
    pub keys: u16,
    pub aim: Option<u16>,
    pub aim_presentation: bool,
    pub camera: u8,
    pub settings: render_settings::Settings,
    pub language: Vec<LanguageChange>,
    pub checksum: u64,
}
#[derive(Serialize, Deserialize)]
struct Info {
    version: u32,
    runtime: String,
    rom: String,
    created: u64,
    frames: usize,
    initial_sha256: String,
    input_sha256: String,
}
#[derive(Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub label: String,
}
pub struct Recording {
    initial: Vec<u8>,
    frames: Vec<Frame>,
}
pub struct Playback {
    pub frames: Vec<Frame>,
    pub cursor: usize,
}
/// The live objects are moved here, never reconstructed or written to disk.
pub struct Live {
    pub machine: Machine,
    pub audio: Option<enhanced_audio::Output>,
    pub drops: Option<drop_lifetime::Policy>,
    pub language: Option<localization::Language>,
    pub preview: Option<camera_preview::Preview>,
    pub settings: render_settings::Settings,
    pub preserve_cash: bool,
    pub drop_start: u64,
}
#[derive(Clone)]
pub enum Command {
    Record,
    Latest,
    Browse,
    Play(PathBuf),
    Stop,
}
pub fn checksum(m: &Machine) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in m
        .bus
        .ewram
        .iter()
        .chain(&m.bus.iwram)
        .copied()
        .chain(m.cpu.regs.iter().flat_map(|v| v.to_le_bytes()))
        .chain(m.cpu.cpsr.to_le_bytes())
        .chain(m.bus.clock.to_le_bytes())
        .chain(m.bus.framebuffer.iter().flat_map(|v| v.to_le_bytes()))
    {
        hash = (hash ^ byte as u64).wrapping_mul(0x100000001b3);
    }
    hash
}
fn bounded(path: &Path) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    fs::File::open(path)?
        .take(MAX_FILE + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > MAX_FILE {
        return Err("Replay file is too large".into());
    }
    Ok(data)
}
impl Recording {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        m: &Machine,
        rom: &[u8],
        audio: &Option<enhanced_audio::Output>,
        drops: &Option<drop_lifetime::Policy>,
        language: &Option<localization::Language>,
        settings: render_settings::Settings,
        preserve_cash: bool,
        drop_start: u64,
    ) -> Result<Self> {
        let machine = m.try_fork_for_research()?;
        machine.bus.validate_snapshot(rom.len())?;
        if machine.bus.rom.len() < rom.len() {
            return Err("Invalid runtime ROM".into());
        }
        let mut patches = Vec::new();
        let mut i = 0;
        while i < rom.len() {
            if rom[i] == machine.bus.rom[i] {
                i += 1;
                continue;
            }
            let begin = i;
            while i < rom.len() && rom[i] != machine.bus.rom[i] {
                i += 1;
            }
            patches.push((begin, machine.bus.rom[begin..i].to_vec()));
        }
        // Serialize host DSP and policy through references to avoid cloning their buffers.
        #[derive(Serialize)]
        struct Capture<'a> {
            machine: &'a Machine,
            patches: Vec<(usize, Vec<u8>)>,
            tail: &'a [u8],
            audio: &'a Option<enhanced_audio::Output>,
            drops: &'a Option<drop_lifetime::Policy>,
            language: Option<&'a localization::Language>,
            settings: render_settings::Settings,
            preserve_cash: bool,
            drop_start: u64,
        }
        let capture = Capture {
            machine: &machine,
            patches,
            tail: &machine.bus.rom[rom.len()..],
            audio,
            drops,
            language: language.as_ref(),
            settings,
            preserve_cash,
            drop_start,
        };
        Ok(Self {
            initial: serde_json::to_vec(&capture)?,
            frames: Vec::new(),
        })
    }
    pub fn push(&mut self, frame: Frame) {
        self.frames.push(frame);
    }
    pub fn full(&self) -> bool {
        self.frames.len() >= MAX_FRAMES
    }
    pub fn save(self, root: &Path, rom: &[u8]) -> Result<Option<PathBuf>> {
        if self.frames.is_empty() {
            return Ok(None);
        }
        fs::create_dir_all(root)?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let name = format!("replay-{}-{:09}", now.as_secs(), now.subsec_nanos());
        let staging = root.join(format!(".{name}.tmp"));
        fs::create_dir(&staging)?;
        let result = (|| {
            let input = serde_json::to_vec(&self.frames)?;
            if self.initial.len() as u64 > MAX_FILE || input.len() as u64 > MAX_FILE {
                return Err("Replay exceeds the file size limit".into());
            }
            let info = Info {
                version: VERSION,
                runtime: env!("CARGO_PKG_VERSION").into(),
                rom: rom_profile::digest(rom),
                created: now.as_secs(),
                frames: self.frames.len(),
                initial_sha256: rom_profile::digest(&self.initial),
                input_sha256: rom_profile::digest(&input),
            };
            for (name, bytes) in [
                ("initial.json", self.initial),
                ("input.json", input),
                ("info.json", serde_json::to_vec_pretty(&info)?),
            ] {
                let mut f = fs::File::create_new(staging.join(name))?;
                f.write_all(&bytes)?;
                f.sync_all()?;
            }
            let destination = root.join(name);
            fs::rename(&staging, &destination)?;
            Ok(Some(destination))
        })();
        if result.is_err() {
            let _ = fs::remove_dir_all(&staging);
        }
        result
    }
}
fn timestamp(seconds: u64) -> String {
    let seconds = seconds.min(253_402_300_799); // Through year 9999, bounded even for damaged metadata.
    let mut days = seconds / 86400;
    let mut year = 1970;
    let leap = |year| year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    loop {
        let count = if leap(year) { 366 } else { 365 };
        if days < count {
            break;
        }
        days -= count;
        year += 1;
    }
    let months = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 0;
    while days >= months[month] {
        days -= months[month];
        month += 1;
    }
    format!(
        "{year:04}-{:02}-{:02} {:02}:{:02}",
        month + 1,
        days + 1,
        seconds / 3600 % 24,
        seconds / 60 % 60
    )
}
pub fn list(root: &Path) -> Result<Vec<Entry>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    for dir in fs::read_dir(root)? {
        let dir = dir?;
        if !dir.file_type()?.is_dir() || !dir.file_name().to_string_lossy().starts_with("replay-") {
            continue;
        }
        let Ok(data) = bounded(&dir.path().join("info.json")) else {
            continue;
        };
        let Ok(info) = serde_json::from_slice::<Info>(&data) else {
            continue;
        };
        if info.version != VERSION || info.frames == 0 || info.frames > MAX_FRAMES {
            continue;
        }
        let seconds = info.frames as f64 * 280896.0 / 16777216.0;
        found.push((
            info.created,
            Entry {
                path: dir.path(),
                label: format!(
                    "{}   {:02}:{:02}",
                    timestamp(info.created),
                    seconds as u64 / 60,
                    seconds as u64 % 60
                ),
            },
        ));
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.path.cmp(&a.1.path)));
    Ok(found.into_iter().map(|(_, entry)| entry).collect())
}
pub fn load(path: &Path, rom: &[u8]) -> Result<(Initial, Playback)> {
    let info: Info = serde_json::from_slice(&bounded(&path.join("info.json"))?)?;
    if info.version != VERSION
        || info.runtime != env!("CARGO_PKG_VERSION")
        || info.rom != rom_profile::digest(rom)
        || info.frames == 0
        || info.frames > MAX_FRAMES
    {
        return Err("Incompatible replay version or ROM".into());
    }
    let bytes = bounded(&path.join("initial.json"))?;
    let input = bounded(&path.join("input.json"))?;
    if rom_profile::digest(&bytes) != info.initial_sha256
        || rom_profile::digest(&input) != info.input_sha256
    {
        return Err("Replay is incomplete or damaged".into());
    }
    let mut initial: Initial = serde_json::from_slice(&bytes)?;
    let frames: Vec<Frame> = serde_json::from_slice(&input)?;
    if frames.len() != info.frames
        || frames
            .iter()
            .any(|f| f.keys > 0x3ff || f.camera > 3 || f.aim.is_some_and(|a| a >= 1024))
    {
        return Err("Invalid replay controls".into());
    }
    initial.machine.bus.rom = rom.to_vec().into();
    let mut previous = 0;
    for (offset, patch) in &initial.patches {
        let end = offset
            .checked_add(patch.len())
            .filter(|&end| end <= rom.len())
            .ok_or("Invalid replay ROM patch")?;
        if *offset < previous {
            return Err("Overlapping replay ROM patches".into());
        }
        initial.machine.bus.rom[*offset..end].copy_from_slice(patch);
        previous = end;
    }
    initial.machine.bus.rom.extend_from_slice(&initial.tail);
    initial.machine.bus.validate_snapshot(rom.len())?;
    if initial.audio.as_ref().is_some_and(|a| !a.valid_snapshot()) {
        return Err("Invalid replay audio state".into());
    }
    Ok((initial, Playback { frames, cursor: 0 }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gba_core::{cpu::Mode, Bus};
    fn root(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "replay-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    fn fixture() -> (Vec<u8>, Machine) {
        let mut rom = vec![0; 0x1000];
        rom[..8].copy_from_slice(&[0x01, 0x00, 0x80, 0xe2, 0xfd, 0xff, 0xff, 0xea]); // add r0,1; b start
        rom[0x100..0x108].copy_from_slice(b"EEPROM_V");
        let mut m = Machine::new(rom.clone());
        m.cpu.regs[15] = 0x08000000;
        m.cpu.cpsr = 0x1f;
        m.cpu.set_cpsr(Mode::Irq as u32);
        m.cpu.regs[13] = 0x03007770;
        m.cpu.set_spsr(0x6000001f);
        m.cpu.set_cpsr(Mode::Sys as u32);
        m.cpu.regs[13] = 0x03007000;
        m.bus.tap_channels = true;
        m.bus.write16(0x04000000, 0x403); // Mode 3
        m.bus.write16(0x06000000, 0x7fff);
        m.bus.write16(0x04000100, 0xff00);
        m.bus.write16(0x04000102, 0x80);
        m.bus.write8(0x04000084, 0x80);
        m.bus.write16(0x04000082, 0x0304);
        m.bus.write32(0x040000a0, 0x10f040e0);
        (rom, m)
    }
    #[test]
    fn snapshot_roundtrip_preserves_banks_devices_audio_and_rom_deltas() {
        let (rom, mut m) = fixture();
        let mut audio = Some(enhanced_audio::Output::new());
        for _ in 0..3 {
            m.run_frame(5_000_000);
            audio.as_mut().unwrap().frame(&mut m.bus);
            m.bus.audio_buf.clear();
        }
        m.bus.rom[0x900..0x904].copy_from_slice(b"text");
        m.bus
            .rom
            .extend_from_slice(b"custom font and translated text");
        let original = serde_json::to_vec(&m).unwrap();
        let mut clip = Recording::start(
            &m,
            &rom,
            &audio,
            &Some(drop_lifetime::Policy::new(30)),
            &None,
            Default::default(),
            true,
            0,
        )
        .unwrap();
        let frame = Frame {
            keys: 0x3ff,
            aim: None,
            aim_presentation: true,
            camera: 0,
            settings: Default::default(),
            language: Vec::new(),
            checksum: checksum(&m),
        };
        clip.push(frame);
        let root = root("roundtrip");
        let path = clip.save(&root, &rom).unwrap().unwrap();
        assert_eq!(list(&root).unwrap().len(), 1);
        let (mut loaded, player) = load(&path, &rom).unwrap();
        assert_eq!(player.frames.len(), 1);
        assert_eq!(serde_json::to_vec(&loaded.machine).unwrap(), original);
        assert_eq!(loaded.machine.bus.rom, m.bus.rom);
        assert_eq!(
            serde_json::to_vec(&loaded.audio).unwrap(),
            serde_json::to_vec(&audio).unwrap()
        );
        let encoded: serde_json::Value =
            serde_json::from_slice(&fs::read(path.join("initial.json")).unwrap()).unwrap();
        assert!(encoded["machine"]["bus"].get("rom").is_none());
        for _ in 0..12 {
            for machine in [&mut m, &mut loaded.machine] {
                machine.bus.keys = 0x3ef;
                machine.run_frame(5_000_000);
            }
            assert_eq!(
                serde_json::to_vec(&m).unwrap(),
                serde_json::to_vec(&loaded.machine).unwrap()
            );
            assert_eq!(
                audio.as_mut().unwrap().frame(&mut m.bus),
                loaded
                    .audio
                    .as_mut()
                    .unwrap()
                    .frame(&mut loaded.machine.bus)
            );
            m.bus.audio_buf.clear();
            loaded.machine.bus.audio_buf.clear();
        }
        let mut other_rom = rom.clone();
        other_rom[999] ^= 1;
        assert!(load(&path, &other_rom).is_err());
        fs::write(path.join("input.json"), b"[]").unwrap();
        assert!(load(&path, &rom).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn incomplete_and_empty_clips_are_not_listed() {
        let (rom, m) = fixture();
        let root = root("empty");
        let clip =
            Recording::start(&m, &rom, &None, &None, &None, Default::default(), false, 0).unwrap();
        assert!(clip.save(&root, &rom).unwrap().is_none());
        assert!(!root.exists());
        fs::create_dir_all(root.join(".replay-interrupted.tmp")).unwrap();
        fs::create_dir_all(root.join("replay-broken")).unwrap();
        fs::write(root.join("replay-broken/info.json"), b"{").unwrap();
        assert!(list(&root).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn invalid_device_buffers_are_rejected_before_native_execution() {
        let (rom, mut m) = fixture();
        m.bus.validate_snapshot(rom.len()).unwrap();
        m.bus.ewram.pop();
        assert!(m.bus.validate_snapshot(rom.len()).is_err());
    }
}

#[cfg(test)]
mod language_replay_tests {
    use super::*;
    #[test]
    #[ignore = "requires local USA ROM"]
    fn snapshots_retain_inflight_fonts_after_switching_to_original_language() {
        let rom = fs::read("roms/usa.gba").unwrap();
        let mut machine = Machine::new(rom.clone());
        let mut language = None;
        let pack = br####"{"version":2,"language":"xx","name":"Example","alphabet":"Q","font":{"width":3,"height":3,"glyphs":{"Q":["###","#.#","###"]}},"entries":{"ui.015b":"Q"}}"####.to_vec();
        localization::change_live(&mut machine, &rom, &mut language, Some(pack)).unwrap();
        localization::change_live(&mut machine, &rom, &mut language, None).unwrap();
        assert!(language.as_ref().unwrap().extended);
        let mut recording = Recording::start(
            &machine,
            &rom,
            &None,
            &None,
            &language,
            Default::default(),
            false,
            0,
        )
        .unwrap();
        recording.push(Frame {
            keys: 0x3ff,
            aim: None,
            aim_presentation: true,
            camera: 0,
            settings: Default::default(),
            language: Vec::new(),
            checksum: checksum(&machine),
        });
        let root = std::env::temp_dir().join(format!(
            "replay-font-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = recording.save(&root, &rom).unwrap().unwrap();
        let (loaded, _) = load(&path, &rom).unwrap();
        assert_eq!(loaded.machine.bus.rom, machine.bus.rom);
        assert_eq!(
            serde_json::to_value(&loaded.language).unwrap(),
            serde_json::to_value(&language).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
