//! Stable host UI keys shared by language packs, the renderer and authoring tools.
use crate::language_font::Font;
use std::{collections::BTreeMap, sync::OnceLock};
pub fn catalog() -> &'static BTreeMap<String, String> {
    static CATALOG: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../resources/menu-en.json")).expect("menu catalog")
    })
}
fn tokens(s: &str) -> Vec<&str> {
    let mut output = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('{') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('}') else {
            break;
        };
        output.push(&rest[..end]);
        rest = &rest[end + 1..];
    }
    output.sort_unstable();
    output
}
pub fn encode(text: &str, font: Option<&Font>) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for c in text.chars() {
        if let Some(code) = font.and_then(|f| f.encode(c)) {
            out.push(code);
            continue;
        }
        let c = match c {
            '’' | '‘' => '\'',
            '“' | '”' => '"',
            '—' | '–' => '-',
            c => c,
        };
        for c in c.to_uppercase() {
            if let Some(code) = font.and_then(|f| f.encode(c)) {
                out.push(code);
            } else if c == '\n' {
                out.push(10);
            } else if (32..=255).contains(&(c as u32)) {
                out.push(c as u8);
            } else {
                return Err(format!("Menu font has no glyph for {c}"));
            }
        }
    }
    Ok(out)
}
pub fn encode_lossy(text: &str, font: Option<&Font>) -> Vec<u8> {
    text.chars()
        .flat_map(|c| encode(&c.to_string(), font).unwrap_or_else(|_| vec![b'?']))
        .collect()
}
pub fn validate(menu: &BTreeMap<String, String>, font: &Font) -> Result<(), String> {
    for (id, text) in menu {
        let base = catalog()
            .get(id)
            .ok_or_else(|| format!("Unknown menu ID: {id}"))?;
        if text.is_empty() {
            continue;
        }
        if text.chars().count() > 512 || text.chars().any(char::is_control) {
            return Err(format!("Invalid menu text: {id}"));
        }
        if tokens(text) != tokens(base) {
            return Err(format!("menu.{id}: preserve placeholders from {base}"));
        }
        encode(text, Some(font)).map_err(|e| format!("menu.{id}: {e}"))?;
    }
    Ok(())
}
#[derive(Clone, Default)]
pub struct Locale {
    pub strings: BTreeMap<String, String>,
}
impl Locale {
    pub fn text<'a>(&'a self, id: &str) -> &'a str {
        self.strings
            .get(id)
            .filter(|s| !s.is_empty())
            .map(String::as_str)
            .unwrap_or_else(|| catalog().get(id).map_or("?", String::as_str))
    }
    pub fn translate(&self, english: &str) -> String {
        for (id, base) in catalog() {
            if base.eq_ignore_ascii_case(english) {
                return self.text(id).to_owned();
            }
            if let Some(start) = base.find('{') {
                let end = base[start..].find('}').unwrap() + start;
                let (prefix, suffix) = (&base[..start], &base[end + 1..]);
                if let Some(value) = english
                    .strip_prefix(prefix)
                    .and_then(|s| s.strip_suffix(suffix))
                {
                    return self.text(id).replace(&base[start..=end], value);
                }
            }
        }
        english.to_owned()
    }
}
pub fn native_key(id: u32) -> Option<&'static str> {
    Some(match id {
        8 => "resume_game",
        9 => "quit_to_menu",
        0x3d => "options",
        0xd => "brief",
        0xc => "stats",
        0x231 => "camera",
        0x18 => "game_paused",
        0x14e => "confirm_quit",
        0x234 => "save_and_quit",
        0x42 => "sfx_volume",
        0x43 => "music",
        0xe => "music_on",
        0xf => "music_off",
        0x1cc => "controls",
        0x1cf => "credits",
        0x232 => "auto",
        0x1cd => "normal",
        0x1ce => "alternate",
        0x9a => "high",
        0x99 => "medium",
        0x98 => "low",
        0x97 => "off",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_translations_have_fallback_and_preserve_template_arguments() {
        let mut locale = Locale::default();
        locale
            .strings
            .insert("quit_to_menu".into(), "В меню".into());
        locale
            .strings
            .insert("save_failed".into(), "Ошибка: {error}".into());
        locale.strings.insert("back".into(), String::new());
        assert_eq!(locale.text("quit_to_menu"), "В меню");
        assert_eq!(locale.text("back"), "Back");
        assert_eq!(
            locale.translate("Save failed: disk full"),
            "Ошибка: disk full"
        );
        validate(&locale.strings, &crate::localization::test_font()).unwrap();
        locale.strings.insert("save_failed".into(), "Ошибка".into());
        assert!(validate(&locale.strings, &crate::localization::test_font()).is_err());
        assert!(validate(
            &BTreeMap::from([("typo".into(), "Text".into())]),
            &Font::default()
        )
        .is_err());
        assert!(validate(
            &BTreeMap::from([("back".into(), "Ψ".into())]),
            &Font::default()
        )
        .is_err());
        assert_eq!(encode_lossy("Path Ψ", None), b"PATH ?");
    }
}
