"""Release automation checks; GitHub operations are mocked."""
import hashlib
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('ci_release', Path(__file__).parents[1] / 'ci/release.py')
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)


class Releases(unittest.TestCase):
    def setUp(self):
        parent = r.ROOT / 'artifacts/release-tests'
        parent.mkdir(parents=True, exist_ok=True)
        self.temporary = tempfile.TemporaryDirectory(dir=parent)
        Path(self.temporary.name).resolve().relative_to(parent.resolve())
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        scope = patch.object(r, 'ROOT', self.root)
        scope.start()
        self.addCleanup(scope.stop)
        (self.root / 'Cargo.toml').write_text('version = "0.1.0-preview.2"\n')
        self.directory = self.root / 'assets'
        self.directory.mkdir()
        for suffix in ['-win64.zip', '-linux-x86_64.tar.gz']:
            name = 'AdvanceLiberty-Enhanced-0.1.0-preview.2' + suffix
            archive = self.directory / name
            archive.write_bytes(name.encode())
            (self.directory / (name + '.sha256')).write_text(hashlib.sha256(archive.read_bytes()).hexdigest() + '  ' + name + '\n')
        self.tag = 'v0.1.0-preview.2'
        self.repository = 'owner/project'
        self.commit = 'a' * 40

    def test_version_tag_is_exact(self):
        self.assertEqual(r.check_tag(self.tag), self.tag)
        for tag in ['v0.1.0', 'v0.1.0-preview.3', 'main']:
            with self.assertRaises(ValueError):
                r.check_tag(tag)

    def test_lockfile_must_follow_package_version_bumps(self):
        lock = self.root / 'Cargo.lock'
        lock.write_text('version = 4\n\n[[package]]\nname = "advance-liberty"\nversion = "0.1.0-preview.2"\n')
        r.check_lock_version()
        lock.write_text(lock.read_text().replace('0.1.0-preview.2', '0.1.0-preview.1'))
        with self.assertRaisesRegex(ValueError, r'Cargo.lock.*Cargo.toml.*cargo update'):
            r.check_lock_version()
        lock.write_text('version = 4\n')
        with self.assertRaisesRegex(ValueError, 'missing package'):
            r.check_lock_version()

    def test_requires_both_platforms_and_matching_checksums(self):
        self.assertEqual(len(r.assets(self.directory)), 4)
        (self.directory / 'AdvanceLiberty-Enhanced-0.1.0-preview.2-win64.zip').write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'checksum'):
            r.assets(self.directory)
        (self.directory / 'AdvanceLiberty-Enhanced-0.1.0-preview.2-win64.zip').unlink()
        with self.assertRaisesRegex(ValueError, 'Missing'):
            r.assets(self.directory)

    def test_dry_run_never_contacts_github(self):
        with patch.object(r, 'gh') as gh, patch.object(r.subprocess, 'run') as view:
            r.publish(self.tag, self.directory, self.repository, self.commit, dry_run=True)
            gh.assert_not_called()
            view.assert_not_called()

    def test_preview_is_uploaded_as_draft_then_published(self):
        with patch.object(r, 'gh') as gh, patch.object(r.subprocess, 'run', return_value=subprocess.CompletedProcess([], 1)):
            r.publish(self.tag, self.directory, self.repository, self.commit)
        create, edit = [call.args[0] for call in gh.call_args_list]
        self.assertEqual(create[:3], ['release', 'create', self.tag])
        self.assertIn('--verify-tag', create)
        self.assertIn('--draft', create)
        self.assertIn('--prerelease', create)
        self.assertTrue(all(f in create for f in r.assets(self.directory)))
        self.assertEqual(edit[:3], ['release', 'edit', self.tag])
        self.assertIn('--draft=false', edit)
        self.assertIn('--latest=false', edit)

    def test_failed_upload_never_publishes(self):
        view = dict(isDraft=True, body=f'Source commit: https://github.com/{self.repository}/commit/{self.commit}', assets=[])
        with patch.object(r.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, stdout=json.dumps(view))), patch.object(r, 'gh', side_effect=subprocess.CalledProcessError(1, 'gh')) as gh:
            with self.assertRaises(subprocess.CalledProcessError):
                r.publish(self.tag, self.directory, self.repository, self.commit)
            self.assertEqual(gh.call_count, 1)
            self.assertEqual(gh.call_args.args[0][:2], ['release', 'upload'])

    def test_published_release_is_not_overwritten_or_retargeted(self):
        view = dict(isDraft=False, body=f'Source commit: https://github.com/{self.repository}/commit/{self.commit}', assets=[dict(name=p.name) for p in r.assets(self.directory)])
        with patch.object(r.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, stdout=json.dumps(view))), patch.object(r, 'gh') as gh:
            r.publish(self.tag, self.directory, self.repository, self.commit)
            gh.assert_not_called()
        view['body'] = view['body'].replace(self.commit, 'b' * 40)
        with patch.object(r.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, stdout=json.dumps(view))):
            with self.assertRaisesRegex(ValueError, 'another commit'):
                r.publish(self.tag, self.directory, self.repository, self.commit)


if __name__ == '__main__':
    unittest.main()
