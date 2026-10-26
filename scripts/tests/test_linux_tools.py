"""Regression checks for profile isolation, failure rollback and release contents."""
import argparse
import csv
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('linux_project', Path(__file__).parents[1] / 'linux/project.py')
p = importlib.util.module_from_spec(spec)
spec.loader.exec_module(p)


class LinuxTools(unittest.TestCase):
    def setUp(self):
        fixtures = p.ROOT / 'artifacts/scripts-tests'
        fixtures.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=fixtures)
        Path(self.temp.name).resolve().relative_to(fixtures.resolve())
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.scope = patch.object(p, 'ROOT', self.root)
        self.scope.start()
        self.addCleanup(self.scope.stop)
        self.rom = self.put('roms/usa.gba', b'\x01\x02\x03\x04')
        self.row = dict(zip(p.HEADER, ['dispatch', '08000001', '1', '0', '1', '0102' + '00' * 30]))
        self.coverage = self.root / 'native/usa/coverage/sessions.csv'
        self.coverage.parent.mkdir(parents=True)
        self.audit(self.coverage, [self.row])
        self.manifest = dict(format=1, rom_sha256=p.digest(self.rom), libraries=[dict(name='sessions', coverage='native/usa/coverage/sessions.csv')])
        p.write_json(self.root / 'native/usa/manifest.json', self.manifest)

    def put(self, name, data):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        return path

    def audit(self, path, rows):
        with path.open('w', newline='') as stream:
            writer = csv.DictWriter(stream, fieldnames=p.HEADER)
            writer.writeheader()
            writer.writerows(rows)

    def install_fixture(self):
        output = p.native_dir()
        output.mkdir(parents=True)
        lib = self.put(str(output.relative_to(self.root) / 'sessions.so'), b'ELF fixture')
        p.write_json(output / 'manifest.json', dict(format=1, rom_sha256=self.manifest['rom_sha256'], libraries=[dict(name='sessions', path=lib.relative_to(self.root).as_posix(), coverage_sha256=p.digest(self.coverage), sha256=p.digest(lib), variants=1)]))
        return lib

    def test_play_records_diagnostic_sessions_only_with_opt_in(self):
        args = argparse.Namespace(original_audio=False, drop_lifetime=30,
                                  original_renderer=False, frames=2,
                                  record_session=False, no_record=False)
        with patch.object(p, 'prepare', return_value=(self.rom, [])), patch.object(p, 'run') as runtime:
            p.play(args)
            command = runtime.call_args.args[0]
            self.assertNotIn('--record-session', command)
            self.assertIn('--replays', command)
            self.assertIn(self.root / 'replays', command)
            self.assertFalse((self.root / 'data/sessions').exists())
            args.record_session = True
            p.play(args)
            command = runtime.call_args.args[0]
            session = command[command.index('--record-session') + 1]
            self.assertEqual(session.parent, self.root / 'data/sessions')
            self.assertTrue(session.parent.is_dir())

    def test_linux_profiles_are_separate_and_detect_stale_coverage(self):
        windows = (self.root / 'native/usa/manifest.json').read_bytes()
        lib = self.install_fixture()
        self.assertEqual(p.native_libraries(), [lib])
        self.coverage.write_text(self.coverage.read_text() + '\n')
        with self.assertRaisesRegex(ValueError, 'coverage changed'):
            p.native_libraries()
        self.assertEqual(windows, (self.root / 'native/usa/manifest.json').read_bytes())

    def test_profile_hash_and_path_validation(self):
        lib = self.install_fixture()
        lib.write_bytes(b'corrupt')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            p.native_libraries()
        for bad in ['../outside', '/absolute', 'native/../../outside', 'native\\usa\\evil.so']:
            with self.assertRaises(ValueError):
                p.local_path(bad)

    def test_failed_compilation_keeps_installed_libraries(self):
        lib = self.install_fixture()
        before = (p.native_dir() / 'manifest.json').read_bytes()
        self.put('target/release/advance-aot-supplement', b'fixture')
        with patch.object(p.shutil, 'which', return_value='/usr/bin/cc'), patch.object(p, 'run', side_effect=ValueError('compiler failed')):
            with self.assertRaisesRegex(ValueError, 'compiler failed'):
                p.build_native(self.rom, force=True)
        self.assertEqual(lib.read_bytes(), b'ELF fixture')
        self.assertEqual((p.native_dir() / 'manifest.json').read_bytes(), before)

    def test_failed_install_restores_previous_directory(self):
        lib = self.install_fixture()
        self.put('target/release/advance-aot-supplement', b'fixture')
        def compile_fake(args, **kwargs):
            Path(args[3]).write_bytes(b'new library')
            return argparse.Namespace(stdout='1 guarded instruction variants; 0 unsupported skipped\n', stderr='')
        original = Path.rename
        def fail_new(path, target):
            if path.name == 'new':
                raise OSError('installation failed')
            return original(path, target)
        with patch.object(p.shutil, 'which', return_value='/usr/bin/cc'), patch.object(p, 'run', side_effect=compile_fake), patch.object(Path, 'rename', fail_new):
            with self.assertRaisesRegex(OSError, 'installation failed'):
                p.build_native(self.rom, force=True)
        self.assertEqual(lib.read_bytes(), b'ELF fixture')

    def test_collect_deduplicates_and_rejects_whole_bad_session(self):
        session = self.root / 'data/sessions/good'
        session.mkdir(parents=True)
        (session / 'summary.txt').write_text('frames=2\ninterpreted_steps=3\n')
        row = dict(self.row, key='03000001', bytes32='aabb' + '00' * 30)
        self.audit(session / 'fallbacks.csv', [row, row, self.row])
        args = argparse.Namespace(rom=None, session=[str(session)], dry_run=True)
        before = self.coverage.read_bytes()
        p.collect(args)
        self.assertEqual(self.coverage.read_bytes(), before)
        args.dry_run = False
        p.collect(args)
        self.assertEqual(len(p.read_audit(self.coverage)), 2)
        merged = self.coverage.read_bytes()
        p.collect(args)
        self.assertEqual(merged, self.coverage.read_bytes())
        self.audit(session / 'fallbacks.csv', [dict(row, key='03000003'), dict(self.row, bytes32='ffff' + '00' * 30)])
        with self.assertRaisesRegex(ValueError, 'Rejected'):
            p.collect(args)
        self.assertEqual(merged, self.coverage.read_bytes())

    def test_package_whitelist_preserves_executable_and_has_no_rom(self):
        self.install_fixture()
        self.put('target/release/AdvanceLiberty', b'ELF runtime')
        self.put('Cargo.toml', b'version = "test"\n')
        for name in ['release/README.linux.txt', 'release/README.linux.ru.txt', 'LICENSE', 'NOTICE', 'release/THIRD_PARTY_NOTICES.txt', 'vendor/LICENSE-MIT', 'vendor/LICENSE-APACHE']:
            self.put(name, b'license or instructions')
        def metadata(args, **kwargs):
            return argparse.Namespace(stdout='host: x86_64-unknown-linux-gnu\n' if args[0] == 'rustc' else '{"packages": []}')
        args = argparse.Namespace(skip_build=True, output=str(self.root / 'dist/release'))
        with patch.object(p, 'run', side_effect=metadata):
            p.package(args)
        folder = self.root / 'dist/release'
        if os.name != 'nt':
            self.assertTrue((folder / 'AdvanceLiberty').stat().st_mode & 0o111)
        self.assertEqual(list(folder.rglob('*.gba')), [])
        self.assertTrue((folder / 'native/usa/sessions.so').is_file())
        self.assertTrue((folder / 'licenses/dependencies.json').is_file())
        self.assertTrue((self.root / 'dist/release.tar.gz.sha256').is_file())
        with self.assertRaisesRegex(ValueError, 'already exists'):
            with patch.object(p, 'run', side_effect=metadata):
                p.package(args)


if __name__ == '__main__':
    unittest.main()
