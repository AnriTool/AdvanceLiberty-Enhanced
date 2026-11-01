//! Package-owned alphabets, monochrome glyphs and indexed-color menu glyphs.
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

fn identity_palette() -> Vec<u8> {
    (0..16).collect()
}
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ColorDefinition {
    pub width: u8,
    pub height: u8,
    #[serde(default = "identity_palette")]
    pub palette: Vec<u8>,
    #[serde(default)]
    pub advance: Option<u8>,
    #[serde(default, deserialize_with = "unique_map")]
    pub advances: BTreeMap<String, u8>,
    #[serde(deserialize_with = "unique_map")]
    pub glyphs: BTreeMap<String, Vec<String>>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct ColorFont {
    #[serde(with = "replay_tiles")]
    pub tiles: Vec<[u8; 256]>,
    pub widths: Vec<u8>,
}
impl ColorFont {
    pub fn compile(
        alphabet: &str,
        mono: &Font,
        definition: Option<ColorDefinition>,
    ) -> Result<Self, String> {
        let Some(definition) = definition else {
            return Ok(Self::generated(mono));
        };
        let width = definition.width as usize;
        let height = definition.height as usize;
        if !(1..=16).contains(&width) || !(1..=16).contains(&height) {
            return Err("Color font must fit a 16x16 cell".into());
        }
        if definition.palette.len() != 16
            || definition.palette[0] != 0
            || definition.palette.iter().any(|&p| p > 15)
        {
            return Err("Color font palette requires 16 native shade indices (0..15), with transparent index 0".into());
        }
        let advance = definition.advance.unwrap_or((definition.width + 3) & !1);
        if !(2..=18).contains(&advance)
            || advance % 2 != 0
            || definition
                .advances
                .values()
                .any(|&v| !(2..=18).contains(&v) || v % 2 != 0)
        {
            return Err("Color font advances must be even pixel counts from 2 to 18".into());
        }
        if definition.glyphs.len() != mono.len()
            || definition
                .advances
                .keys()
                .any(|c| !definition.glyphs.contains_key(c))
        {
            return Err("Color font glyphs must match alphabet exactly".into());
        }
        let mut result = Self::default();
        for c in alphabet.chars() {
            let key = c.to_string();
            let rows = definition
                .glyphs
                .get(&key)
                .ok_or_else(|| format!("Missing color glyph: {c}"))?;
            if rows.len() != height {
                return Err(format!("Color glyph {c}: expected {height} rows"));
            }
            let mut tile = [0u8; 256];
            for (y, row) in rows.iter().enumerate() {
                if row.len() != width || !row.bytes().all(|b| b == b'.' || b.is_ascii_hexdigit()) {
                    return Err(format!(
                        "Color glyph {c}: expected {width} hex pixels or '.'"
                    ));
                }
                for (x, byte) in row.bytes().enumerate() {
                    let shade = if byte == b'.' {
                        0
                    } else {
                        (byte as char).to_digit(16).unwrap() as usize
                    };
                    tile[Self::offset(x, y)] = definition.palette[shade];
                }
            }
            result.tiles.push(tile);
            result
                .widths
                .push(*definition.advances.get(&key).unwrap_or(&advance) - 2);
        }
        Ok(result)
    }
    pub fn offset(x: usize, y: usize) -> usize {
        (y / 8 * 2 + x / 8) * 64 + (y % 8) * 8 + x % 8
    }
    /// Compatibility fallback for packs authored before color_font existed.
    /// It scales the package's own shapes; no alphabet is built into Rust.
    pub fn generated(mono: &Font) -> Self {
        let mut result = Self::default();
        for index in 0..mono.len() {
            let mut fill = [[false; 16]; 16];
            for y in 0..12 {
                for x in 0..8 {
                    let sx = x * mono.width / 8;
                    let sy = y * mono.height / 12;
                    fill[y + 3][x + 1] = mono.rows[index][sy] & (1 << (mono.width - 1 - sx)) != 0;
                }
            }
            let mut pixels = [[0u8; 16]; 16];
            for y in 0..16 {
                for x in 0..16 {
                    if fill[y][x] {
                        for dy in -1i32..=1 {
                            for dx in -1i32..=1 {
                                let nx = x as i32 + dx;
                                let ny = y as i32 + dy;
                                if (0..16).contains(&nx) && (0..16).contains(&ny) {
                                    pixels[ny as usize][nx as usize] = 8;
                                }
                            }
                        }
                        if y < 15 && x < 15 {
                            pixels[y + 1][x + 1] = 9;
                        }
                    }
                }
            }
            let mut tile = [0u8; 256];
            for y in 0..16 {
                for x in 0..16 {
                    if fill[y][x] {
                        pixels[y][x] = if y < 7 {
                            1
                        } else if y < 11 {
                            2
                        } else {
                            3
                        };
                    }
                    tile[Self::offset(x, y)] = pixels[y][x];
                }
            }
            result.tiles.push(tile);
            result.widths.push(8);
        }
        result
    }
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

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Font {
    width: usize,
    height: usize,
    shadow: bool,
    rows: Vec<Vec<u8>>,
    characters: BTreeMap<char, usize>,
    codes: Vec<u8>,
}

/// Optional drawings for each small-text renderer. All slots use the base
/// font's alphabet/encoding; omitted drawings fall back directly to that font.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct FontVariants {
    pub antialiased: Option<Font>,
    pub plain: Option<Font>,
    pub dialogue: Option<Font>,
    pub cutscene: Option<Font>,
}

pub fn compile_variant(
    alphabet: &str,
    name: &str,
    definition: Option<Definition>,
) -> Result<Option<Font>, String> {
    let Some(definition) = definition else {
        return Ok(None);
    };
    if !definition.aliases.is_empty() {
        return Err(format!("{name}: define shared aliases in font.aliases"));
    }
    Font::compile(alphabet, Some(definition))
        .map(Some)
        .map_err(|error| format!("{name}: {error}"))
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
    #[test]
    fn indexed_color_pixels_palette_and_spacing_come_from_data() {
        let mono = Font::compile("Ω", Some(definition())).unwrap();
        let mut rows = vec!["................".to_string(); 16];
        rows[9] = ".........a......".into();
        let mut palette: Vec<u8> = (0..16).collect();
        palette[10] = 14;
        let value = serde_json::json!({"width":16,"height":16,"advance":6,"palette":palette,"glyphs":{"Ω":rows}});
        let definition: ColorDefinition = serde_json::from_value(value.clone()).unwrap();
        let font = ColorFont::compile("Ω", &mono, Some(definition)).unwrap();
        assert_eq!(font.tiles[0][ColorFont::offset(9, 9)], 14);
        assert_eq!(font.tiles[0].iter().filter(|&&p| p != 0).count(), 1);
        assert_eq!(font.widths[0] + 2, 6);
        for (key, bad) in [
            ("width", serde_json::json!(17)),
            ("advance", serde_json::json!(7)),
            ("palette", serde_json::json!([0, 1])),
        ] {
            let mut invalid = value.clone();
            invalid[key] = bad;
            assert!(
                ColorFont::compile("Ω", &mono, Some(serde_json::from_value(invalid).unwrap()))
                    .is_err()
            );
        }
        let mut bad = value;
        bad["glyphs"]["Ω"][0] = "????????????????".into();
        assert!(
            ColorFont::compile("Ω", &mono, Some(serde_json::from_value(bad).unwrap())).is_err()
        );
        assert_eq!(ColorFont::generated(&mono).tiles.len(), 1);
    }
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

// Serde supports fixed arrays up to 32; font tiles are 16x16 indexed pixels.
mod replay_tiles {
    pub fn serialize<S: serde::Serializer>(
        tiles: &[[u8; 256]],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serde::Serialize::serialize(
            &tiles.iter().map(|t| t.as_slice()).collect::<Vec<_>>(),
            serializer,
        )
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<[u8; 256]>, D::Error> {
        let tiles = <Vec<Vec<u8>> as serde::Deserialize>::deserialize(deserializer)?;
        tiles
            .into_iter()
            .map(|tile| {
                tile.try_into()
                    .map_err(|_| serde::de::Error::custom("Invalid font tile"))
            })
            .collect()
    }
}
