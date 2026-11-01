import copy
import importlib.util
import json
from pathlib import Path
import struct
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('texts', Path(__file__).with_name('texts.py'))
texts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(texts)


def catalog():
    return dict(format=texts.FORMAT, base_rom_sha256=texts.USA_SHA,
                scope='confirmed-five-language-tables', entries=[
                    dict(id='text.00cabf84', texts={lang: 'MISSION SUCCESS' for lang in texts.LANGUAGES}),
                    dict(id='text.00cabfac', texts={lang: 'MISSION FAILED' for lang in texts.LANGUAGES})])


class Tables(unittest.TestCase):
    def fixture(self):
        data = bytearray(256)
        for index in range(5):
            struct.pack_into('<II', data, 16 + index*8, index, texts.BASE + 100 + index*8)
            data[100+index*8:103+index*8] = b'TXT'
        return data

    def test_structure_and_empty_translation(self):
        data = self.fixture()
        data[108:111] = b'\0\0\0'
        groups = texts.scan_tables(data)
        self.assertEqual(len(groups), 1)
        self.assertEqual(groups[0]['offset'], 16)
        self.assertEqual(groups[0]['texts']['en'], b'TXT')
        self.assertEqual(groups[0]['texts']['es'], b'')

    def test_false_pointer_run_is_not_a_language_table(self):
        data = self.fixture()
        struct.pack_into('<I', data, 32, 7)
        self.assertEqual(texts.scan_tables(data), [])

    def test_bad_pointer_and_missing_terminator(self):
        data = self.fixture()
        struct.pack_into('<I', data, 20, texts.BASE + len(data))
        self.assertEqual(texts.scan_tables(data), [])
        data = self.fixture()
        struct.pack_into('<I', data, 20, texts.BASE + 250)
        data[250:] = b'XXXXXX'
        self.assertEqual(texts.scan_tables(data), [])

    def test_shared_string_does_not_merge_table_ids(self):
        data = self.fixture()
        data[56:96] = data[16:56]
        self.assertEqual([g['offset'] for g in texts.scan_tables(data)], [16, 56])

    def test_unknown_rom_cannot_assign_usa_ids(self):
        with self.assertRaises(texts.TextError):
            texts.build_catalog(self.fixture())
        with self.assertRaises(texts.TextError):
            texts.source_groups(self.fixture(), [])


class Packs(unittest.TestCase):
    def test_small_font_variants_copy_validate_and_allow_base_fallback(self):
        for key in texts.FONT_VARIANTS:
            variant = copy.deepcopy(self.pack['font'])
            variant.pop('aliases', None)
            self.pack[key] = variant
        upgraded = texts.upgrade_pack(self.pack)
        self.assertEqual(upgraded['dialogue_font'], self.pack['dialogue_font'])
        created = texts.new_pack(self.catalog, 'ru', 'Russian', font_source=self.pack)
        for key in texts.FONT_VARIANTS:
            self.assertEqual(created[key], self.pack[key])
            bad = copy.deepcopy(self.pack)
            bad[key]['glyphs'] = {}
            with self.assertRaisesRegex(texts.TextError, key):
                texts.upgrade_pack(bad)
            bad = copy.deepcopy(self.pack)
            bad[key]['aliases'] = {'а': 'А'}
            with self.assertRaisesRegex(texts.TextError, key):
                texts.upgrade_pack(bad)
        # A replacement donor without overrides restores the base fallback.
        replaced = texts.upgrade_pack(self.pack, texts.legacy_font())
        self.assertTrue(all(key not in replaced for key in texts.FONT_VARIANTS))
        self.assertTrue(texts.validate_pack(self.catalog, replaced)['runtime_supported'])

    def test_menu_translations_templates_and_old_packs(self):
        pack=copy.deepcopy(self.pack)
        pack['menu']={'quit_to_menu':'В меню','save_failed':'Ошибка: {error}'}
        result=texts.validate_pack(self.catalog,pack)
        self.assertTrue(result['runtime_supported'])
        self.assertEqual(result['menu_translated'],2)
        for menu in ({'typo':'Text'},{'save_failed':'Ошибка'},{'back':'Ψ'}):
            pack['menu']=menu
            with self.assertRaises(texts.TextError):texts.validate_pack(self.catalog,pack)
        self.assertEqual(texts.validate_pack(self.catalog,self.pack)['menu_translated'],0)
    def setUp(self):
        self.catalog = catalog()
        self.pack = texts.new_pack(self.catalog, 'ru', 'Русский', font_source=texts.legacy_font())

    def test_color_font_palette_geometry_and_spacing_are_validated(self):
        self.pack['color_font']=dict(width=2,height=2,advance=6,palette=list(range(16)),glyphs={c:['12','89'] for c in self.pack['alphabet']})
        texts.upgrade_pack(self.pack)
        copy_pack=texts.new_pack(self.catalog,'ru','Russian',font_source=self.pack)
        self.assertEqual(copy_pack['color_font'],self.pack['color_font'])
        for field,value in [('width',17),('advance',7),('palette',[0,1]),('glyphs',{})]:
            bad=copy.deepcopy(self.pack);bad['color_font'][field]=value
            with self.assertRaises(texts.TextError):texts.upgrade_pack(bad)
        bad=copy.deepcopy(self.pack);bad['color_font']['glyphs']['А']=['zz','12']
        with self.assertRaises(texts.TextError):texts.upgrade_pack(bad)

    def test_ui_entries_preserve_letter_substitutions_and_line_breaks(self):
        self.catalog['scope']='confirmed-story-and-ui-tables'
        self.catalog['entries'].append(dict(id='ui.022c',texts={lang:'USE %A TO DESTROY %9 VEHICLES.\nNEXT' for lang in texts.LANGUAGES}))
        self.pack['entries']['ui.022c']='ИСПОЛЬЗУЙ %A ДЛЯ УНИЧТОЖЕНИЯ %9 МАШИН.\nДАЛЕЕ'
        self.assertTrue(texts.validate_pack(self.catalog,self.pack)['runtime_supported'])
        self.pack['entries']['ui.022c']='ИСПОЛЬЗУЙ %9 МАШИН.'
        with self.assertRaises(texts.TextError):
            texts.validate_pack(self.catalog,self.pack)

    def test_partial_unicode_pack_and_fallback(self):
        self.pack['entries']['text.00cabf84'] = 'МИССИЯ ВЫПОЛНЕНА'
        result = texts.validate_pack(self.catalog, self.pack)
        self.assertEqual(result['translated'], 1)
        self.assertEqual(result['fallback_entries'], 1)
        self.assertTrue(result['requires_extended_font'])
        self.assertTrue(result['runtime_supported'])

    def test_runtime_limits_and_control_tokens(self):
        self.pack['entries']['text.00cabf84'] = 'A'*610
        self.assertFalse(texts.validate_pack(self.catalog, self.pack)['runtime_supported'])
        self.pack['entries']['text.00cabf84'] = '😀'
        self.assertFalse(texts.validate_pack(self.catalog, self.pack)['runtime_supported'])
        self.pack['entries']['text.00cabf84'] = '@9COLOUR@0'
        self.assertTrue(texts.validate_pack(self.catalog, self.pack)['valid'])
        self.pack['entries']['text.00cabf84'] = '%12'
        with self.assertRaises(texts.TextError): texts.validate_pack(self.catalog, self.pack)

    def test_existing_language_copy(self):
        pack = texts.new_pack(self.catalog, 'fr', 'Français', 'fr')
        self.assertEqual(texts.validate_pack(self.catalog, pack)['fallback_entries'], 0)

    def test_missing_entry_is_fallback(self):
        del self.pack['entries']['text.00cabfac']
        self.assertEqual(texts.validate_pack(self.catalog, self.pack)['fallback_entries'], 2)

    def test_unknown_entry_and_version_rejected(self):
        self.pack['entries']['text.ffffffff'] = 'BAD'
        with self.assertRaises(texts.TextError): texts.validate_pack(self.catalog, self.pack)
        del self.pack['entries']['text.ffffffff']
        self.pack['version'] = 99
        with self.assertRaises(texts.TextError): texts.validate_pack(self.catalog, self.pack)

    def test_generic_font_does_not_assume_cyrillic(self):
        source = dict(alphabet='Ω',font=dict(width=3,height=3,glyphs={'Ω':['###','#.#','#.#']},aliases={'ω':'Ω'}))
        pack=texts.new_pack(self.catalog,'el','Greek',font_source=source)
        pack['entries']['text.00cabf84']='Ω ω'
        result=texts.validate_pack(self.catalog,pack)
        self.assertTrue(result['runtime_supported'])
        self.assertEqual(result['custom_glyphs'],1)
        pack['entries']['text.00cabf84']='Ψ'
        self.assertFalse(texts.validate_pack(self.catalog,pack)['runtime_supported'])

    def test_font_schema_aliases_and_integer_version(self):
        for version in [True,2.0,'2']:
            pack=copy.deepcopy(self.pack);pack['version']=version
            with self.assertRaises(texts.TextError):texts.validate_pack(self.catalog,pack)
        for mutate in [lambda p:p['font'].update(width=9),
                       lambda p:p['font']['glyphs'].pop(p['alphabet'][0]),
                       lambda p:p['font']['glyphs'][p['alphabet'][0]].__setitem__(0,'bad'),
                       lambda p:p.update(alphabet=p['alphabet']+p['alphabet'][0]),
                       lambda p:p['font']['aliases'].update({'x':'missing'})]:
            pack=copy.deepcopy(self.pack);mutate(pack)
            with self.assertRaises(texts.TextError):texts.validate_pack(self.catalog,pack)

    def test_legacy_migration_and_minimal_v2(self):
        old=dict(format=texts.LEGACY_PACK_FORMAT,base_rom_sha256=texts.USA_SHA,scope='confirmed-five-language-tables',language='ru',name='Русский',entries={'text.00cabf84':'МИССИЯ'})
        new=texts.upgrade_pack(old)
        self.assertEqual(new['version'],2)
        self.assertNotIn('scope',new)
        self.assertNotIn('base_rom_sha256',new)
        self.assertEqual(new['entries'],old['entries'])
        self.assertTrue(texts.validate_pack(self.catalog,new)['runtime_supported'])
        minimal=dict(version=2,language='en',name='English',entries={})
        self.assertTrue(texts.validate_pack(self.catalog,minimal)['runtime_supported'])

    def test_nul_control_surrogate_and_nonstring_rejected(self):
        for bad in ['A\0B', '\t', '\x81', '\ud800', 123]:
            with self.subTest(bad=repr(bad)):
                self.pack['entries']['text.00cabf84'] = bad
                with self.assertRaises(texts.TextError): texts.validate_pack(self.catalog, self.pack)

    def test_duplicate_ids_rejected(self):
        self.catalog['entries'].append(copy.deepcopy(self.catalog['entries'][0]))
        with self.assertRaises(texts.TextError): texts.check_catalog(self.catalog)

    def test_unicode_roundtrip_and_no_overwrite(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'pack.json'
            texts.write_json(path, self.pack)
            self.assertEqual(texts.load_json(path), self.pack)
            with self.assertRaises(texts.TextError): texts.write_json(path, {})
            path.write_text('{"entries":{},"entries":{}}')
            with self.assertRaises(texts.TextError): texts.load_json(path)


if __name__ == '__main__':
    unittest.main()
