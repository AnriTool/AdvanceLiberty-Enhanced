# Text language packs

**English** | [Русский](../ru/LOCALIZATION.md)

Execution still uses the verified USA ROM. Images, splash screens and textures
are not translated by this system. The tool prepares external UTF-8 packs;
the runtime discovers them in `languages/` and loads the pack selected in
Enhanced Options. Strings and fonts change only in memory; the ROM file and
native DLLs remain unchanged.

## Using packs in the game

On the first normal launch, the game creates `languages/en.json`, a v2 pack
named **English (editable)** containing all 1774 recognized English entries.
Edit it directly or copy it to another filename and change `language`, `name`,
text and fonts. Select the pack in settings and save to see edits. Creating
the file does not automatically select its language.

Existing values in en.json are preserved. As the catalog grows, only missing
IDs are added; malformed JSON is left intact. Removing or moving the file
allows a fresh original to be generated on the next launch. Replays and
headless checks do not create it. Write errors are logged without preventing
startup. Generated game text remains local and is not committed to Git.

The catalog includes 1156 five-language story tables and a separate 618-entry
bank covering rampages, paramedic, firefighter, police and taxi tasks, menus,
statistics and names. Text drawn into images is excluded.

From the project directory:

```powershell
.\Play.cmd
```

To select a pack:

1. Put the JSON file directly in `languages/`; subdirectories are not scanned.
2. Start the game and open **Esc → Enhanced Options → Language**.
3. Select a pack and press Enter. Esc returns to the main settings screen.
4. Choose **Save and return** to apply it to the current game.

Changing language preserves CPU state, devices, timers, progress and audio in
memory. The game does not restart and no intermediate save states are created.
An already composed message may remain until the next normal redraw or line;
new text and glyphs come from the selected pack. Old text arenas stay alive for
in-flight accesses, and selecting the same pack again reuses its arena.
**Original game / ROM** disables the external pack and retains the game's own
language selection. Discard and return or Esc on the main settings screen
cancels edits. Selection is stored in `data/config/enhanced.cfg`, for example
`Language = ru-demo.json`; an older file without this entry means Original.

The pack list is refreshed at startup and when opening settings; each pack is
validated in full. New JSON files can be added without restarting. Selecting
the active language again and saving rereads the file, allowing live translation
edits. Unsupported, malformed or incompatible packs are skipped with a logged
explanation. If a saved pack is removed or fails validation, startup uses the
ROM's original language. Filenames are stored without absolute paths so the
game folder remains portable. Discovery is independent of the working directory:
development builds use the project's root `languages/`; portable builds use
`languages/` beside the EXE.

To create a translation, copy languages/en.json to a new name, change `language`
and `name`, add an alphabet/font, then translate `entries`. New UI-bank IDs look
like `ui.015b`; existing `text.00cabfac` IDs are retained.

For diagnostics, launch-script option `-LanguagePack PATH` overrides the saved
choice for one run without changing preferences. The runtime equivalent is
`--language-pack PATH`. Packs require the exact USA ROM SHA-256 and AOT
libraries using one instruction per block. Interpreter mode and differential
block checking are rejected when a pack is active.

Packs work independently of the original game's selected language: all five
pointers in a table point to the translation or its English fallback. Translation
data is appended after the original 16 MiB in the virtual cartridge address
space. Original EEPROM addressing is preserved separately from that size, so
additional text does not change save visibility.

Original Latin letters, accents, button icons and punctuation are preserved.
Other characters, pixel glyphs and case mappings are defined by the JSON pack,
without Rust changes or rebuilding. One pack can mix alphabets. Fonts are added
to UI atlases and dialogue-window variants. Cutscenes have separate pixels and
proportional widths; copies are generated from `font` while retaining original
Latin glyphs. The original character parser runs natively; its return handling
is extended for private glyph codes. Original apostrophe, Spanish ¡ and other
standard character bytes are not reused.

The regular font currently supports up to **110 custom glyphs**, each fitting
an **8×8** cell including any optional shadow. Original game logic controls
character spacing and writing direction; complex typography, ligatures and
right-to-left text are not implemented. This is a pixel-font format, not a TTF
loader. The Russian example has 33 5×7 glyphs and explicit lowercase-to-uppercase
aliases. Other languages receive no automatic case conversion: add required
forms to `alphabet` or `font.aliases`.

The current experimental limit is 609 characters per entry. Translations may
be longer than their originals; the game performs word wrapping. This limit
does not guarantee suitable layout in every window. Long dialogue, pages and
timing constraints still need checks per text category. Characters without
either a glyph or a standard representation are rejected with an explanation.

Session recording copies exact pack bytes, including alphabet/font data, to
`language-pack.json` in the recording directory. replay.ps1 and the runtime
automatically read that snapshot beside input.gba-input, so later edits to the
source pack do not change old replays. If metadata requires a pack but its
snapshot is missing, replay fails explicitly. Saved user language preferences
do not apply to replays; the recording's pack, or an explicit diagnostic override,
takes precedence. Live changes are recorded with their frame and full JSON
snapshot in `# language` lines of input.gba-input. No extra state files are
created. Replay applies the same changes independently of today's languages/.
Diagnostic `--text-trace NEW.csv` records table accesses with frames, IDs and
language slots; an adjacent `NEW.glyphs.csv` records font diagnostics.

## Text sources in the ROM

The USA ROM contains 1156 verified tables, each holding five `(language, pointer)`
pairs: 0 English, 1 Español, 2 Français, 3 Italiano, 4 Deutsch. This totals
5780 language entries, including mission dialogue and MISSION SUCCESS /
MISSION FAILED messages. Strings are uncompressed, NUL-terminated, single-byte
Latin-1.

A separate UI bank contains five arrays of 618 pointers, each array sized
`0x9A8`. The game reads it through `0805e330`. IDs `ui.0000`…`ui.0269` refer
to the same index across all five languages. Unlike a generic string search,
the tool does not treat every pointer-like sequence as a language table or
include internal mission/object names as translatable text.

The supplied European ROM has the same 1156 tables shifted by `0x3B0`.
Import checks found matching bytes for all five languages in every table;
these tables add no translations beyond those already in USA.

The supplied ru.gba retains the table structure. Its English slot has 1152
nonempty entries; Spanish/French/Italian have 0 and German has 6. Font pixels
are changed. Strings such as `CAÊAÛ` mix ASCII with custom character codes.
Reading them as Latin-1 or Windows-1251 does not recover Russian text. This
explains Advance Studio's incorrect-looking output; the text alone does not
establish ROM corruption.

Import with an explicit character map is supported, but a verified map for
that Russian font has not been prepared. Automatic import without one is
rejected to avoid producing a corrupted pack.

## Tool

`tools/localization/texts.py` requires Python 3.10+ with no external dependencies.
Run the following commands from the project root:

```powershell
# Identify a known revision, table count and populated language slots
python tools/localization/texts.py inspect roms/usa.gba
python tools/localization/texts.py inspect roms/ru.gba

# Extract a catalog with all five original languages
python tools/localization/texts.py extract roms/usa.gba artifacts/text/usa.json

# Create a blank Russian template with the example alphabet and font
python tools/localization/texts.py new-pack artifacts/text/usa.json artifacts/text/ru.json --language ru --name "Русский" --font-from resources/legacy-language-v1-font.json

# Validate the edited pack
python tools/localization/texts.py validate artifacts/text/usa.json artifacts/text/ru.json

# Create a pack from a built-in language
python tools/localization/texts.py new-pack artifacts/text/usa.json artifacts/text/fr.json --language fr --name "Français" --copy-language fr

# Import French from the known European ROM and map it to USA
python tools/localization/texts.py import-rom roms/usa.gba roms/europe.gba artifacts/text/fr-europe.json --language fr --name "Français" --slot fr

# Upgrade an old v1 pack to a self-contained v2 pack
python tools/localization/texts.py upgrade-pack artifacts/text/old-ru.json artifacts/text/ru-v2.json
```

Existing output files are not overwritten; select a new path for another
export. Unknown ROMs may be inspected diagnostically, but cannot be automatically
mapped to USA.

## Pack format

`text.00cabf84` identifies a USA table entry, not the address of one language's
string. Identical dialogue in different tables retains different IDs. IDs are
bound to the exact original USA ROM SHA-256; a different base ROM requires an
explicit mapping.

A minimal partial pack using the standard Latin font:

```json
{
  "version": 2,
  "language": "en-custom",
  "name": "Custom text",
  "entries": {
    "text.00cabfac": "TRY AGAIN"
  }
}
```

For a custom alphabet, add `alphabet` and `font`. This single Greek glyph
can be used in both text and the pack name:

```json
{
  "version": 2,
  "language": "el-example",
  "name": "Omega Ω",
  "alphabet": "Ω",
  "font": {
    "width": 5,
    "height": 7,
    "shadow": false,
    "glyphs": {
      "Ω": [".###.", "#...#", "#...#", "#...#", ".#.#.", ".#.#.", "##.##"]
    },
    "aliases": {"ω": "Ω"}
  },
  "entries": {"text.00cabfac": "Ω ω"}
}
```

`alphabet` lists unique Unicode characters; `glyphs` must contain exactly
their drawings. In each row, `#` is a pixel and `.` is transparent. There must
be `height` rows of `width` characters each. `shadow: true` adds a shadow one
pixel right and down, limiting width/height to 7. Without a shadow, dimensions
from 1 to 8 are valid. Optional `aliases` maps each source character directly
to one alphabet glyph, without alias chains or extra cells. Spaces, control
characters, `@` and `%` are reserved. Latin glyphs may also be overridden.

A ready Russian alphabet/font is in `resources/legacy-language-v1-font.json`.
Another alphabet example is `tools/localization/examples/greek-symbols.json`.
The language name in settings uses the same font; larger glyphs are fitted
into a 5×7 cell for menus.

### Color font for menus and statistics

Optional `color_font` supplies separate drawings for the original 16×16 color
font. It shares `alphabet` and `font.aliases`, but drawings and character
spacing are independent of the regular `font`. A complete example is in
`languages/ru.json`.

- `width`, `height`: drawing dimensions from 1 to 16 inside a 16×16 cell.
- `glyphs`: exactly one drawing per alphabet character. Rows contain hexadecimal
  indices `0`…`f`; `.` is equivalent to transparent `0`.
- `palette`: 16 indices into the game's original shades, from 0 to 15. The
  first is always 0. Replacing entries 1, 2 and 3 with 15, for example, makes
  the fill white while retaining outline and shadow. The game still selects
  its menu palette and highlights.
- `advance`: character spacing in pixels, an even value from 2 to 18. The
  default is drawing width plus two pixels, rounded up to an even value.
- `advances`: optional per-character spacing overrides; keys belong to
  `alphabet` and values follow the same limits.

The Russian set has 10×16 drawings and advance 10. Original Latin glyphs,
digits and metrics are retained. Without color_font, color glyphs are generated
from the pack's regular font, preserving older packs. `new-pack --font-from`
and `upgrade-pack --font-from` also copy color_font when present in the source.

The font applies to native menu/statistics output and live language changes.
After editing JSON, reselect the active language and choose Save and return.
Already drawn game strings update on their next normal redraw.

`version` is an integer schema version, currently **2**. String field `format`
is no longer used. Old `scope` identified the verified five-language tables
and `base_rom_sha256` pinned the USA ROM. New packs omit them: the runtime
validates the exact USA ROM and allowed IDs itself. The technical extraction
catalog still carries ROM metadata. Optional `source_rom_sha256` and
`source_slot` record an imported translation's provenance and are unnecessary
for manual translation.

Old v1 packs and recording snapshots still load. Their immutable legacy font
remains in resources/legacy-language-v1-font.json; new packs do not depend on
it. upgrade-pack embeds that font in the output v2 pack.

An empty or missing string uses the English fallback. Optional `fallback`
accepts only `en`, already the default. Files use UTF-8; translated length is
not constrained by the original ROM space. Validation rejects incompatible
base ROMs, unknown IDs, duplicate JSON keys, unknown versions, invalid glyphs
or aliases, and unsupported controls. Long strings produce a warning. Characters
are retained rather than replaced by `?`; validation reports
`requires_extended_font: true` and `custom_glyphs` separately. `runtime_supported`
reflects current runtime limits; `runtime_errors` lists oversized strings and
unsupported characters. A structurally valid pack with these errors remains
editable, but the game will refuse to load it. Validation also requires preserving
dynamic `%...` substitutions and button icons `@1`…`@4` from the original English
entry; order may change. Highlight codes `@0`, `@8` and `@9` may differ, as they
do in original translations. Final layout must be checked in the game.

For a nonstandard ROM, `--charmap FILE.json` accepts a map such as
`{"CA": "М"}`: two-digit byte code → one Unicode character. Every encountered
non-ASCII byte must be mapped. ASCII is preserved unless overridden. Incomplete
maps are rejected. This is a mechanism for a future verified decoder, not a
ready-made Russian ROM importer.

## Translating the host menu

A v2 pack can add an optional `menu` object beside `entries`:

```json
"menu": {
  "quit_to_menu": "В главное меню",
  "quit_to_desktop": "На рабочий стол",
  "quit_to_desktop_action": "На рабочий стол...",
  "save_failed": "Ошибка сохранения: {error}"
}
```

This is an excerpt, not a standalone file. The complete stable-key catalog
with English source labels is `resources/menu-en.json`; a partial Russian
example is `resources/menu-ru-example.json`. Translatable areas include pause,
Options lists, PC preferences, control bindings, hints and the exit dialog.
Original Brief/Stats/Credits and startup pages continue using game translations
from `entries`.

Empty or missing values use the original label. For original pause entries,
an `entries` translation is retained unless explicitly overridden by `menu`;
the renamed quit_to_menu defaults to Quit to Menu. Old packs without menu
still load. On launch, languages/en.json gains missing menu keys without
overwriting author edits. New-pack generation also includes the menu section.

Labels use the game font. Cyrillic and other extra characters come from the
selected pack's alphabet, font.glyphs, font.aliases and color_font. The menu
section does not add a font by itself; use a pack with Russian glyphs for the
Russian example. Keep labels short and single-line: long strings are clipped
to the available width. The limit is 512 characters; control characters are
forbidden. Unknown keys and missing glyphs are rejected. Preserve `{error}`
and `{file}` placeholders from English templates, although their position may
change. Game-string `%...` and `@1` syntax does not apply to this section.

`validate-pack` checks both sections and reports `menu_translated`. To apply
edits without restarting, reselect the pack in Enhanced Options → Language
and choose Save and return.
