#!/usr/bin/env python3
"""Validate versioned release assets and publish them with GitHub CLI."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def version():
    value = re.search(r'^version = "([^"]+)"', (ROOT / 'Cargo.toml').read_text(), re.M)[1]
    if not re.fullmatch(r'\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?', value):
        raise ValueError('Unsupported release version in Cargo.toml.')
    return value


def check_tag(tag):
    expected = 'v' + version()
    if tag != expected:
        raise ValueError(f'Release tag must match Cargo.toml: expected {expected}, got {tag}')
    return tag


def check_lock_version():
    expected = version()
    # Cargo-generated lockfiles put name/version first in each package block.
    # Avoid tomllib: Ubuntu 22.04's system Python is 3.10.
    locked = re.findall(r'^\[\[package\]\]\s+name = "advance-liberty"\s+version = "([^"]+)"',
                        (ROOT / 'Cargo.lock').read_text(), re.M)
    if locked != [expected]:
        actual = ', '.join(locked) or 'missing package'
        raise ValueError(f'Cargo.lock has advance-liberty {actual}; Cargo.toml has {expected}. '
                         'Run cargo update --offline --package advance-liberty and commit Cargo.lock '
                         'together with Cargo.toml before building or tagging a release.')


def assets(directory):
    prefix = 'AdvanceLiberty-Enhanced-' + version()
    archives = [directory / (prefix + '-win64.zip'), directory / (prefix + '-linux-x86_64.tar.gz')]
    files = []
    for archive in archives:
        checksum = archive.with_name(archive.name + '.sha256')
        if not archive.is_file() or not checksum.is_file():
            raise ValueError(f'Missing release archive/checksum: {archive.name}')
        expected = checksum.read_text(encoding='utf-8-sig').strip().split()
        actual = hashlib.sha256(archive.read_bytes()).hexdigest()
        if len(expected) != 2 or expected[0].lower() != actual or expected[1] != archive.name:
            raise ValueError(f'Archive checksum mismatch: {archive.name}')
        files.extend([archive, checksum])
    return files


def gh(args, **kwargs):
    return subprocess.run(['gh', *map(str, args)], check=True, text=True, **kwargs)


def publish(tag, directory, repository, commit, dry_run=False):
    check_tag(tag)
    files = assets(directory)
    if not re.fullmatch(r'[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+', repository):
        raise ValueError('Invalid GitHub repository.')
    if not re.fullmatch(r'[0-9a-fA-F]{40}', commit):
        raise ValueError('Full release commit SHA required.')
    prerelease = '-' in version()
    print(f'Release: {repository} / {tag}; prerelease={prerelease}; commit={commit}', flush=True)
    print('\n'.join(str(p) for p in files), flush=True)
    if dry_run:
        return
    # Drafts can resume after interrupted uploads. Published releases are immutable
    # to this tool: a rerun must not replace assets that users already downloaded.
    existing = subprocess.run(['gh', 'release', 'view', tag, '--repo', repository,
                               '--json', 'isDraft,body,assets'], capture_output=True, text=True)
    if existing.returncode == 0:
        release = json.loads(existing.stdout)
        # GitHub's targetCommitish may be a branch name for an existing tag.
        # Our notes retain the exact source commit independently of that field.
        source = f'Source commit: https://github.com/{repository}/commit/{commit}'
        if source not in release.get('body', '').splitlines():
            raise ValueError('Existing release targets another commit; use a new version/tag.')
        if not release['isDraft']:
            names = {a['name'] for a in release['assets']}
            if not all(f.name in names for f in files):
                raise ValueError('Published release is incomplete. Existing public assets will not be replaced.')
            print('Release already published; keeping its existing assets.')
            return
        gh(['release', 'upload', tag, *files, '--repo', repository, '--clobber'])
    else:
        notes = ('Provide your own legally obtained, unmodified USA cartridge dump. '
                 'No ROM, BIOS or extracted original game assets are included or downloaded.\n\n'
                 'Windows x64: extract the ZIP, place your .gba in roms/, run AdvanceLiberty.exe.\n'
                 'Linux x86_64: extract the tar.gz, place your .gba in roms/, run ./AdvanceLiberty.\n\n'
                 f'Source commit: https://github.com/{repository}/commit/{commit}\n')
        # Keep the notes local to the ephemeral runner; the release is published
        # only after gh has successfully attached all four verified assets.
        temporary_root = ROOT / 'artifacts'
        temporary_root.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(prefix='release-', dir=temporary_root) as temporary:
            path = Path(temporary) / 'notes.md'
            path.write_text(notes, encoding='utf-8')
            options = ['--prerelease'] if prerelease else []
            gh(['release', 'create', tag, *files, '--repo', repository, '--verify-tag', '--draft',
                '--target', commit, '--title', 'AdvanceLiberty: Enhanced ' + tag, '--notes-file', path, *options])
    options = ['--latest=false'] if prerelease else ['--latest']
    gh(['release', 'edit', tag, '--repo', repository, '--draft=false',
        '--prerelease=' + str(prerelease).lower(), *options])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['check-version', 'verify-assets', 'publish'])
    parser.add_argument('--tag')
    parser.add_argument('--directory', type=Path, default=Path('release-assets'))
    parser.add_argument('--repository', default=os.environ.get('GITHUB_REPOSITORY', ''))
    parser.add_argument('--commit', default=os.environ.get('GITHUB_SHA', ''))
    parser.add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    tag = args.tag or os.environ.get('GITHUB_REF_NAME', '')
    if args.command == 'check-version':
        check_lock_version()
        if os.environ.get('GITHUB_REF_TYPE') == 'tag' or args.tag is not None:
            check_tag(tag)
        print('Package version: ' + version())
    elif args.command == 'verify-assets':
        assets(args.directory)
        print('Both platform archives and SHA-256 files verified.')
    else:
        publish(tag, args.directory, args.repository, args.commit, args.dry_run)


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f'Error: {error}', file=sys.stderr)
        sys.exit(1)
