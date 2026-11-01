//! Opt-in external text data for the verified USA image. No guest code patches.
use crate::language_font::{compile_variant, ColorDefinition, ColorFont, FontVariants};
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

/// Export English and add newly discovered IDs, preserving every existing value.
/// Invalid files are left alone so unfinished translator edits remain recoverable.
pub fn ensure_english(
    directory: &Path,
    rom: &[u8],
) -> Result<Option<PathBuf>, Box<dyn std::error::Error>> {
    let path = directory.join("en.json");
    let previous = fs::read(&path).ok();
    let mut pack = if let Some(bytes) = previous.as_ref() {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(
            bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes),
        ) else {
            return Ok(None);
        };
        if value.get("version").and_then(|v| v.as_u64()) != Some(2)
            || value.get("language").and_then(|v| v.as_str()) != Some("en")
            || !value.get("entries").is_some_and(|v| v.is_object())
        {
            return Ok(None);
        }
        value
    } else {
        serde_json::json!({"version":2,"language":"en","name":"English (editable)","alphabet":"","entries":{}})
    };
    if format!("{:x}", Sha256::digest(rom)) != USA {
        return Err("English export requires the verified USA ROM".into());
    }
    let tables = entries(rom);
    if tables.len() != 1156 + UI_COUNT {
        return Err("USA language table structure differs".into());
    }
    let mut added = 0;
    for (id, pointers) in tables {
        let target = pack["entries"].as_object_mut().unwrap();
        if !target.contains_key(&id) {
            let original: String = string(rom, word(rom, pointers[0]))
                .unwrap()
                .iter()
                .map(|&byte| char::from(byte))
                .collect();
            target.insert(id, original.into());
            added += 1;
        }
    }
    if pack.get("menu").is_none() {
        pack["menu"] = serde_json::json!({});
    }
    let Some(menu) = pack["menu"].as_object_mut() else {
        return Ok(None);
    };
    for (id, english) in crate::menu_text::catalog() {
        if !menu.contains_key(id) {
            menu.insert(id.clone(), english.clone().into());
            added += 1;
        }
    }
    if previous.is_some() && added == 0 {
        return Ok(None);
    }
    let mut bytes = serde_json::to_vec_pretty(&pack)?;
    bytes.push(b'\n');
    // The generated pack must pass exactly the same checks as authored packs.
    Language::from_bytes(bytes.clone(), rom)?;
    fs::create_dir_all(directory)?;
    let output = if previous.is_some() {
        directory.join(format!(".en-additions-{}.tmp", std::process::id()))
    } else {
        path.clone()
    };
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    if previous.is_some() {
        if fs::read(&path)? != previous.unwrap() {
            fs::remove_file(&output)?;
            return Err("English pack changed while adding new IDs; retry next startup".into());
        }
        fs::rename(&output, &path)?;
    }
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

use crate::rom_profile::USA;
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
    #[serde(default)]
    color_font: Option<ColorDefinition>,
    #[serde(default)]
    antialiased_font: Option<FontDefinition>,
    #[serde(default)]
    plain_font: Option<FontDefinition>,
    #[serde(default)]
    dialogue_font: Option<FontDefinition>,
    #[serde(default)]
    cutscene_font: Option<FontDefinition>,
    #[serde(deserialize_with = "crate::language_font::unique_map")]
    entries: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "crate::language_font::unique_map")]
    menu: BTreeMap<String, String>,
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
    color_font: ColorFont,
    font_variants: FontVariants,
    menu: BTreeMap<String, String>,
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
        let font = Font::compile(&pack.alphabet, pack.font)?;
        let color_font = ColorFont::compile(&pack.alphabet, &font, pack.color_font)?;
        let font_variants = FontVariants {
            antialiased: compile_variant(
                &pack.alphabet,
                "antialiased_font",
                pack.antialiased_font,
            )?,
            plain: compile_variant(&pack.alphabet, "plain_font", pack.plain_font)?,
            dialogue: compile_variant(&pack.alphabet, "dialogue_font", pack.dialogue_font)?,
            cutscene: compile_variant(&pack.alphabet, "cutscene_font", pack.cutscene_font)?,
        };
        Ok(Pack {
            language: pack.language,
            name: pack.name,
            fallback: pack.fallback,
            entries: pack.entries,
            font,
            color_font,
            font_variants,
            menu: pack.menu,
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
        let font = legacy_font()?;
        let color_font = ColorFont::generated(&font);
        Ok(Pack {
            language: pack.language,
            name: pack.name,
            fallback: pack.fallback,
            entries: pack.entries,
            font,
            color_font,
            font_variants: FontVariants::default(),
            menu: BTreeMap::new(),
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

const UI_BANK: usize = 0xec4324;
const UI_COUNT: usize = 618;
fn entries(rom: &[u8]) -> BTreeMap<String, Vec<usize>> {
    let mut result: BTreeMap<_, _> = tables(rom)
        .into_iter()
        .map(|(id, offset)| (id, (0..5).map(|slot| offset + slot * 8 + 4).collect()))
        .collect();
    if rom.len() >= UI_BANK + UI_COUNT * 5 * 4 {
        for index in 0..UI_COUNT {
            let pointers: Vec<_> = (0..5)
                .map(|slot| UI_BANK + (slot * UI_COUNT + index) * 4)
                .collect();
            if pointers
                .iter()
                .all(|&p| string(rom, word(rom, p)).is_some())
            {
                result.insert(format!("ui.{index:04x}"), pointers);
            }
        }
    }
    result
}

fn controls(text: &[u8]) -> Vec<Vec<u8>> {
    let mut result = Vec::new();
    let mut i = 0;
    while i + 1 < text.len() {
        if matches!(text[i], b'@' | b'%')
            && (text[i + 1].is_ascii_digit() || text[i] == b'%' && text[i + 1].is_ascii_uppercase())
        {
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
            '\n' | '\r' => 10,
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

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Language {
    pub bytes: Vec<u8>,
    pub label: String,
    pub translated: usize,
    pub extended: bool,
    pub glyph_calls: u64,
    pub font: Font,
    color_font: ColorFont,
    #[serde(default)]
    font_variants: FontVariants,
    pub menu: BTreeMap<String, String>,
    pub(crate) menu_font: Font,
    pub(crate) menu_color_font: ColorFont,
    #[serde(skip)]
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
        if bytes.len() as u64 > MAX_FILE {
            return Err("Language pack exceeds 4 MiB".into());
        }
        if format!("{:x}", Sha256::digest(rom)) != USA {
            return Err("Language packs require the verified USA ROM".into());
        }
        let pack = parse_pack(&bytes)?;
        crate::menu_text::validate(&pack.menu, &pack.font)?;
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
        let tables = entries(rom);
        if tables.len() != 1156 + UI_COUNT {
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
        for (id, pointers) in &tables {
            let english = word(rom, pointers[0]);
            let mut pointer = english;
            if let Some(text) = pack.entries.get(id).filter(|t| !t.is_empty()) {
                let original = string(rom, english).unwrap();
                let native_pointer = if text.chars().any(|c| pack.font.encode(c).is_some()) {
                    None
                } else {
                    pointers.iter().map(|&p| word(rom, p)).find(|&p| {
                        string(rom, p)
                            .unwrap()
                            .iter()
                            .map(|&b| char::from(b))
                            .eq(text.chars())
                    })
                };
                if let Some(native) = native_pointer {
                    pointer = native;
                    translated += 1;
                    for &literal in pointers {
                        set_word(&mut image, literal, pointer);
                    }
                    continue;
                }
                let (encoded, needs_font) =
                    encode(text, &pack.font).map_err(|error| format!("{id}: {error}"))?;
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
            for &literal in pointers {
                set_word(&mut image, literal, pointer);
            }
        }
        if extended {
            install_fonts(
                &mut image,
                &pack.font,
                &pack.color_font,
                &pack.font_variants,
            );
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
            menu_font: pack.font.clone(),
            menu_color_font: pack.color_font.clone(),
            font: pack.font,
            color_font: pack.color_font,
            font_variants: pack.font_variants,
            menu: pack.menu,
            rom: image,
        })
    }
    pub fn install(&mut self, m: &mut Machine) {
        m.bus.rom = std::mem::take(&mut self.rom).into();
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

/// Swap only cartridge text data. CPU, devices, timers, audio and game progress
/// stay live in memory; no serialized state or temporary save files are needed.
pub fn change_live(
    m: &mut Machine,
    base: &[u8],
    current: &mut Option<Language>,
    bytes: Option<Vec<u8>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let original = bytes.is_none();
    let mut next = Language::from_bytes(
        bytes.unwrap_or_else(|| {
            br#"{"version":2,"language":"en","name":"Original game","entries":{}}"#.to_vec()
        }),
        base,
    )?;
    // Retain the outgoing glyphs when returning to Latin-only text: already
    // composed messages can finish displaying until the game redraws them.
    if !next.extended {
        if let Some(old) = current.as_ref().filter(|old| old.extended) {
            next.font = old.font.clone();
            next.color_font = old.color_font.clone();
            next.font_variants = old.font_variants.clone();
            next.extended = true;
            install_fonts(
                &mut next.rom,
                &next.font,
                &next.color_font,
                &next.font_variants,
            );
        }
    }
    if original {
        // Restore the cartridge's own language selection for future lookups.
        for pointers in entries(base).values() {
            for &p in pointers {
                set_word(&mut next.rom, p, word(base, p));
            }
        }
        next.label = "Original game / ROM".into();
    }
    // Keep previous arenas alive: an in-flight script may still own a string
    // pointer outside UI state. Reuse identical arenas on subsequent switches
    // instead of growing memory each time a language is toggled.
    let cartridge_len = base.len();
    let tail = &next.rom[cartridge_len..];
    let cached = if tail.is_empty() {
        Some(cartridge_len)
    } else {
        (cartridge_len..=m.bus.rom.len().saturating_sub(tail.len()))
            .step_by(4)
            .find(|&start| m.bus.rom.get(start..start + tail.len()) == Some(tail))
    };
    let mut image = m.bus.rom.clone();
    image[..cartridge_len].copy_from_slice(&next.rom[..cartridge_len]);
    let arena = if let Some(start) = cached {
        start
    } else {
        while image.len() % 4 != 0 {
            image.push(0);
        }
        let start = image.len();
        if start + tail.len() > 32 * 1024 * 1024 {
            return Err("Language arenas exceed 32 MiB; current language retained".into());
        }
        image.extend_from_slice(tail);
        start
    };
    let delta = (arena - cartridge_len) as u32;
    for literal in entries(base)
        .into_values()
        .flatten()
        .chain([
            0x2add8, 0x4b74, 0x4f3c, 0x601c, 0x2e520, 0x2e4f0, 0x642cc, 0x642d0,
        ])
        .chain(COLOR_LITERALS)
        .chain(COLOR_WIDTH_LITERALS)
        .chain(ALT_COLOR_LITERALS)
        .chain(ALT_COLOR_WIDTH_LITERALS)
    {
        let pointer = word(&image, literal);
        if pointer >= 0x08000000 + cartridge_len as u32 {
            set_word(&mut image, literal, pointer + delta);
        }
    }
    next.rom = image.into_vec();
    let mut ranges = BTreeMap::new();
    let guest_slot = word(&m.bus.ewram, 0x35cc0).min(4) as usize;
    for pointers in entries(base).values() {
        for (slot, &p) in pointers.iter().enumerate() {
            let old = word(&m.bus.rom, p);
            let new = word(
                &next.rom,
                pointers[if original { guest_slot } else { slot }],
            );
            if old != new {
                if let Some(text) = string(&m.bus.rom, old) {
                    let length = string(&next.rom, new).unwrap().len();
                    ranges
                        .entry(old)
                        .or_insert((new, text.len() as u32, length as u32));
                }
            }
        }
    }
    for (literal, length) in [
        (0x2add8, 88 * 32),
        (0x4b74, 88 * 32),
        (0x2e520, 88 * 32),
        (0x2e4f0, 88 * 32),
        (0x642cc, 224 * 64),
        (0x642d0, 224),
        (0x1770, 224 * 256),
        (0x1774, 224),
        (0x1f18, 224 * 256),
        (0x1f1c, 224),
    ] {
        let old = word(&m.bus.rom, literal);
        let new = word(&next.rom, literal);
        if old != new {
            let extra = if length == 88 * 32 {
                current.as_ref().map_or(0, |l| l.font.len() * 32)
            } else {
                0
            };
            ranges.insert(old, (new, (length + extra) as u32, (length + extra) as u32));
        }
    }
    let relocate = |value: u32| {
        if let Some((&start, &(target, length, next_length))) = ranges.range(..=value).next_back() {
            let offset = value - start;
            if offset <= length {
                return target + offset.min(next_length);
            }
        }
        value
    };
    // Only text/UI state and CPU stacks contain these references. Never scan
    // world/entity memory: fixed-point coordinates can look like ROM pointers.
    for (internal, start, end) in [
        (false, 0x26000, 0x27280),
        (false, 0x35b00, 0x36340),
        (true, 0x7b00, 0x8000),
    ] {
        let memory = if internal {
            &mut m.bus.iwram
        } else {
            &mut m.bus.ewram
        };
        for chunk in memory[start..end].chunks_exact_mut(4) {
            let value = u32::from_le_bytes(chunk.try_into().unwrap());
            chunk.copy_from_slice(&relocate(value).to_le_bytes());
        }
    }
    for reg in &mut m.cpu.regs[..13] {
        *reg = relocate(*reg);
    }
    // The next native draw repopulates its glyph cache with the selected font.
    set_word(&mut m.bus.ewram, 0x142c, 0);
    next.install(m);
    *current = Some(next);
    Ok(())
}

pub fn changes(path: &Path) -> Result<Vec<(u64, Option<Vec<u8>>)>, Box<dyn std::error::Error>> {
    let mut result = Vec::new();
    for line in fs::read_to_string(path)?.lines() {
        if let Some(line) = line.strip_prefix("# language ") {
            let (frame, json) = line
                .split_once(' ')
                .ok_or("Invalid recorded language change")?;
            let frame = frame.parse()?;
            if result.last().is_some_and(|(last, _)| *last > frame) {
                return Err("Invalid language event order".into());
            }
            let pack: Option<String> = serde_json::from_str(json)?;
            result.push((frame, pack.map(String::into_bytes)));
        }
    }
    Ok(result)
}

// Preserve 88 tiles per font view, including adjacent reserved/icon tiles,
// and append package-owned tiles. The dialogue font has its own
// colour-remapping path, separate from the antialiased and plain UI fonts.
fn install_fonts(rom: &mut Vec<u8>, custom: &Font, color: &ColorFont, variants: &FontVariants) {
    while rom.len() % 4 != 0 {
        rom.push(0);
    }
    for (source, literals, ink, drawing) in [
        (
            0x346478,
            &[0x2add8][..],
            13u8,
            variants.antialiased.as_ref().unwrap_or(custom),
        ),
        (
            0x346f78,
            &[0x4b74, 0x4f3c, 0x601c][..],
            3u8,
            variants.plain.as_ref().unwrap_or(custom),
        ),
        // Dialogue compositor recolours nibble 3 to the message colour.
        (
            0x3478d8,
            &[0x2e520][..],
            3u8,
            variants.dialogue.as_ref().unwrap_or(custom),
        ),
        (
            0x346d78,
            &[0x2e4f0][..],
            3u8,
            variants.dialogue.as_ref().unwrap_or(custom),
        ),
    ] {
        let mut font = rom[source..source + FONT_GLYPHS * 32].to_vec();
        for index in 0..drawing.len() {
            font.extend_from_slice(&drawing.tile(index, ink));
        }
        let address = 0x08000000 + rom.len() as u32;
        rom.extend_from_slice(&font);
        for &literal in literals {
            assert_eq!(word(rom, literal), 0x08000000 + source as u32);
            set_word(rom, literal, address);
        }
    }
    install_cutscene_font(rom, variants.cutscene.as_ref().unwrap_or(custom));
    install_color_font(rom, custom, color);
}

const COLOR_LITERALS: [usize; 11] = [
    0x1770, 0x1ad0, 0x1ec0, 0x206c, 0x20c8, 0x2230, 0x3dfc, 0x3e5c, 0x5c9c, 0x5dbc, 0x62df0,
];
const COLOR_WIDTH_LITERALS: [usize; 11] = [
    0x1774, 0x1ad4, 0x1ec4, 0x2070, 0x20cc, 0x2234, 0x3e00, 0x3e60, 0x5ca0, 0x5dc0, 0x62df4,
];
// Unselected menu items switch to a second atlas and width table. Keep its
// original Latin glyphs while installing the same package-owned custom slots.
const ALT_COLOR_LITERALS: [usize; 5] = [0x1f18, 0x20bc, 0x20f8, 0x3e54, 0x5cc4];
const ALT_COLOR_WIDTH_LITERALS: [usize; 5] = [0x1f1c, 0x20c0, 0x20fc, 0x3e58, 0x5cc8];
// Match the game's bright-to-inactive shade ramp (1..9). Custom fonts can
// additionally use the darker ramp and white: keep outlines/shadows visible,
// and turn white fill into inactive gold. Zero always stays transparent.
const INACTIVE_COLOR_SHADES: [u8; 16] = [0, 5, 6, 7, 8, 10, 10, 12, 12, 9, 13, 14, 14, 14, 14, 5];
fn install_color_font(rom: &mut Vec<u8>, custom: &Font, color: &ColorFont) {
    for (source, width_source, literals, width_literals) in [
        (
            0x8314d0,
            0x83f4d0,
            &COLOR_LITERALS[..],
            &COLOR_WIDTH_LITERALS[..],
        ),
        (
            0x83f7b0,
            0x84d7b0,
            &ALT_COLOR_LITERALS[..],
            &ALT_COLOR_WIDTH_LITERALS[..],
        ),
    ] {
        let mut atlas = rom[source..source + 224 * 256].to_vec();
        let mut widths = rom[width_source..width_source + 224].to_vec();
        for (index, byte) in crate::language_font::private_codes()
            .into_iter()
            .take(custom.len())
            .enumerate()
        {
            let slot = byte as usize - 32;
            for (pixel, &shade) in atlas[slot * 256..(slot + 1) * 256]
                .iter_mut()
                .zip(&color.tiles[index])
            {
                *pixel = if source == 0x83f7b0 {
                    INACTIVE_COLOR_SHADES[shade as usize]
                } else {
                    shade
                };
            }
            widths[slot] = color.widths[index];
        }
        let address = 0x08000000 + rom.len() as u32;
        rom.extend_from_slice(&atlas);
        rom.extend_from_slice(&widths);
        for &literal in literals {
            assert_eq!(word(rom, literal), 0x08000000 + source as u32);
            set_word(rom, literal, address);
        }
        for &literal in width_literals {
            assert_eq!(word(rom, literal), 0x08000000 + width_source as u32);
            set_word(rom, literal, address + atlas.len() as u32);
        }
    }
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
    pointers: BTreeMap<u32, (String, usize)>,
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
    for (id, literals) in entries(rom) {
        for (slot, literal) in literals.into_iter().enumerate() {
            pointers.insert(0x08000000 + literal as u32, (id.clone(), slot));
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
    let value = crate::polyphonic_audio::read32(m, address);
    if (8..=13).contains(&(address >> 24)) {
        let normalized = 0x08000000 | (address & 0x1ffffff);
        TRACE.with(|cell| {
            let mut trace = cell.borrow_mut();
            let Some(trace) = trace.as_mut() else {
                return;
            };
            let Some((id, slot)) = trace.pointers.get(&normalized).cloned() else {
                return;
            };
            let machine = unsafe { &*(m as *const Machine) };
            let frame = machine.bus.frames;
            if trace.last.insert(normalized, frame) != Some(frame) && trace.error.is_none() {
                if let Err(error) = writeln!(
                    trace.output,
                    "{frame},{:08x},{id},{slot},{value:08x}",
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
    fn optional_font_variants_validate_shapes_and_share_base_encoding() {
        let mut json = serde_json::json!({"version":2,"language":"el","name":"Greek",
            "alphabet":"Ω","font":{"width":1,"height":1,"glyphs":{"Ω":["#"]},"aliases":{"ω":"Ω"}},"entries":{}});
        let original = parse_pack(json.to_string().as_bytes()).unwrap();
        assert!(original.font_variants.antialiased.is_none());
        assert!(original.font_variants.plain.is_none());
        assert!(original.font_variants.dialogue.is_none());
        assert!(original.font_variants.cutscene.is_none());
        for name in [
            "antialiased_font",
            "plain_font",
            "dialogue_font",
            "cutscene_font",
        ] {
            json[name] = serde_json::json!({"width":2,"height":1,"glyphs":{"Ω":[".#"]}});
        }
        let pack = parse_pack(json.to_string().as_bytes()).unwrap();
        assert_eq!(pack.font.encode('ω'), pack.font.encode('Ω'));
        for variant in [
            &pack.font_variants.antialiased,
            &pack.font_variants.plain,
            &pack.font_variants.dialogue,
            &pack.font_variants.cutscene,
        ] {
            let variant = variant.as_ref().unwrap();
            assert_eq!(
                variant.index(pack.font.encode('ω').unwrap() as u32),
                Some(0)
            );
            assert_ne!(variant.tile(0, 3), pack.font.tile(0, 3));
        }
        for name in [
            "antialiased_font",
            "plain_font",
            "dialogue_font",
            "cutscene_font",
        ] {
            let mut invalid = json.clone();
            invalid[name]["glyphs"] = serde_json::json!({});
            assert!(parse_pack(invalid.to_string().as_bytes()).is_err());
            invalid = json.clone();
            invalid[name]["aliases"] = serde_json::json!({"ω":"Ω"});
            assert!(parse_pack(invalid.to_string().as_bytes()).is_err());
        }
    }

    #[test]
    #[ignore = "requires the user's USA ROM"]
    fn distinct_font_routes_fall_back_independently_and_survive_live_reset() {
        let rom = fs::read(std::env::var_os("ADVANCE_LIBERTY_LANGUAGE_TEST_ROM").unwrap()).unwrap();
        let drawing =
            |row: &str| serde_json::json!({"width":row.len(),"height":1,"glyphs":{"Ω":[row]}});
        let json = serde_json::json!({"version":2,"language":"el","name":"Greek","alphabet":"Ω",
            "font":drawing("#"),"antialiased_font":drawing(".#"),
            "dialogue_font":drawing("..#"),"cutscene_font":drawing("...#"),
            "entries":{"text.00cc7f0c":"Ω Ω","ui.015b":"Ω"}});
        let pack = parse_pack(json.to_string().as_bytes()).unwrap();
        let mut machine = Machine::new(rom.clone());
        let mut current = None;
        change_live(
            &mut machine,
            &rom,
            &mut current,
            Some(json.to_string().into_bytes()),
        )
        .unwrap();
        for reset in [false, true] {
            if reset {
                change_live(&mut machine, &rom, &mut current, None).unwrap();
            }
            for (literal, source, ink, drawing) in [
                (
                    0x2add8,
                    0x346478,
                    13,
                    pack.font_variants.antialiased.as_ref().unwrap(),
                ),
                (0x4b74, 0x346f78, 3, &pack.font), // plain_font omitted: direct fallback
                (
                    0x2e520,
                    0x3478d8,
                    3,
                    pack.font_variants.dialogue.as_ref().unwrap(),
                ),
                (
                    0x2e4f0,
                    0x346d78,
                    3,
                    pack.font_variants.dialogue.as_ref().unwrap(),
                ),
            ] {
                let target = (word(&machine.bus.rom, literal) - 0x08000000) as usize;
                assert_eq!(
                    &machine.bus.rom[target..target + FONT_GLYPHS * 32],
                    &rom[source..source + FONT_GLYPHS * 32]
                );
                assert_eq!(
                    &machine.bus.rom[target + FONT_GLYPHS * 32..target + (FONT_GLYPHS + 1) * 32],
                    drawing.tile(0, ink)
                );
            }
            let atlas = (word(&machine.bus.rom, 0x642cc) - 0x08000000) as usize;
            let widths = (word(&machine.bus.rom, 0x642d0) - 0x08000000) as usize;
            let slot = crate::language_font::private_codes()[0] as usize - 32;
            assert_eq!(
                &machine.bus.rom[atlas + slot * 64..atlas + slot * 64 + 8],
                &[0, 0, 0, 2, 0, 0, 0, 0]
            );
            assert_eq!(machine.bus.rom[widths + slot], 4);
            let serialized = serde_json::to_value(current.as_ref().unwrap()).unwrap();
            let restored: Language = serde_json::from_value(serialized.clone()).unwrap();
            assert_eq!(
                restored
                    .font_variants
                    .cutscene
                    .as_ref()
                    .unwrap()
                    .glyph_width(),
                4
            );
            let mut legacy_snapshot = serialized;
            legacy_snapshot
                .as_object_mut()
                .unwrap()
                .remove("font_variants");
            assert!(serde_json::from_value::<Language>(legacy_snapshot)
                .unwrap()
                .font_variants
                .cutscene
                .is_none());
        }
    }

    #[test]
    fn host_menu_schema_is_optional_and_duplicate_keys_are_rejected() {
        let original = br#"{"version":2,"language":"en","name":"English","entries":{}}"#;
        assert!(parse_pack(original).unwrap().menu.is_empty());
        let bad=br#"{"version":2,"language":"en","name":"English","entries":{},"menu":{"back":"Back","back":"Return"}}"#;
        assert!(parse_pack(bad).is_err());
    }
    #[test]
    #[ignore = "requires the user's USA ROM"]
    fn host_menu_only_pack_keeps_guest_fonts_and_live_switches_alphabets() {
        let rom = fs::read(std::env::var_os("ADVANCE_LIBERTY_LANGUAGE_TEST_ROM").unwrap()).unwrap();
        let mut machine = Machine::new(rom.clone());
        let mut current = None;
        let ru = fs::read("resources/legacy-language-v1-font.json").unwrap();
        let font: serde_json::Value = serde_json::from_slice(&ru).unwrap();
        let pack = serde_json::json!({"version":2,"language":"ru","name":"Russian","alphabet":font["alphabet"],"font":font["font"],"entries":{"text.00cabf84":"МИССИЯ"}});
        change_live(
            &mut machine,
            &rom,
            &mut current,
            Some(pack.to_string().into_bytes()),
        )
        .unwrap();
        let greek = serde_json::json!({"version":2,"language":"el","name":"Greek","alphabet":"Ω","font":{"width":3,"height":3,"glyphs":{"Ω":["###","#.#","###"]}},"entries":{},"menu":{"quit_to_desktop":"Ω"}});
        let language = Language::from_bytes(greek.to_string().into_bytes(), &rom).unwrap();
        assert!(!language.extended);
        assert_eq!(&language.rom[0x8314d0..0x83f4d0], &rom[0x8314d0..0x83f4d0]);
        change_live(
            &mut machine,
            &rom,
            &mut current,
            Some(greek.to_string().into_bytes()),
        )
        .unwrap();
        let language = current.as_ref().unwrap();
        assert!(
            language.font.encode('М').is_some(),
            "unfinished guest text retains its previous font"
        );
        assert!(language.menu_font.encode('Ω').is_some());
        assert_eq!(language.menu["quit_to_desktop"], "Ω");
        #[cfg(feature = "desktop")]
        {
            let mut skin = crate::game_menu::Skin::load(&machine.bus.rom).unwrap();
            std::sync::Arc::get_mut(&mut skin)
                .unwrap()
                .apply_language(current.as_ref())
                .unwrap();
            assert_eq!(skin.host("quit_to_desktop"), &[0x80]);
        }
    }
    #[test]
    #[ignore = "requires the user's USA ROM"]
    fn live_change_keeps_game_state_and_reuses_memory_arenas() {
        let rom = fs::read(std::env::var_os("ADVANCE_LIBERTY_LANGUAGE_TEST_ROM").unwrap()).unwrap();
        let mut machine = Machine::new(rom.clone());
        let original = word(&rom, 0xcc7f0c + 4);
        machine.bus.frames = 420;
        machine.bus.clock = 1234567;
        machine.bus.audio_buf = vec![17, -19];
        set_word(&mut machine.bus.ewram, 0x1000, original); // world data is not a text reference
        set_word(&mut machine.bus.ewram, 0x2693c, original);
        // The native menu can be using the unselected-item font at switch time.
        set_word(&mut machine.bus.ewram, 0x36308, word(&rom, 0x1f18) + 256);
        set_word(&mut machine.bus.ewram, 0x36314, word(&rom, 0x1f1c) + 7);
        let cpu = machine.cpu.clone();
        let pack=serde_json::json!({"version":2,"language":"el","name":"Greek","alphabet":"Ω","font":{"width":3,"height":3,"glyphs":{"Ω":["###","#.#","###"]}},"entries":{"text.00cc7f0c":"Ω Ω", "ui.015b":"Ω"}}).to_string().into_bytes();
        let mut current = None;
        change_live(&mut machine, &rom, &mut current, Some(pack.clone())).unwrap();
        let first_len = machine.bus.rom.len();
        assert_eq!(
            word(&machine.bus.ewram, 0x36308),
            word(&machine.bus.rom, 0x1f18) + 256
        );
        assert_eq!(
            word(&machine.bus.ewram, 0x36314),
            word(&machine.bus.rom, 0x1f1c) + 7
        );
        let pointer = word(&machine.bus.ewram, 0x2693c);
        assert_eq!(
            string(&machine.bus.rom, pointer),
            Some(&[0x80, b' ', 0x80][..])
        );
        assert_eq!(
            string(
                &machine.bus.rom,
                word(&machine.bus.rom, UI_BANK + 0x15b * 4)
            ),
            Some(&[0x80][..])
        );
        assert_eq!(word(&machine.bus.ewram, 0x1000), original);
        assert_eq!(machine.cpu.regs, cpu.regs);
        assert_eq!(machine.cpu.cpsr, cpu.cpsr);
        assert_eq!(machine.bus.frames, 420);
        assert_eq!(machine.bus.clock, 1234567);
        assert_eq!(machine.bus.audio_buf, vec![17, -19]);
        change_live(&mut machine, &rom, &mut current, None).unwrap();
        assert_eq!(
            word(&machine.bus.ewram, 0x36308),
            word(&machine.bus.rom, 0x1f18) + 256
        );
        assert_eq!(
            word(&machine.bus.ewram, 0x36314),
            word(&machine.bus.rom, 0x1f1c) + 7
        );
        assert_eq!(word(&machine.bus.ewram, 0x2693c), original);
        assert_eq!(
            word(&machine.bus.rom, UI_BANK + 0x15b * 4),
            word(&rom, UI_BANK + 0x15b * 4)
        );
        let reset_len = machine.bus.rom.len();
        assert!(reset_len >= first_len);
        for _ in 0..4 {
            change_live(&mut machine, &rom, &mut current, Some(pack.clone())).unwrap();
            change_live(&mut machine, &rom, &mut current, None).unwrap();
        }
        assert_eq!(machine.bus.rom.len(), reset_len);
        let previous = machine.bus.rom.clone();
        assert!(change_live(
            &mut machine,
            &rom,
            &mut current,
            Some(b"invalid JSON".to_vec())
        )
        .is_err());
        assert_eq!(machine.bus.rom, previous);
        assert_eq!(machine.bus.frames, 420);
    }
    #[test]
    #[ignore = "requires the user's USA ROM"]
    fn cutscene_atlas_preserves_original_text_and_covers_all_transport_slots() {
        let rom = fs::read(
            std::env::var_os("ADVANCE_LIBERTY_LANGUAGE_TEST_ROM")
                .expect("ADVANCE_LIBERTY_LANGUAGE_TEST_ROM"),
        )
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
        let mut color = ColorFont::generated(&font);
        // White is valid package ink even though original gold letters use 1..9.
        for tile in &mut color.tiles {
            tile[0] = 15;
        }
        install_fonts(&mut image, &font, &color, &FontVariants::default());
        let atlas = (word(&image, 0x642cc) - 0x08000000) as usize;
        let widths = (word(&image, 0x642d0) - 0x08000000) as usize;
        let color_atlas = (word(&image, 0x1770) - 0x08000000) as usize;
        let color_widths = (word(&image, 0x1774) - 0x08000000) as usize;
        assert!(COLOR_LITERALS
            .iter()
            .all(|&literal| word(&image, literal) == word(&image, 0x1770)));
        assert!(COLOR_WIDTH_LITERALS
            .iter()
            .all(|&literal| word(&image, literal) == word(&image, 0x1774)));
        let alt_atlas = (word(&image, 0x1f18) - 0x08000000) as usize;
        let alt_widths = (word(&image, 0x1f1c) - 0x08000000) as usize;
        assert!(ALT_COLOR_LITERALS
            .iter()
            .all(|&literal| word(&image, literal) == word(&image, 0x1f18)));
        assert!(ALT_COLOR_WIDTH_LITERALS
            .iter()
            .all(|&literal| word(&image, literal) == word(&image, 0x1f1c)));
        // Verify the shade conversion against actual original menu letters,
        // rather than just expecting the implementation's lookup table.
        for byte in b"ABCNEW" {
            let slot = (*byte as usize - 32) * 256;
            let expected: Vec<_> = rom[0x8314d0 + slot..0x8314d0 + slot + 256]
                .iter()
                .map(|&shade| INACTIVE_COLOR_SHADES[shade as usize])
                .collect();
            assert_eq!(expected, rom[0x83f7b0 + slot..0x83f7b0 + slot + 256]);
        }
        for byte in 32..=255u8 {
            let slot = byte as usize - 32;
            if font.index(byte as u32).is_some() {
                assert_eq!(image[color_atlas + slot * 256], 15);
                assert_eq!(image[alt_atlas + slot * 256], 5);
                let inactive: Vec<_> = image
                    [color_atlas + slot * 256..color_atlas + (slot + 1) * 256]
                    .iter()
                    .map(|&shade| INACTIVE_COLOR_SHADES[shade as usize])
                    .collect();
                assert_eq!(
                    &image[alt_atlas + slot * 256..alt_atlas + (slot + 1) * 256],
                    inactive.as_slice()
                );
                assert_eq!(image[alt_widths + slot], image[color_widths + slot]);
            } else {
                assert_eq!(
                    &image[alt_atlas + slot * 256..alt_atlas + (slot + 1) * 256],
                    &rom[0x83f7b0 + slot * 256..0x83f7b0 + (slot + 1) * 256]
                );
                assert_eq!(image[alt_widths + slot], rom[0x84d7b0 + slot]);
            }
            if font.index(byte as u32).is_none() {
                assert_eq!(
                    &image[color_atlas + slot * 256..color_atlas + (slot + 1) * 256],
                    &rom[0x8314d0 + slot * 256..0x8314d0 + (slot + 1) * 256]
                );
                assert_eq!(image[color_widths + slot], rom[0x83f4d0 + slot]);
                assert_eq!(
                    &image[atlas + slot * 64..atlas + (slot + 1) * 64],
                    &rom[0x84da90 + slot * 64..0x84da90 + (slot + 1) * 64]
                );
                assert_eq!(image[widths + slot], rom[0x851310 + slot]);
            } else {
                assert_eq!(image[color_atlas + slot * 256 + ColorFont::offset(1, 3)], 1);
                assert_eq!(image[color_widths + slot] + 2, 10);
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
        let rom = fs::read(
            std::env::var_os("ADVANCE_LIBERTY_LANGUAGE_TEST_ROM")
                .expect("ADVANCE_LIBERTY_LANGUAGE_TEST_ROM"),
        )
        .unwrap();
        let root = std::env::temp_dir().join(format!("english-export-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        let directory = root.join("languages");
        let path = ensure_english(&directory, &rom).unwrap().unwrap();
        let bytes = fs::read(&path).unwrap();
        let pack = parse_pack(&bytes).unwrap();
        assert_eq!(pack.entries.len(), 1774);
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
        let mut partial: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        partial["entries"]
            .as_object_mut()
            .unwrap()
            .remove("ui.015b");
        partial["entries"]["text.00cabfac"] = "CUSTOM FAILURE".into();
        fs::write(&path, serde_json::to_vec(&partial).unwrap()).unwrap();
        assert!(ensure_english(&directory, &rom).unwrap().is_some());
        let restored = parse_pack(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored.entries["ui.015b"], "RAMPAGE FAILED.");
        assert_eq!(restored.entries["text.00cabfac"], "CUSTOM FAILURE");
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
        let rom = fs::read(
            std::env::var_os("ADVANCE_LIBERTY_LANGUAGE_TEST_ROM")
                .expect("ADVANCE_LIBERTY_LANGUAGE_TEST_ROM"),
        )
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
            color_font: ColorFont::generated(&test_font()),
            font_variants: FontVariants::default(),
            menu: BTreeMap::new(),
            menu_font: test_font(),
            menu_color_font: ColorFont::generated(&test_font()),
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
