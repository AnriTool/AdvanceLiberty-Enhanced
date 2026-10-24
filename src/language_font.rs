//! Package-owned alphabets and monochrome glyphs for the original 8x8 cells.
use serde::Deserialize;
use std::collections::BTreeMap;

pub fn unique_map<'de, D, V>(deserializer: D) -> Result<BTreeMap<String, V>, D::Error>
where
    D: serde::Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct Map<V>(std::marker::PhantomData<V>);
    impl<'de, V: Deserialize<'de>> serde::de::Visitor<'de> for Map<V> {
        type Value = BTreeMap<String, V>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an object with unique keys")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut map: A,
        ) -> Result<Self::Value, A::Error> {
            let mut output = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, V>()? {
                if output.insert(key.clone(), value).is_some() {
                    return Err(serde::de::Error::custom(format!("Duplicate key: {key}")));
                }
            }
            Ok(output)
        }
    }
    deserializer.deserialize_map(Map(std::marker::PhantomData))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub width: u8,
    pub height: u8,
    #[serde(default)]
    pub shadow: bool,
    #[serde(deserialize_with = "unique_map")]
    pub glyphs: BTreeMap<String, Vec<String>>,
    #[serde(default, deserialize_with = "unique_map")]
    pub aliases: BTreeMap<String, String>,
}

// These are transport slots in the GBA parser, not a language alphabet.
// Keep all of its original accented letters and punctuation untouched.
pub fn private_codes() -> Vec<u8> {
    (0x80..=0xff)
        .filter(|b| {
            !matches!(
                b,
                0x92 | 0xa1
                    | 0xbf
                    | 0xc0
                    | 0xc1
                    | 0xc4
                    | 0xc8
                    | 0xc9
                    | 0xcc
                    | 0xcd
                    | 0xcf
                    | 0xd1
                    | 0xd2
                    | 0xd3
                    | 0xd6
                    | 0xd9
                    | 0xda
                    | 0xdc
            )
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Font {
    width: usize,
    height: usize,
    shadow: bool,
    rows: Vec<Vec<u8>>,
    characters: BTreeMap<char, usize>,
    codes: Vec<u8>,
}
impl Default for Font {
    fn default() -> Self {
        Self {
            width: 0,
            height: 0,
            shadow: false,
            rows: Vec::new(),
            characters: BTreeMap::new(),
            codes: Vec::new(),
        }
    }
}
fn character(value: &str) -> Result<char, String> {
    let mut chars = value.chars();
    let first = chars.next().ok_or("Expected one Unicode character")?;
    if chars.next().is_some() || first.is_control() || matches!(first, ' ' | '@' | '%') {
        return Err(format!(
            "Invalid alphabet character: {value:?}; spaces, controls, @ and % are reserved"
        ));
    }
    Ok(first)
}
impl Font {
    pub fn compile(alphabet: &str, definition: Option<Definition>) -> Result<Self, String> {
        if alphabet.is_empty() && definition.is_none() {
            return Ok(Self::default());
        }
        let definition = definition.ok_or("An alphabet requires font glyphs")?;
        let width = definition.width as usize;
        let height = definition.height as usize;
        if !(1..=8).contains(&width)
            || !(1..=8).contains(&height)
            || definition.shadow && (width == 8 || height == 8)
        {
            return Err(
                "Font must fit an 8x8 cell, including its optional one-pixel shadow".into(),
            );
        }
        let pool = private_codes();
        if alphabet.chars().count() > pool.len() {
            return Err(format!(
                "Runtime supports at most {} custom glyphs",
                pool.len()
            ));
        }
        let mut font = Self {
            width,
            height,
            shadow: definition.shadow,
            ..Self::default()
        };
        for (index, c) in alphabet.chars().enumerate() {
            character(&c.to_string())?;
            if font.characters.insert(c, index).is_some() {
                return Err(format!("Duplicate alphabet character: {c}"));
            }
            let source = definition
                .glyphs
                .get(&c.to_string())
                .ok_or_else(|| format!("Missing glyph: {c}"))?;
            if source.len() != height {
                return Err(format!("Glyph {c}: expected {height} rows"));
            }
            let mut rows = Vec::new();
            for row in source {
                if row.len() != width || !row.bytes().all(|b| matches!(b, b'.' | b'#')) {
                    return Err(format!(
                        "Glyph {c}: each row must contain {width} '.'/'#' pixels"
                    ));
                }
                rows.push(
                    row.bytes()
                        .fold(0u8, |bits, b| (bits << 1) | u8::from(b == b'#')),
                );
            }
            font.rows.push(rows);
            font.codes.push(pool[index]);
        }
        if definition.glyphs.len() != font.rows.len() {
            return Err("Font glyph keys must match alphabet exactly".into());
        }
        let canonical = font.characters.clone();
        for (alias, target) in definition.aliases {
            let alias = character(&alias)?;
            let target = character(&target)?;
            let index = canonical
                .get(&target)
                .copied()
                .ok_or("Alias target must be an alphabet glyph")?;
            if font.characters.insert(alias, index).is_some() {
                return Err(format!("Alias conflicts with alphabet: {alias}"));
            }
        }
        Ok(font)
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn glyph_width(&self) -> u8 {
        self.width as u8
    }
    pub fn encode(&self, c: char) -> Option<u8> {
        self.characters.get(&c).map(|&i| self.codes[i])
    }
    pub fn index(&self, code: u32) -> Option<usize> {
        self.codes.iter().position(|&c| c as u32 == code)
    }
    pub fn tile(&self, index: usize, ink: u8) -> [u8; 32] {
        let mut pixels = [[0u8; 8]; 8];
        if self.shadow {
            for (y, &row) in self.rows[index].iter().enumerate() {
                for x in 0..self.width {
                    if row & (1 << (self.width - 1 - x)) != 0 {
                        pixels[y + 1][x + 1] = 1;
                    }
                }
            }
        }
        for (y, &row) in self.rows[index].iter().enumerate() {
            for x in 0..self.width {
                if row & (1 << (self.width - 1 - x)) != 0 {
                    pixels[y][x] = ink;
                }
            }
        }
        let mut tile = [0u8; 32];
        for y in 0..8 {
            for x in 0..8 {
                tile[y * 4 + x / 2] |= pixels[y][x] << (4 * (x % 2));
            }
        }
        tile
    }
    /// Settings labels use a fixed 5x7 host cell; fit larger package glyphs.
    pub fn ui_glyph(&self, c: char) -> Option<[u8; 7]> {
        let index = *self.characters.get(&c)?;
        let mut result = [0u8; 7];
        let width = self.width.min(5);
        let height = self.height.min(7);
        for y in 0..self.height {
            for x in 0..self.width {
                if self.rows[index][y] & (1 << (self.width - 1 - x)) != 0 {
                    result[y * height / self.height] |= 1 << (4 - x * width / self.width);
                }
            }
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn definition() -> Definition {
        serde_json::from_value(serde_json::json!({"width":3,"height":3,"glyphs":{"Ω":["###","#.#","#.#"]},"aliases":{"ω":"Ω"}})).unwrap()
    }
    #[test]
    fn arbitrary_alphabet_and_aliases_come_from_data() {
        let font = Font::compile("Ω", Some(definition())).unwrap();
        assert_eq!(font.encode('Ω'), font.encode('ω'));
        assert_eq!(font.encode('А'), None);
        assert!(font.tile(0, 3).iter().any(|&b| b != 0));
        assert_eq!(font.ui_glyph('Ω'), Some([28, 20, 20, 0, 0, 0, 0]));
        assert_eq!(private_codes().len(), 110);
    }
    #[test]
    fn bad_glyphs_dimensions_aliases_and_duplicate_keys_are_rejected() {
        assert!(Font::compile("ΩΩ", Some(definition())).is_err());
        assert!(Font::compile("ΩΨ", Some(definition())).is_err());
        let mut d = definition();
        d.width = 9;
        assert!(Font::compile("Ω", Some(d)).is_err());
        let mut d = definition();
        d.glyphs.get_mut("Ω").unwrap()[0] = "bad".into();
        assert!(Font::compile("Ω", Some(d)).is_err());
        let mut d = definition();
        d.aliases.insert("Ω".into(), "Ω".into());
        assert!(Font::compile("Ω", Some(d)).is_err());
        let mut d = definition();
        d.aliases.insert("ψ".into(), "Ψ".into());
        assert!(Font::compile("Ω", Some(d)).is_err());
        assert!(Font::compile("@", Some(definition())).is_err());
        assert!(serde_json::from_str::<Definition>(
            r##"{"width":1,"height":1,"glyphs":{"x":["#"],"x":["."]}}"##
        )
        .is_err());
    }
}
