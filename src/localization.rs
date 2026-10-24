//! Opt-in external text data for the verified USA image. No guest code patches.
pub use crate::language_font::{Definition as FontDefinition, Font};
use gba_core::Machine;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    ffi::c_void,
    io::{self, BufWriter, Write},
};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub file: String,
    pub label: String,
    pub font: Font,
}

pub fn valid_file_name(file: &str) -> bool {
    !file.is_empty()
        && file.trim() == file
        && !file.chars().any(|c| c.is_control() || "/\\:#=".contains(c))
        && file.to_ascii_lowercase().ends_with(".json")
        && Path::new(file).components().count() == 1
}

/// Portable installs keep languages beside the exe; Cargo installs keep it
/// beside target/. Never depend on the caller's current working directory.
pub fn directory(executable: &Path) -> PathBuf {
    let parent = executable.parent().unwrap_or(Path::new("."));
    if matches!(
        parent.file_name().and_then(|s| s.to_str()),
        Some("release" | "debug")
    ) && parent
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|s| s == "target")
    {
        return parent.parent().unwrap().parent().unwrap().join("languages");
    }
    parent.join("languages")
}

/// Export editable English once. Never replace a translator's existing file,
/// even when it is incomplete or no longer valid JSON.
pub fn ensure_english(
    directory: &Path,
    rom: &[u8],
) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let path = directory.join("en.json");
    if path.try_exists()? {
        return Ok(None);
    }
    if format!("{:x}", Sha256::digest(rom)) != USA {
        return Err("English export requires the verified USA ROM".into());
    }
    let tables = tables(rom);
    if tables.len() != 1156 {
        return Err("USA language table structure differs".into());
    }
    let entries: BTreeMap<_, _> = tables
        .iter()
        .map(|(id, offset)| {
            let original = string(rom, word(rom, offset + 4)).unwrap();
            let text: String = original.iter().map(|&byte| char::from(byte)).collect();
            (id.clone(), text)
        })
        .collect();
    let pack = serde_json::json!({
        "version": 2, "language": "en", "name": "English (editable)",
        "alphabet": "", "entries": entries
    });
    let mut bytes = serde_json::to_vec_pretty(&pack)?;
    bytes.push(b'\n');
    // The generated pack must pass exactly the same checks as authored packs.
    Language::from_bytes(bytes.clone(), rom)?;
    fs::create_dir_all(directory)?;
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(Some(path))
}

pub fn discover(directory: &Path, rom: &[u8]) -> Vec<Choice> {
    let files = match fs::read_dir(directory) {
        Ok(files) => files,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("Language discovery: {}: {error}", directory.display());
            }
            return Vec::new();
        }
    };
    let mut choices = Vec::new();
    for entry in files {
        let Ok(entry) = entry else {
            continue;
        };
        if !entry.path().is_file() {
            continue;
        }
        let Some(file) = entry
            .file_name()
            .to_str()
            .map(str::to_owned)
            .filter(|s| valid_file_name(s))
        else {
            continue;
        };
        match Language::load(&entry.path(), rom) {
            Ok(language) => choices.push(Choice {
                file,
                label: language.label,
                font: language.font,
            }),
            Err(error) => eprintln!("Language pack skipped: {}: {error}", entry.path().display()),
        }
    }
    choices.sort_by(|a, b| a.label.cmp(&b.label).then_with(|| a.file.cmp(&b.file)));
    choices
}

const USA: &str = "215c574d5afbda0aadbf89dc76b765cfaaeb35953eab63f7687d8bfe8a8e0f4d";
const FONT_GLYPHS: usize = 88;
const FONT_RETURN: u32 = 0x0802de67;
const MAX_FILE: u64 = 4 * 1024 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyPack {
    format: String,
    base_rom_sha256: String,
    language: String,
    name: String,
    fallback: String,
    scope: String,
    #[serde(deserialize_with = "crate::language_font::unique_map")]
    entries: BTreeMap<String, String>,
    #[serde(default)]
    source_rom_sha256: Option<String>,
    #[serde(default)]
    source_slot: Option<String>,
}
fn english() -> String {
    "en".into()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackV2 {
    version: u32,
    language: String,
    name: String,
    #[serde(default = "english")]
    fallback: String,
    #[serde(default)]
    alphabet: String,
    #[serde(default)]
    font: Option<FontDefinition>,
    #[serde(deserialize_with = "crate::language_font::unique_map")]
    entries: BTreeMap<String, String>,
    #[serde(default)]
    source_rom_sha256: Option<String>,
    #[serde(default)]
    source_slot: Option<String>,
}
#[derive(Deserialize)]
struct FontData {
    alphabet: String,
    font: FontDefinition,
}
fn legacy_font() -> Result<Font, Box<dyn std::error::Error>> {
    // Frozen v1 compatibility data. New packages always carry their own font.
    let data: FontData =
        serde_json::from_str(include_str!("../resources/legacy-language-v1-font.json"))?;
    Ok(Font::compile(&data.alphabet, Some(data.font))?)
}
struct Pack {
    language: String,
    name: String,
    fallback: String,
    entries: BTreeMap<String, String>,
    font: Font,
}
fn parse_pack(bytes: &[u8]) -> Result<Pack, Box<dyn std::error::Error>> {
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
    let header: serde_json::Value = serde_json::from_slice(bytes)?;
    if header.get("version").is_some() {
        let pack: PackV2 = serde_json::from_slice(bytes)?;
        if pack.version != 2 {
            return Err("Unsupported language pack version".into());
        }
        let _provenance = (&pack.source_rom_sha256, &pack.source_slot);
        Ok(Pack {
            language: pack.language,
            name: pack.name,
            fallback: pack.fallback,
            entries: pack.entries,
            font: Font::compile(&pack.alphabet, pack.font)?,
        })
    } else {
        let pack: LegacyPack = serde_json::from_slice(bytes)?;
        if pack.format != "gta-advance-language-pack-v1"
            || pack.base_rom_sha256 != USA
            || pack.scope != "confirmed-five-language-tables"
        {
            return Err("Unsupported legacy language pack".into());
        }
        let _provenance = (&pack.source_rom_sha256, &pack.source_slot);
        Ok(Pack {
            language: pack.language,
            name: pack.name,
            fallback: pack.fallback,
            entries: pack.entries,
            font: legacy_font()?,
        })
    }
}

fn word(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}
fn set_word(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
fn string(rom: &[u8], pointer: u32) -> Option<&[u8]> {
    let start = pointer.checked_sub(0x08000000)? as usize;
    let rest = rom.get(start..)?;
    let end = rest.iter().take(4097).position(|&b| b == 0)?;
    Some(&rest[..end])
}
fn tables(rom: &[u8]) -> BTreeMap<String, usize> {
    let mut result = BTreeMap::new();
    for offset in (0..rom.len().saturating_sub(39)).step_by(4) {
        if (0..5).all(|i| {
            word(rom, offset + i * 8) == i as u32
                && string(rom, word(rom, offset + i * 8 + 4)).is_some()
        }) {
            result.insert(format!("text.{offset:08x}"), offset);
        }
    }
    result
}

fn controls(text: &[u8]) -> Vec<Vec<u8>> {
    let mut result = Vec::new();
    let mut i = 0;
    while i + 1 < text.len() {
        if matches!(text[i], b'@' | b'%') && text[i + 1].is_ascii_digit() {
            let start = i;
            i += 2;
            if text[start] == b'%' {
                while i < text.len() && text[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let token = &text[start..i];
            // Original languages use different optional colour emphasis.
            if !matches!(token, b"@0" | b"@8" | b"@9") {
                result.push(token.to_vec());
            }
        } else {
            i += 1;
        }
    }
    result.sort();
    result
}

fn encode(text: &str, font: &Font) -> Result<(Vec<u8>, bool), String> {
    let mut bytes = Vec::new();
    let mut extended = false;
    for character in text.chars() {
        if let Some(code) = font.encode(character) {
            bytes.push(code);
            extended = true;
            continue;
        }
        let code = match character {
            '\n' => 10,
            '’' | '‘' => b'\'',
            '“' | '”' => b'"',
            '—' | '–' => b'-',
            c if c.is_ascii_alphanumeric() || " -_$!?\"',.+/:;()%@".contains(c) => c as u8,
            // Accents explicitly supported by the original 0802dd2c parser.
            'Ü' | 'Ñ' | 'É' | '¿' | 'Á' | 'Ó' | 'È' | 'Í' | '¡' | 'Ö' | 'À' | 'Ù' | 'Ä' | 'Ú'
            | 'Ì' | 'Ò' | 'Ï' => character as u8,
            _ => {
                return Err(format!(
                    "Unsupported character U+{:04X}: {character}",
                    character as u32
                ))
            }
        };
        bytes.push(code);
    }
    for (index, &byte) in bytes.iter().enumerate() {
        if byte == b'@'
            && !bytes
                .get(index + 1)
                .is_some_and(|b| matches!(b, b'0' | b'1' | b'2' | b'3' | b'4' | b'8' | b'9'))
        {
            return Err("Unsupported @ control marker".into());
        }
    }
    // Bound the prototype to the largest verified original dialogue length.
    // Longer authoring strings remain valid JSON but need buffer research.
    if bytes.len() > 609 {
        return Err("Runtime prototype supports at most 609 characters per string".into());
    }
    Ok((bytes, extended))
}

pub struct Language {
    pub bytes: Vec<u8>,
    pub label: String,
    pub translated: usize,
    pub extended: bool,
    pub glyph_calls: u64,
    pub font: Font,
    rom: Vec<u8>,
}
impl Language {
    pub fn load(path: &Path, rom: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        if fs::metadata(path)?.len() > MAX_FILE {
            return Err("Language pack exceeds 4 MiB".into());
        }
        Self::from_bytes(fs::read(path)?, rom)
    }
    fn from_bytes(bytes: Vec<u8>, rom: &[u8]) -> Result<Self, Box<dyn std::error::Error>> {
        if format!("{:x}", Sha256::digest(rom)) != USA {
            return Err("Language packs require the verified USA ROM".into());
        }
        let pack = parse_pack(&bytes)?;
        if pack.fallback != "en" {
            return Err("Only English fallback is supported".into());
        }
        if pack.language.is_empty()
            || pack.language.len() > 32
            || !pack
                .language
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            || pack.name.trim().is_empty()
            || pack.name.chars().any(char::is_control)
        {
            return Err("Invalid language tag or name".into());
        }
        let tables = tables(rom);
        if tables.len() != 1156 {
            return Err("USA language table structure differs".into());
        }
        for id in pack.entries.keys() {
            if !tables.contains_key(id) {
                return Err(format!("Unknown text ID: {id}").into());
            }
        }
        // Work on a private in-memory image. Machine/save detection still runs
        // against the original 16 MiB cartridge before this image is installed.
        let mut image = rom.to_vec();
        let mut translated = 0;
        let mut extended = false;
        for (id, offset) in &tables {
            let english = word(rom, offset + 4);
            let mut pointer = english;
            if let Some(text) = pack.entries.get(id).filter(|t| !t.is_empty()) {
                let (encoded, needs_font) =
                    encode(text, &pack.font).map_err(|error| format!("{id}: {error}"))?;
                let original = string(rom, english).unwrap();
                if controls(&encoded) != controls(original) {
                    return Err(format!("{id}: preserve dynamic @ and % tokens").into());
                }
                extended |= needs_font;
                translated += 1;
                if encoded != original {
                    pointer = 0x08000000 + image.len() as u32;
                    image.extend_from_slice(&encoded);
                    image.push(0);
                }
            }
            // A host language is independent of the cartridge's language menu.
            // All guest slots resolve to the chosen text or English fallback.
            for language in 0..5 {
                set_word(&mut image, offset + language * 8 + 4, pointer);
            }
        }
        if extended {
            install_fonts(&mut image, &pack.font);
        }
        if image.len() > 32 * 1024 * 1024 {
            return Err("Text arena exceeds cartridge address space".into());
        }
        Ok(Self {
            bytes,
            label: format!("{} ({})", pack.name, pack.language),
            translated,
            extended,
            glyph_calls: 0,
            font: pack.font,
            rom: image,
        })
    }
    pub fn install(&mut self, m: &mut Machine) {
        m.bus.rom = std::mem::take(&mut self.rom);
    }
    pub fn observe(&mut self, m: &mut Machine, key: u32) {
        TRACE.with(|cell| {
            if let Some(trace) = cell.borrow_mut().as_mut() {
                if let Err(error) = writeln!(
                    trace.glyphs,
                    "{},{key:08x},{:08x},{:08x},{:08x},{:08x},{:08x}",
                    m.bus.frames,
                    m.cpu.regs[0],
                    m.cpu.regs[1],
                    m.cpu.regs[2],
                    m.cpu.regs[4],
                    m.cpu.regs[14]
                ) {
                    trace.error = Some(error);
                }
            }
        });
        if self.extended && key == FONT_RETURN {
            let byte = m.cpu.regs[1];
            if m.cpu.regs[0] == u32::MAX {
                if let Some(index) = self.font.index(byte) {
                    m.cpu.regs[0] = FONT_GLYPHS as u32 + index as u32;
                    self.glyph_calls += 1;
                }
            }
        }
    }
}

// Preserve 88 tiles per font view, including adjacent reserved/icon tiles,
// and append package-owned tiles. The dialogue font has its own
// colour-remapping path, separate from the antialiased and plain UI fonts.
fn install_fonts(rom: &mut Vec<u8>, custom: &Font) {
    while rom.len() % 4 != 0 {
        rom.push(0);
    }
    for (source, literals, ink) in [
        (0x346478, &[0x2add8][..], 13u8),
        (0x346f78, &[0x4b74, 0x4f3c, 0x601c][..], 3u8),
        // Dialogue compositor recolours nibble 3 to the message colour.
        (0x3478d8, &[0x2e520][..], 3u8),
        (0x346d78, &[0x2e4f0][..], 3u8),
    ] {
        let mut font = rom[source..source + FONT_GLYPHS * 32].to_vec();
        for index in 0..custom.len() {
            font.extend_from_slice(&custom.tile(index, ink));
        }
        let address = 0x08000000 + rom.len() as u32;
        rom.extend_from_slice(&font);
        for &literal in literals {
            assert_eq!(word(rom, literal), 0x08000000 + source as u32);
            set_word(rom, literal, address);
        }
    }
    install_cutscene_font(rom, custom);
}

// Cut-scenes use an independent byte-per-pixel atlas and proportional widths,
// indexed directly by character - 0x20. They never call FONT_RETURN. Extend a
// private copy of that atlas at the transport byte slots, preserving the game's
// ASCII glyphs, palette remapping, compositor and text wrapping.
fn install_cutscene_font(rom: &mut Vec<u8>, custom: &Font) {
    const SOURCE: usize = 0x84da90;
    const WIDTHS: usize = 0x851310;
    const GLYPHS: usize = 256 - 32;
    let mut atlas = rom[SOURCE..SOURCE + GLYPHS * 64].to_vec();
    let mut widths = rom[WIDTHS..WIDTHS + GLYPHS].to_vec();
    for (index, byte) in crate::language_font::private_codes()
        .into_iter()
        .take(custom.len())
        .enumerate()
    {
        let glyph = byte as usize - 32;
        let tile = custom.tile(index, 2);
        for pixel in 0..64 {
            atlas[glyph * 64 + pixel] = (tile[pixel / 2] >> (4 * (pixel % 2))) & 15;
        }
        widths[glyph] = custom.glyph_width();
    }
    assert_eq!(word(rom, 0x642cc), 0x08000000 + SOURCE as u32);
    assert_eq!(word(rom, 0x642d0), 0x08000000 + WIDTHS as u32);
    let address = 0x08000000 + rom.len() as u32;
    rom.extend_from_slice(&atlas);
    rom.extend_from_slice(&widths);
    set_word(rom, 0x642cc, address);
    set_word(rom, 0x642d0, address + atlas.len() as u32);
}

struct Trace {
    output: BufWriter<fs::File>,
    glyphs: BufWriter<fs::File>,
    pointers: BTreeMap<u32, (usize, usize)>,
    last: BTreeMap<u32, u64>,
    error: Option<io::Error>,
}
thread_local! { static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) }; }
pub fn open_trace(path: &Path, rom: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    if format!("{:x}", Sha256::digest(rom)) != USA {
        return Err("Text trace requires the verified USA ROM".into());
    }
    let mut output = BufWriter::new(fs::File::create_new(path)?);
    writeln!(output, "frame,pc,text_id,slot,pointer")?;
    let mut glyphs = BufWriter::new(fs::File::create_new(path.with_extension("glyphs.csv"))?);
    writeln!(glyphs, "frame,pc,r0,r1,r2,r4,lr")?;
    let mut pointers = BTreeMap::new();
    for &offset in tables(rom).values() {
        for slot in 0..5 {
            pointers.insert(0x08000000 + (offset + slot * 8 + 4) as u32, (offset, slot));
        }
    }
    TRACE.with(|t| {
        *t.borrow_mut() = Some(Trace {
            output,
            glyphs,
            pointers,
            last: BTreeMap::new(),
            error: None,
        })
    });
    Ok(())
}
pub extern "C" fn read32(m: *mut c_void, address: u32) -> u32 {
    let value = crate::mouse_aim::read32(m, address);
    if (8..=13).contains(&(address >> 24)) {
        let normalized = 0x08000000 | (address & 0x1ffffff);
        TRACE.with(|cell| {
            let mut trace = cell.borrow_mut();
            let Some(trace) = trace.as_mut() else {
                return;
            };
            let Some(&(offset, slot)) = trace.pointers.get(&normalized) else {
                return;
            };
            let machine = unsafe { &*(m as *const Machine) };
            let frame = machine.bus.frames;
            if trace.last.insert(normalized, frame) != Some(frame) && trace.error.is_none() {
                if let Err(error) = writeln!(
                    trace.output,
                    "{frame},{:08x},text.{offset:08x},{slot},{value:08x}",
                    machine.cpu.regs[15]
                ) {
                    trace.error = Some(error);
                }
            }
        });
    }
    value
}
pub fn close_trace() -> io::Result<()> {
    TRACE.with(|cell| {
        if let Some(mut trace) = cell.borrow_mut().take() {
            if let Some(error) = trace.error {
                return Err(error);
            }
            trace.output.flush()?;
            trace.glyphs.flush()?;
        }
        Ok(())
    })
}

#[cfg(test)]
pub fn test_font() -> Font {
    legacy_font().unwrap()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires the user's USA ROM"]
    fn cutscene_atlas_preserves_original_text_and_covers_all_transport_slots() {
        let rom =
            fs::read(std::env::var_os("GTA_LANGUAGE_TEST_ROM").expect("GTA_LANGUAGE_TEST_ROM"))
                .unwrap();
        let alphabet: String = (0..110)
            .map(|i| char::from_u32(0x400 + i).unwrap())
            .collect();
        let glyphs: BTreeMap<_, _> = alphabet
            .chars()
            .map(|c| (c.to_string(), vec!["###", "#.#", "###"]))
            .collect();
        let definition: FontDefinition =
            serde_json::from_value(serde_json::json!({"width":3,"height":3,"glyphs":glyphs}))
                .unwrap();
        let font = Font::compile(&alphabet, Some(definition)).unwrap();
        let mut image = rom.clone();
        install_fonts(&mut image, &font);
        let atlas = (word(&image, 0x642cc) - 0x08000000) as usize;
        let widths = (word(&image, 0x642d0) - 0x08000000) as usize;
        for byte in 32..=255u8 {
            let slot = byte as usize - 32;
            if font.index(byte as u32).is_none() {
                assert_eq!(
                    &image[atlas + slot * 64..atlas + (slot + 1) * 64],
                    &rom[0x84da90 + slot * 64..0x84da90 + (slot + 1) * 64]
                );
                assert_eq!(image[widths + slot], rom[0x851310 + slot]);
            } else {
                // The native uppercase routine must leave every private byte intact.
                assert_eq!(rom[0xbd3165 + byte as usize] & 2, 0);
                assert_eq!(image[widths + slot], 3);
                assert_eq!(
                    &image[atlas + slot * 64..atlas + slot * 64 + 8],
                    &[2, 2, 2, 0, 0, 0, 0, 0]
                );
                assert_eq!(
                    &image[atlas + slot * 64 + 8..atlas + slot * 64 + 16],
                    &[2, 0, 2, 0, 0, 0, 0, 0]
                );
                assert!(image[atlas + slot * 64 + 24..atlas + (slot + 1) * 64]
                    .iter()
                    .all(|&p| p == 0));
            }
        }
    }
    #[test]
    #[ignore = "requires the user's USA ROM"]
    fn english_export_is_complete_loadable_and_never_overwrites_edits() {
        let rom =
            fs::read(std::env::var_os("GTA_LANGUAGE_TEST_ROM").expect("GTA_LANGUAGE_TEST_ROM"))
                .unwrap();
        let root = std::env::temp_dir().join(format!("english-export-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let directory = root.join("languages");
        let path = ensure_english(&directory, &rom).unwrap().unwrap();
        let bytes = fs::read(&path).unwrap();
        let pack = parse_pack(&bytes).unwrap();
        assert_eq!(pack.entries.len(), 1156);
        assert_eq!(pack.language, "en");
        for (id, offset) in tables(&rom) {
            let original: String = string(&rom, word(&rom, offset + 4))
                .unwrap()
                .iter()
                .map(|&b| char::from(b))
                .collect();
            assert_eq!(pack.entries[&id], original);
        }
        let language = Language::load(&path, &rom).unwrap();
        assert_eq!(language.font.len(), 0);
        assert!(!language.extended);
        assert_eq!(discover(&directory, &rom).len(), 1);
        assert!(ensure_english(&directory, &rom).unwrap().is_none());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        fs::write(&path, b"unfinished translator edits").unwrap();
        assert!(ensure_english(&directory, &rom).unwrap().is_none());
        assert_eq!(fs::read(&path).unwrap(), b"unfinished translator edits");
        let mut wrong = rom.clone();
        wrong[0] ^= 1;
        let other = root.join("wrong-rom");
        assert!(ensure_english(&other, &wrong).is_err());
        assert!(!other.exists());
    }
    #[test]
    #[ignore = "requires the user's USA ROM"]
    fn discover_real_rom_validates_packs_and_ignores_broken_files() {
        let rom =
            fs::read(std::env::var_os("GTA_LANGUAGE_TEST_ROM").expect("GTA_LANGUAGE_TEST_ROM"))
                .unwrap();
        let root = std::env::temp_dir().join(format!("language-discovery-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let pack = serde_json::json!({"version":2,"language":"ru","name":"Русский","entries":{}});
        fs::write(root.join("z.json"), pack.to_string()).unwrap();
        fs::write(root.join("a.JSON"), pack.to_string()).unwrap();
        fs::write(root.join("broken.json"), b"not json").unwrap();
        let mut wrong = pack.clone();
        wrong["version"] = 99.into();
        fs::write(root.join("wrong.json"), wrong.to_string()).unwrap();
        fs::write(root.join("ignored.txt"), pack.to_string()).unwrap();
        fs::create_dir(root.join("nested")).unwrap();
        fs::write(root.join("nested/ignored.json"), pack.to_string()).unwrap();
        let choices = discover(&root, &rom);
        assert_eq!(
            choices.iter().map(|c| c.file.as_str()).collect::<Vec<_>>(),
            ["a.JSON", "z.json"]
        );
        assert!(choices.iter().all(|c| c.label == "Русский (ru)"));
        assert!(discover(&root.join("missing"), &rom).is_empty());
    }
    #[test]
    fn language_directory_is_independent_of_working_directory() {
        assert_eq!(
            directory(Path::new("game/target/release/game.exe")),
            Path::new("game/languages")
        );
        assert_eq!(
            directory(Path::new("portable/game.exe")),
            Path::new("portable/languages")
        );
        assert!(valid_file_name("ru-demo.json"));
        assert!(valid_file_name("Русский.JSON"));
        assert!(!valid_file_name("sub/ru.json"));
        assert!(!valid_file_name("ru.json#bad"));
    }
    #[test]
    fn unicode_codes_are_bounded_and_preserve_lowercase_as_uppercase() {
        let data: FontData =
            serde_json::from_str(include_str!("../resources/legacy-language-v1-font.json"))
                .unwrap();
        let font = Font::compile(&data.alphabet, Some(data.font)).unwrap();
        let (a, extended) = encode(&data.alphabet, &font).unwrap();
        assert!(extended);
        assert_eq!(a, crate::language_font::private_codes()[..33]);
        assert!(!a.contains(&0x92) && !a.contains(&0xa1));
        assert_eq!(
            encode("миссия", &font).unwrap().0,
            encode("МИССИЯ", &font).unwrap().0
        );
        assert!(encode("😀", &font).is_err());
        assert!(encode("a\0b", &font).is_err());
        assert!(encode("A&B", &font).is_err());
        assert!(encode("@5", &font).is_err());
        assert!(encode(&"A".repeat(610), &font).is_err());
    }
    #[test]
    fn glyph_hook_only_extends_verified_character_return() {
        let mut m = Machine::new(vec![0; 4]);
        let mut language = Language {
            bytes: vec![],
            label: String::new(),
            translated: 0,
            extended: true,
            glyph_calls: 0,
            font: test_font(),
            rom: vec![],
        };
        m.cpu.regs[0] = u32::MAX;
        m.cpu.regs[1] = 0x80;
        language.observe(&mut m, FONT_RETURN - 2);
        assert_eq!(m.cpu.regs[0], u32::MAX);
        language.observe(&mut m, FONT_RETURN);
        assert_eq!(m.cpu.regs[0], 88);
        m.cpu.regs[0] = u32::MAX;
        m.cpu.regs[1] = b'A' as u32;
        language.observe(&mut m, FONT_RETURN);
        assert_eq!(m.cpu.regs[0], u32::MAX);
    }
    #[test]
    fn native_tokens_and_duplicate_entries_are_not_lost() {
        assert_eq!(controls(b"A @0 B @8 %12 C"), vec![b"%12".to_vec()]);
        let bad = br#"{"version":2,"language":"en","name":"English","entries":{"text.00cabf84":"x","text.00cabf84":"y"}}"#;
        assert!(parse_pack(bad).is_err());
    }
    #[test]
    fn integer_version_and_legacy_fonts_are_validated_without_pack_rom_metadata() {
        let minimal =
            serde_json::json!({"version":2,"language":"en","name":"English","entries":{}});
        assert_eq!(
            parse_pack(minimal.to_string().as_bytes())
                .unwrap()
                .font
                .len(),
            0
        );
        for version in [
            serde_json::json!(true),
            serde_json::json!(2.0),
            serde_json::json!("2"),
            serde_json::json!(3),
        ] {
            let mut bad = minimal.clone();
            bad["version"] = version;
            assert!(parse_pack(bad.to_string().as_bytes()).is_err());
        }
        let legacy = serde_json::json!({"format":"gta-advance-language-pack-v1","base_rom_sha256":USA,"scope":"confirmed-five-language-tables","language":"ru","name":"Russian","fallback":"en","entries":{}});
        assert_eq!(
            parse_pack(legacy.to_string().as_bytes())
                .unwrap()
                .font
                .len(),
            33
        );
        let mut wrong = legacy.clone();
        wrong["base_rom_sha256"] = "wrong".into();
        assert!(parse_pack(wrong.to_string().as_bytes()).is_err());
    }
}
