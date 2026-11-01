"""Read GTA Advance language tables and prepare external UTF-8 text packs.

Only data is read. This tool never patches a ROM or changes game saves.
Table discovery was informed by GTA Advance Studio's pointer scanner; unlike
its heuristic grouping, this reader requires all five explicit language tags.
"""
from __future__ import annotations

import argparse
import collections
import copy
import hashlib
import json
from pathlib import Path
import re
import struct
import sys

LANGUAGES = ('en', 'es', 'fr', 'it', 'de')
USA_SHA = '215c574d5afbda0aadbf89dc76b765cfaaeb35953eab63f7687d8bfe8a8e0f4d'
EU_SHA = '48892e001bd86836d619a875b2a0605927cae9f922224686efa01bc7e470272a'
RU_SHA = 'e10f0f044ba3f119440f5bf2ddfe9aeaae67c288d3d82850237dd75e688f6d1e'
BASE = 0x08000000
MAX_TEXT_BYTES = 4096
UI_BANK = 0xec4324
UI_COUNT = 618
FORMAT = 'gta-advance-text-catalog-v1'
PACK_VERSION = 2
LEGACY_PACK_FORMAT = 'gta-advance-language-pack-v1'
MENU_TEXTS = json.loads((Path(__file__).resolve().parents[2] / 'resources/menu-en.json').read_text(encoding='utf-8'))
PROFILES = {USA_SHA: 'usa', EU_SHA: 'europe', RU_SHA: 'ru-custom-font'}


class TextError(ValueError):
    pass


def digest(data):
    return hashlib.sha256(data).hexdigest()


def load_json(path):
    def unique_pairs(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise TextError(f'Duplicate JSON key: {key}')
            result[key] = value
        return result
    return json.loads(Path(path).read_text(encoding='utf-8-sig'), object_pairs_hook=unique_pairs)


def write_json(path, value):
    path = Path(path)
    if path.exists():
        raise TextError(f'Output already exists: {path}; choose a new path')
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')


def read_rom(path):
    data = Path(path).read_bytes()
    if len(data) != 16 * 1024 * 1024 or data[0xac:0xb0] not in (b'BGTE', b'BGTP'):
        raise TextError('Expected a 16 MiB GTA Advance USA/Europe ROM')
    return data


def scan_tables(data):
    """Require five consecutive (language_id, ROM_pointer) pairs, IDs 0..4.

    Empty strings are retained: modified ROMs can intentionally empty slots.
    Table addresses identify records, even when multiple records share text.
    """
    groups = []
    for offset in range(0, len(data) - 39, 4):
        if data[offset:offset + 4] != b'\0\0\0\0':
            continue
        fields = struct.unpack_from('<10I', data, offset)
        if fields[::2] != (0, 1, 2, 3, 4):
            continue
        texts, pointers = {}, {}
        for language, pointer in zip(LANGUAGES, fields[1::2]):
            start = pointer - BASE
            if not 0 <= start < len(data):
                break
            end = data.find(b'\0', start, min(start + MAX_TEXT_BYTES + 1, len(data)))
            if end < 0:
                break
            texts[language] = data[start:end]
            pointers[language] = start
        if len(texts) == len(LANGUAGES):
            groups.append(dict(offset=offset, texts=texts, pointers=pointers))
    return groups


def inspect(data):
    groups = scan_tables(data)
    high = collections.Counter(byte for group in groups for raw in group['texts'].values() for byte in raw if byte >= 128)
    return dict(rom_sha256=digest(data), profile=PROFILES.get(digest(data), 'unknown'),
                game_code=data[0xac:0xb0].decode('ascii'), table_count=len(groups),
                languages=list(LANGUAGES),
                nonempty_by_language={lang: sum(bool(group['texts'][lang]) for group in groups) for lang in LANGUAGES},
                high_bytes={f'{byte:02X}': count for byte, count in sorted(high.items())},
                scope='Confirmed five-language tables only; not all menu/location strings',
                custom_encoding_required=digest(data) == RU_SHA)


def source_groups(data, usa_groups):
    """Known donor layouts only: never match translations by scan order alone."""
    sha = digest(data)
    if sha not in PROFILES:
        raise TextError('Unknown donor ROM hash: inspect it first and add a verified profile')
    shift = 0x3b0 if sha == EU_SHA else 0
    source = {group['offset']: group for group in scan_tables(data)}
    if len(source) != len(usa_groups):
        raise TextError('Donor table count differs from the verified USA structure')
    result = []
    for base in usa_groups:
        donor = source.get(base['offset'] + shift)
        if donor is None:
            raise TextError(f'Missing donor table for {base["offset"]:08X}')
        if sha == EU_SHA and donor['texts'] != base['texts']:
            raise TextError(f'Europe table content differs at {base["offset"]:08X}')
        result.append(donor)
    return result


def build_catalog(data):
    if digest(data) != USA_SHA:
        raise TextError('Catalog IDs must be created from the verified USA ROM')
    groups = scan_tables(data)
    entries = []
    for group in groups:
        texts = {}
        for language, raw in group['texts'].items():
            if any(byte < 0x20 and byte not in (10,13) or 0x7f <= byte < 0xa0 for byte in raw):
                raise TextError(f'Unexpected text control byte at {group["offset"]:08X}')
            texts[language] = raw.decode('latin-1')
        entries.append(dict(id=f'text.{group["offset"]:08x}',
                            table_offset=f'{group["offset"]:08x}',
                            string_offsets={lang: f'{ptr:08x}' for lang, ptr in group['pointers'].items()},
                            texts=texts))
    for index in range(UI_COUNT):
        pointers = {language: struct.unpack_from('<I',data,UI_BANK+(slot*UI_COUNT+index)*4)[0]-BASE for slot,language in enumerate(LANGUAGES)}
        texts = {}
        for language,pointer in pointers.items():
            if not 0<=pointer<len(data): raise TextError('Invalid UI bank pointer')
            end=data.find(b'\0',pointer,pointer+MAX_TEXT_BYTES+1)
            if end<0: raise TextError('Unterminated UI text')
            texts[language]=data[pointer:end].decode('latin-1')
        entries.append(dict(id=f'ui.{index:04x}', pointer_offsets={lang:f'{UI_BANK+(slot*UI_COUNT+index)*4:08x}' for slot,lang in enumerate(LANGUAGES)}, string_offsets={lang:f'{ptr:08x}' for lang,ptr in pointers.items()}, texts=texts))
    return dict(format=FORMAT, base_rom_sha256=USA_SHA, languages=list(LANGUAGES),
                scope='confirmed-story-and-ui-tables', entries=entries)


def check_catalog(catalog):
    if not isinstance(catalog, dict) or catalog.get('format') != FORMAT or catalog.get('base_rom_sha256') != USA_SHA:
        raise TextError('Expected a USA text catalog v1')
    if catalog.get('scope') not in ('confirmed-five-language-tables','confirmed-story-and-ui-tables'):
        raise TextError('Unsupported catalog scope')
    entries = catalog.get('entries')
    if not isinstance(entries, list) or not entries:
        raise TextError('Catalog entries must be a nonempty list')
    seen = set()
    for entry in entries:
        identifier = entry.get('id') if isinstance(entry, dict) else None
        if not isinstance(identifier, str) or not re.fullmatch(r'(?:text\.[0-9a-f]{8}|ui\.[0-9a-f]{4})', identifier) or identifier in seen:
            raise TextError(f'Invalid or duplicate catalog ID: {identifier}')
        seen.add(identifier)
        if not isinstance(entry.get('texts'), dict) or any(not isinstance(entry['texts'].get(lang), str) for lang in LANGUAGES):
            raise TextError(f'Missing language text: {identifier}')
    return {entry['id']: entry for entry in entries}


def legacy_font():
    return load_json(Path(__file__).resolve().parents[2] / 'resources/legacy-language-v1-font.json')


FONT_VARIANTS = ('antialiased_font', 'plain_font', 'dialogue_font', 'cutscene_font')


def copy_fonts(result, font_source):
    result['alphabet'] = copy.deepcopy(font_source.get('alphabet', ''))
    result['font'] = copy.deepcopy(font_source.get('font'))
    for key in ('color_font', *FONT_VARIANTS):
        if key in font_source:
            result[key] = copy.deepcopy(font_source[key])
        else:
            result.pop(key, None)


def validate_font_variants(pack):
    for key in FONT_VARIANTS:
        font = pack.get(key)
        if font is not None:
            try:
                validate_font(pack.get('alphabet', ''), font)
                if font.get('aliases'):
                    raise TextError('Define shared aliases in font.aliases')
            except TextError as error:
                raise TextError(f'{key}: {error}') from error


def upgrade_pack(pack, font_source=None):
    if not isinstance(pack, dict):
        raise TextError('Expected a language pack object')
    if 'version' in pack:
        if type(pack['version']) is not int or pack['version'] != PACK_VERSION:
            raise TextError('Unsupported language pack version')
        result = copy.deepcopy(pack)
    else:
        if pack.get('format') != LEGACY_PACK_FORMAT or pack.get('base_rom_sha256') != USA_SHA or pack.get('scope') != 'confirmed-five-language-tables':
            raise TextError('Unsupported legacy language pack')
        result = {key: copy.deepcopy(value) for key,value in pack.items() if key not in ('format', 'base_rom_sha256', 'scope')}
        result['version'] = PACK_VERSION
        result.update(copy.deepcopy(legacy_font()))
    if font_source is not None:
        copy_fonts(result, font_source)
    allowed = {'version','language','name','fallback','alphabet','font','color_font','entries','menu','source_rom_sha256','source_slot', *FONT_VARIANTS}
    if set(result)-allowed:
        raise TextError('Unknown pack fields: ' + ', '.join(sorted(set(result)-allowed)))
    if not isinstance(result.get('language'),str) or not re.fullmatch(r'[a-z]{2,3}(?:-[A-Za-z0-9]{2,8})*',result['language']):
        raise TextError('Invalid language tag')
    if not isinstance(result.get('name'),str) or not result['name'].strip() or result.get('fallback','en')!='en' or not isinstance(result.get('entries'),dict):
        raise TextError('Pack requires a name, entries and English fallback')
    validate_font(result.get('alphabet',''),result.get('font'))
    validate_color_font(result.get('alphabet',''),result.get('color_font'))
    validate_font_variants(result)
    return result


def validate_font(alphabet, font):
    if not isinstance(alphabet, str):
        raise TextError('Alphabet must be a Unicode string')
    if not alphabet and font is None:
        return {}
    if not isinstance(font, dict) or set(font)-{'width','height','shadow','glyphs','aliases'}:
        raise TextError('Expected a font object with width, height and glyphs')
    width,height = font.get('width'),font.get('height')
    shadow = font.get('shadow', False)
    if type(width) is not int or type(height) is not int or not 1 <= width <= 8 or not 1 <= height <= 8 or type(shadow) is not bool or shadow and (width == 8 or height == 8):
        raise TextError('Font must fit an 8x8 cell, including its optional shadow')
    glyphs,aliases = font.get('glyphs'),font.get('aliases', {})
    if not isinstance(glyphs, dict) or not isinstance(aliases, dict):
        raise TextError('Glyphs and aliases must be objects')
    if len(set(alphabet)) != len(alphabet) or len(alphabet) > 110:
        raise TextError('Alphabet must contain at most 110 unique characters')
    def character(value):
        if not isinstance(value,str) or len(value)!=1 or value in ' @%' or ord(value)<32 or 0x7f<=ord(value)<0xa0 or 0xd800<=ord(value)<=0xdfff:
            raise TextError(f'Invalid alphabet character: {value!r}')
    if set(glyphs) != set(alphabet):
        raise TextError('Font glyph keys must match alphabet exactly')
    mapping = {}
    for c in alphabet:
        character(c)
        rows = glyphs[c]
        if not isinstance(rows,list) or len(rows)!=height or any(not isinstance(row,str) or len(row)!=width or set(row)-{'.','#'} for row in rows):
            raise TextError(f'Glyph {c}: expected {height} rows of {width} . or # pixels')
        mapping[c]=c
    for alias,target in aliases.items():
        character(alias)
        character(target)
        if alias in mapping or target not in glyphs:
            raise TextError(f'Invalid or conflicting alias: {alias}')
        mapping[alias]=target
    return mapping


def validate_color_font(alphabet,font):
    if font is None:return
    if not isinstance(font,dict) or set(font)-{'width','height','advance','advances','palette','glyphs'}:
        raise TextError('Expected a color_font object with width, height and glyphs')
    width,height=font.get('width'),font.get('height')
    if type(width) is not int or type(height) is not int or not 1<=width<=16 or not 1<=height<=16:
        raise TextError('Color font must fit a 16x16 cell')
    palette=font.get('palette',list(range(16)))
    if not isinstance(palette,list) or len(palette)!=16 or palette[0]!=0 or any(type(p) is not int or not 0<=p<=15 for p in palette):
        raise TextError('Color font requires 16 native palette indices (0..15), with transparent index 0')
    advance=font.get('advance')
    if advance is None:advance=(width+3)&~1
    advances=font.get('advances',{})
    if not isinstance(advances,dict) or any(type(v) is not int or not 2<=v<=18 or v%2 for v in [advance,*advances.values()]):
        raise TextError('Color font advances must be even pixel counts from 2 to 18')
    glyphs=font.get('glyphs')
    if not isinstance(glyphs,dict) or set(glyphs)!=set(alphabet) or set(advances)-set(alphabet):
        raise TextError('Color font glyphs must match alphabet exactly')
    for c,rows in glyphs.items():
        if not isinstance(rows,list) or len(rows)!=height or any(not isinstance(row,str) or len(row)!=width or not re.fullmatch('[.0-9a-fA-F]+',row) for row in rows):
            raise TextError(f'Color glyph {c}: expected {height} rows of {width} hex pixels or dots')


def new_pack(catalog, language, name, source_language=None, font_source=None):
    entries = check_catalog(catalog)
    if not re.fullmatch(r'[a-z]{2,3}(?:-[A-Za-z0-9]{2,8})*', language):
        raise TextError('Language must be a tag such as ru, fr or pt-BR')
    result = dict(version=PACK_VERSION, language=language, name=name, alphabet='',
                  entries={key: entry['texts'][source_language] if source_language else '' for key, entry in entries.items()},
                  menu={key: value if source_language=='en' else '' for key,value in MENU_TEXTS.items()})
    if font_source is not None:
        copy_fonts(result, font_source)
    validate_font(result['alphabet'],result.get('font'))
    validate_color_font(result['alphabet'],result.get('color_font'))
    validate_font_variants(result)
    return result


def validate_pack(catalog, pack):
    entries = check_catalog(catalog)
    pack = upgrade_pack(pack)
    if not isinstance(pack.get('language'), str) or not re.fullmatch(r'[a-z]{2,3}(?:-[A-Za-z0-9]{2,8})*', pack['language']):
        raise TextError('Invalid language tag')
    if not isinstance(pack.get('name'), str) or not pack['name'].strip() or any(ord(c)<32 or 0x7f<=ord(c)<0xa0 for c in pack['name']) or pack.get('fallback','en') != 'en':
        raise TextError('Pack requires a name and English fallback')
    if not isinstance(pack.get('entries'), dict):
        raise TextError('Invalid pack entries')
    custom = validate_font(pack.get('alphabet',''),pack.get('font'))
    translated, warnings, runtime_errors = 0, [], []
    non_latin1 = set()
    for identifier, text in pack['entries'].items():
        if identifier not in entries:
            raise TextError(f'Unknown text ID: {identifier}')
        if not isinstance(text, str):
            raise TextError(f'Text must be a string: {identifier}')
        if any(ord(char) < 0x20 and char not in '\n\r' or 0x7f <= ord(char) < 0xa0 or 0xd800 <= ord(char) <= 0xdfff for char in text):
            raise TextError(f'Unsupported control character: {identifier}')
        if len(text.encode('utf-8')) > 65535:
            raise TextError(f'Text exceeds 65535 UTF-8 bytes: {identifier}')
        if not text:
            continue
        translated += 1
        original = entries[identifier]['texts']['en']
        tokens = lambda value: sorted(token for token in re.findall(r'@[0-9]|%(?:[0-9]+|[A-Z])', value) if token not in ('@0', '@8', '@9'))
        native_source=text in entries[identifier]['texts'].values()
        if tokens(text) != tokens(original) and not native_source:
            raise TextError(f'Preserve dynamic @ and % tokens: {identifier}')
        if len(text) > 609:
            runtime_errors.append(dict(id=identifier, reason='runtime-limit-609-characters'))
        for character in text:
            if character in custom:
                non_latin1.add(character)
                continue
            if native_source:
                continue
            if character in '\n\r’‘“”—–' or character.isascii() and (character.isalnum() or character in ' -_$!?\"\',.+/:;()%@') or character in 'ÜÑÉ¿ÁÓÈÍ¡ÖÀÙÄÚÌÒÏ':
                continue
            runtime_errors.append(dict(id=identifier, reason=f'missing-glyph-U+{ord(character):04X}'))
            break
        if re.search(r'@(?![0123489])', text):
            runtime_errors.append(dict(id=identifier, reason='unsupported-@-control-marker'))
        # A conservative warning, not a guessed reconstruction of game layout.
        if len(text) > len(original) * 1.5 + 20:
            warnings.append(dict(id=identifier, reason='longer-than-source', source_length=len(original), target_length=len(text)))
    menu=pack.get('menu',{})
    if not isinstance(menu,dict):raise TextError('menu must be an object')
    for identifier,text in menu.items():
        if identifier not in MENU_TEXTS:raise TextError(f'Unknown menu ID: {identifier}')
        if not isinstance(text,str) or len(text)>512 or any(ord(c)<32 or 0x7f<=ord(c)<0xa0 for c in text):raise TextError(f'Invalid menu text: {identifier}')
        if not text:continue
        placeholders=lambda s:sorted(re.findall(r'\{([^}]+)\}',s))
        if placeholders(text)!=placeholders(MENU_TEXTS[identifier]):raise TextError(f'Preserve menu placeholders: {identifier}')
        for char in text:
            if char in custom:non_latin1.add(char);continue
            char={'’':"'",'‘':"'",'“':'"','”':'"','—':'-','–':'-'}.get(char,char)
            for c in char.upper():
                if c in custom:non_latin1.add(c)
                elif c!='\n' and not 32<=ord(c)<=255:raise TextError(f'Menu font has no glyph for {c}: {identifier}')
    return dict(valid=True, entries=len(entries), translated=translated,
                menu_translated=sum(bool(v) for v in menu.values()),
                fallback_entries=len(entries)-translated,
                requires_extended_font=bool(non_latin1), extended_characters=''.join(sorted(non_latin1)),
                custom_glyphs=len(pack.get('alphabet','')),
                warnings=warnings, runtime_supported=not runtime_errors, runtime_errors=runtime_errors,
                note='Custom alphabets and glyphs come from the pack; 8x8 cells, 110 custom glyphs and 609 characters per entry')


def import_rom(usa, donor, catalog, language, name, slot, charmap=None):
    base = scan_tables(usa)
    source = source_groups(donor, base)
    if digest(donor) == RU_SHA and charmap is None:
        raise TextError('This Russian ROM uses a custom font encoding. Supply a verified --charmap; Latin-1/CP1251 would produce incorrect text')
    mapping = None
    if charmap is not None:
        if not isinstance(charmap, dict):
            raise TextError('Charmap must be a JSON object')
        mapping = {}
        for key, value in charmap.items():
            if not re.fullmatch(r'[0-9A-Fa-f]{2}', key) or not isinstance(value, str) or len(value) != 1:
                raise TextError('Charmap must map two-digit hex bytes to single Unicode characters')
            mapping[int(key,16)] = value
    pack = new_pack(catalog, language, name)
    for original, translated in zip(base, source):
        raw = translated['texts'][slot]
        if mapping is None:
            text = raw.decode('latin-1')
        else:
            missing = sorted({byte for byte in raw if byte >= 128 and byte not in mapping})
            if missing:
                raise TextError('Unmapped source bytes: ' + ', '.join(f'{byte:02X}' for byte in missing))
            text = ''.join(mapping.get(byte, chr(byte)) for byte in raw)
        pack['entries'][f'text.{original["offset"]:08x}'] = text
    bank=UI_BANK+(0x3b0 if digest(donor)==EU_SHA else 0)
    slot_index=LANGUAGES.index(slot)
    for index in range(UI_COUNT):
        pointer=struct.unpack_from('<I',donor,bank+(slot_index*UI_COUNT+index)*4)[0]-BASE
        if not 0<=pointer<len(donor): raise TextError('Invalid donor UI pointer')
        end=donor.find(b'\0',pointer,pointer+MAX_TEXT_BYTES+1)
        if end<0: raise TextError('Unterminated donor UI string')
        raw=donor[pointer:end]
        if mapping is None:
            text=raw.decode('latin-1')
        else:
            if any(byte>=128 and byte not in mapping for byte in raw): raise TextError('Unmapped donor UI bytes')
            text=''.join(mapping.get(byte,chr(byte)) for byte in raw)
        pack['entries'][f'ui.{index:04x}']=text
    pack['source_rom_sha256'] = digest(donor)
    pack['source_slot'] = slot
    validate_pack(catalog, pack)
    return pack


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    probe = commands.add_parser('inspect', help='Report tables, languages and source profile')
    probe.add_argument('rom', type=Path)
    probe.add_argument('--output', type=Path)
    extract = commands.add_parser('extract', help='Extract the five-language USA catalog')
    extract.add_argument('rom', type=Path)
    extract.add_argument('output', type=Path)
    template = commands.add_parser('new-pack', help='Create an empty UTF-8 translation template')
    template.add_argument('catalog', type=Path)
    template.add_argument('output', type=Path)
    template.add_argument('--language', required=True)
    template.add_argument('--name', required=True)
    template.add_argument('--copy-language', choices=LANGUAGES)
    template.add_argument('--font-from',type=Path,help='Copy alphabet and font from another pack or font definition')
    upgrade = commands.add_parser('upgrade-pack',help='Convert a legacy language pack to version 2')
    upgrade.add_argument('pack',type=Path)
    upgrade.add_argument('output',type=Path)
    upgrade.add_argument('--font-from',type=Path)
    validate = commands.add_parser('validate', help='Check an authored pack against its catalog')
    validate.add_argument('catalog', type=Path)
    validate.add_argument('pack', type=Path)
    validate.add_argument('--output', type=Path)
    donor = commands.add_parser('import-rom', help='Map a known donor ROM to stable USA text IDs')
    donor.add_argument('usa', type=Path)
    donor.add_argument('donor', type=Path)
    donor.add_argument('output', type=Path)
    donor.add_argument('--language', required=True)
    donor.add_argument('--name', required=True)
    donor.add_argument('--slot', choices=LANGUAGES, required=True)
    donor.add_argument('--charmap', type=Path)
    donor.add_argument('--font-from', type=Path)
    args = parser.parse_args()
    if args.command == 'inspect':
        result = inspect(read_rom(args.rom))
        if args.output: write_json(args.output, result)
    elif args.command == 'extract':
        result = build_catalog(read_rom(args.rom))
        write_json(args.output, result)
        result = dict(entries=len(result['entries']), output=str(args.output))
    elif args.command == 'new-pack':
        result = new_pack(load_json(args.catalog), args.language, args.name, args.copy_language,
                          load_json(args.font_from) if args.font_from else None)
        write_json(args.output, result)
        result = dict(entries=len(result['entries']), output=str(args.output))
    elif args.command == 'upgrade-pack':
        result = upgrade_pack(load_json(args.pack),load_json(args.font_from) if args.font_from else None)
        write_json(args.output,result)
        result = dict(version=PACK_VERSION,output=str(args.output))
    elif args.command == 'validate':
        result = validate_pack(load_json(args.catalog), load_json(args.pack))
        if args.output: write_json(args.output, result)
    else:
        usa = read_rom(args.usa)
        catalog = build_catalog(usa)
        result = import_rom(usa, read_rom(args.donor), catalog, args.language, args.name, args.slot,
                            load_json(args.charmap) if args.charmap else None)
        if args.font_from:
            result=upgrade_pack(result,load_json(args.font_from))
            validate_pack(catalog,result)
        write_json(args.output, result)
        result = dict(entries=len(result['entries']), output=str(args.output))
    print(json.dumps(result, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    if hasattr(sys.stdout, 'reconfigure'):
        sys.stdout.reconfigure(encoding='utf-8')
    try:
        main()
    except (TextError, OSError, json.JSONDecodeError) as error:
        print(f'Error: {error}', file=sys.stderr)
        sys.exit(1)
