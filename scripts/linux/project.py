#!/usr/bin/env python3
"""Linux build, native-profile and release tools; Python standard library only."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import uuid

ROOT = Path(__file__).resolve().parents[2]
HEADER = ['reason', 'key', 'steps', 'first_frame', 'last_frame', 'bytes32']


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.sha256(stream.read()).hexdigest()


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + '\n', encoding='utf-8')


def run(args, **kwargs):
    return subprocess.run([str(a) for a in args], cwd=ROOT, check=True, **kwargs)


def source_manifest():
    return json.loads((ROOT / 'native/usa/manifest.json').read_text(encoding='utf-8-sig'))


def local_path(relative):
    p = Path(relative)
    if p.is_absolute() or '..' in p.parts or '\\' in str(relative):
        raise ValueError(f'Unsafe project path: {relative}')
    result = (ROOT / p).resolve()
    result.relative_to(ROOT.resolve())
    return result


def architecture():
    name = platform.machine().lower()
    if name in ('amd64', 'x86_64'):
        return 'x86_64'
    raise ValueError('The Linux build currently targets x86_64. Other architectures are untested.')


def native_dir():
    return ROOT / 'native' / ('linux-' + architecture())


def rom_path(value=None):
    path = Path(value).resolve() if value else ROOT / 'roms/usa.gba'
    if not path.is_file():
        raise ValueError(f'Provide your own unmodified USA ROM: {path} (or use --rom FILE).')
    if digest(path) != source_manifest()['rom_sha256']:
        raise ValueError('Unsupported ROM: the exact unmodified USA dump is required.')
    return path


def executable():
    return ROOT / 'target/release/AdvanceLiberty'


def native_libraries(check_coverage=True):
    data = json.loads((native_dir() / 'manifest.json').read_text())
    base = source_manifest()
    if data.get('format') != 1 or data.get('rom_sha256') != base['rom_sha256']:
        raise ValueError('Incompatible Linux native manifest.')
    if [e['name'] for e in data['libraries']] != [e['name'] for e in base['libraries']]:
        raise ValueError('Linux native profile order differs from source coverage.')
    result = []
    for entry, original in zip(data['libraries'], base['libraries']):
        path = local_path(entry['path'])
        path.relative_to(native_dir().resolve())
        if path.suffix != '.so' or digest(path) != entry['sha256']:
            raise ValueError(f'Linux native library checksum mismatch: {path}')
        if check_coverage and entry.get('coverage_sha256') != digest(local_path(original['coverage'])):
            raise ValueError('Native coverage changed. Run ./rebuild-native.sh.')
        result.append(path)
    return result


def build_native(rom, force=False, cc=None, coverage_only=False):
    if not force:
        try:
            return native_libraries()
        except (OSError, ValueError, KeyError):
            pass
    compiler = shutil.which(cc or os.environ.get('CC', 'cc'))
    if not compiler:
        raise ValueError('Host C compiler missing. Install GCC or pass --cc /path/to/gcc.')
    generator = ROOT / 'target/release/advance-aot-supplement'
    if not generator.is_file():
        raise ValueError('AOT tool missing. Run ./build.sh first.')
    work = ROOT / 'artifacts' / ('native-linux-' + uuid.uuid4().hex)
    stage = work / 'new'
    stage.mkdir(parents=True)
    manifest = source_manifest()
    entries = []
    for entry in manifest['libraries']:
        name = entry['name']
        if not re.fullmatch(r'[a-z][a-z0-9_-]*', name):
            raise ValueError('Invalid native profile name.')
        output = stage / (name + '.so')
        options = ['--ram-variants'] + (['--coverage-only'] if coverage_only else [])
        completed = run([generator, local_path(entry['coverage']), '-' if coverage_only else rom, output, compiler, *options], capture_output=True, text=True)
        print(completed.stdout, end='', flush=True)
        if completed.stderr:
            print(completed.stderr, file=sys.stderr, end='')
        counts = re.search(r'(\d+) guarded instruction variants; (\d+) unsupported skipped', completed.stdout)
        if not counts or int(counts[2]):
            raise ValueError(f'Incomplete native build: {name}. Previous profiles unchanged.')
        entries.append(dict(name=name, path=(native_dir() / output.name).relative_to(ROOT).as_posix(),
                            coverage=entry['coverage'], coverage_sha256=digest(local_path(entry['coverage'])),
                            sha256=digest(output), variants=int(counts[1])))
    write_json(stage / 'manifest.json', dict(format=1, rom_sha256=manifest['rom_sha256'], libraries=entries))
    destination = native_dir()
    backup = work / 'before-install'
    if destination.exists():
        destination.rename(backup)
    try:
        stage.rename(destination)
    except OSError:
        if backup.exists():
            backup.rename(destination)
        raise
    print(f'Installed Linux native profiles. Build/backup: {work}')
    return native_libraries()


def build(args):
    flags = ['--offline'] if args.offline else []
    run(['cargo', 'build', '--release', '--locked', '--package', 'advance-liberty', '--bin', 'AdvanceLiberty', *flags])
    run(['cargo', 'build', '--release', '--locked', '--package', 'advance-aot-supplement', *flags])
    if args.native:
        build_native(None if args.coverage_only else rom_path(args.rom), args.force, args.cc, args.coverage_only)


def prepare(args):
    rom = rom_path(args.rom)
    if not args.skip_build:
        build(argparse.Namespace(offline=args.offline, native=False))
        libraries = build_native(rom)
    else:
        libraries = native_libraries()
    if not executable().is_file():
        raise ValueError('Runtime missing. Run ./build.sh.')
    return rom, libraries


def play(args):
    rom, libraries = prepare(args)
    for directory in ['data/saves', 'data/config']:
        (ROOT / directory).mkdir(parents=True, exist_ok=True)
    options = ['--save', ROOT / 'data/saves/usa.sav', '--controls', ROOT / 'data/config/controls.cfg', '--camera-preview', '--replays', ROOT / 'replays']
    if not args.original_audio:
        options += ['--enhanced-audio']
        if args.polyphonic_audio:
            options += ['--polyphonic-audio']
    if args.drop_lifetime:
        options += ['--drop-lifetime', str(args.drop_lifetime)]
    if args.record_session and not args.no_record:
        (ROOT / 'data/sessions').mkdir(parents=True, exist_ok=True)
        session = ROOT / 'data/sessions' / ('session-' + uuid.uuid4().hex)
        options += ['--record-session', session]
        print(f'Session: {session}', flush=True)
    if args.original_renderer:
        options += ['--original-renderer']
    run([executable(), rom, '-', str(args.frames), '-', 'play', *libraries, *options])


def session_path(value):
    path = Path(value)
    if not path.is_dir():
        path = ROOT / 'data/sessions' / value
    return path.resolve(strict=True)


def summary(path):
    return dict(line.split('=', 1) for line in (path / 'summary.txt').read_text().splitlines() if '=' in line)


def replay(args):
    rom, libraries = prepare(args)
    session = session_path(args.session)
    frames = int(summary(session)['frames'])
    if frames <= 0:
        raise ValueError('Completed nonempty session required.')
    output = Path(args.output).resolve() if args.output else ROOT / 'artifacts/replays' / uuid.uuid4().hex
    output.mkdir(parents=True, exist_ok=False)
    save = output / 'replay.sav'
    shutil.copy2(session / 'initial.sav', save)
    options = ['--save', save, '--audio-hashes', '--audit', output / 'fallbacks.csv']
    if not args.allow_interpreter:
        options += ['--require-native']
    run([executable(), rom, session / 'input.gba-input', str(frames), output / 'frames.csv', 'multi', *libraries, *options])
    if digest(output / 'frames.csv') != digest(session / 'frames.csv'):
        raise ValueError(f'Recorded video/audio hashes differ. See {output}')
    print(f'MATCH: {frames} video/audio frames. {output}')


def read_audit(path):
    with Path(path).open(newline='', encoding='utf-8-sig') as stream:
        reader = csv.DictReader(stream)
        if reader.fieldnames != HEADER:
            raise ValueError(f'Invalid audit header: {path}')
        return list(reader)


def signature(row, rom):
    if not re.fullmatch(r'[0-9a-fA-F]{8}', row['key']) or not re.fullmatch(r'[0-9a-fA-F]{64}', row['bytes32']):
        raise ValueError('Invalid instruction address/bytes.')
    key = int(row['key'], 16)
    size = 2 if key & 1 else 4
    address = key & ~1
    if key >> 24 not in (2, 3, 8) or address % size:
        raise ValueError('Unsupported or misaligned instruction.')
    for field in HEADER[2:5]:
        if int(row[field]) < 0:
            raise ValueError('Negative audit count/frame.')
    code = row['bytes32'][:size * 2].lower()
    if key >> 24 == 8:
        offset = address & 0x1ffffff
        if rom[offset:offset + size].hex() != code:
            raise ValueError(f'ROM bytes differ at {key:08x}')
    return f'{key:08x}:{code}'


def collect(args):
    rom = rom_path(args.rom).read_bytes()
    manifest = source_manifest()
    known, merged = set(), {}
    target = local_path(next(e['coverage'] for e in manifest['libraries'] if e['name'] == 'sessions'))
    for entry in manifest['libraries']:
        for row in read_audit(local_path(entry['coverage'])):
            if int(row['key'], 16) >> 24 not in (2, 3, 8):
                continue
            sig = signature(row, rom)
            known.add(sig)
            if entry['name'] == 'sessions':
                merged[sig] = row
    before = len(merged)
    sessions = [session_path(s) for s in args.session] if args.session else sorted((ROOT / 'data/sessions').glob('*'))
    for session in sessions:
        try:
            stats = summary(session)
            if int(stats['frames']) <= 0:
                raise ValueError('Empty session.')
            int(stats['interpreted_steps'])
            pending = {}
            for row in read_audit(session / 'fallbacks.csv'):
                sig = signature(row, rom)
                if sig not in known:
                    pending[sig] = row
            merged.update(pending)
            known.update(pending)
        except (OSError, ValueError, KeyError) as error:
            if args.session:
                raise ValueError(f'Rejected {session}: {error}') from error
            print(f'Skipping {session.name}: {error}', file=sys.stderr)
    added = len(merged) - before
    if added and not args.dry_run:
        backup = ROOT / 'artifacts' / ('native-collect-' + uuid.uuid4().hex)
        backup.mkdir(parents=True)
        shutil.copy2(target, backup / 'before-sessions.csv')
        staged = target.with_suffix('.csv.' + uuid.uuid4().hex + '.tmp')
        try:
            with staged.open('w', newline='', encoding='utf-8') as stream:
                writer = csv.DictWriter(stream, fieldnames=HEADER, lineterminator='\n')
                writer.writeheader()
                writer.writerows(merged[key] for key in sorted(merged))
            os.replace(staged, target)
        finally:
            staged.unlink(missing_ok=True)
    print(f'Native coverage: +{added} variants; sessions={len(merged)}; dry_run={args.dry_run}')


def package(args):
    if not args.skip_build:
        prepare(args)
    libraries = native_libraries()
    version = re.search(r'^version = "([^"]+)"', (ROOT / 'Cargo.toml').read_text(), re.M)[1]
    destination = Path(args.output).resolve() if args.output else ROOT / 'dist' / f'AdvanceLiberty-Enhanced-{version}-linux-x86_64'
    archive = destination.with_name(destination.name + '.tar.gz')
    if destination.exists() or archive.exists():
        raise ValueError('Release output already exists; choose a new --output directory.')
    destination.mkdir(parents=True)
    for folder in ['native/usa', 'roms', 'languages', 'licenses']:
        (destination / folder).mkdir(parents=True, exist_ok=True)
    shutil.copy2(executable(), destination / 'AdvanceLiberty')
    (destination / 'AdvanceLiberty').chmod(0o755)
    data = json.loads((native_dir() / 'manifest.json').read_text())
    entries = []
    for entry, library in zip(data['libraries'], libraries):
        relative = 'native/usa/' + library.name
        shutil.copy2(library, destination / relative)
        entries.append(dict(name=entry['name'], path=relative, sha256=entry['sha256'], variants=entry['variants']))
    write_json(destination / 'native/usa/manifest.json', dict(format=1, rom_sha256=data['rom_sha256'], libraries=entries))
    for source, target in [('release/README.linux.txt', 'README.txt'), ('release/README.linux.ru.txt', 'README.ru.txt'),
                           ('LICENSE', 'LICENSE'), ('NOTICE', 'NOTICE'), ('release/THIRD_PARTY_NOTICES.txt', 'licenses/THIRD_PARTY_NOTICES.txt'),
                           ('vendor/LICENSE-MIT', 'licenses/gba-recomp-MIT.txt'), ('vendor/LICENSE-APACHE', 'licenses/gba-recomp-APACHE.txt')]:
        shutil.copy2(ROOT / source, destination / target)
    (destination / 'roms/PUT_YOUR_ROM_HERE.txt').write_text('Provide your own legally obtained unmodified USA .gba dump. No ROM is included or downloaded.\n')
    (destination / 'languages/README.txt').write_text('Language packs go here. Editable en.json is generated from your own ROM at first launch.\n')
    host = re.search(r'^host: (.+)$', run(['rustc', '-vV'], capture_output=True, text=True).stdout, re.M)[1]
    metadata = json.loads(run(['cargo', 'metadata', '--offline', '--locked', '--format-version', '1', '--filter-platform', host], capture_output=True, text=True).stdout)
    dependencies = []
    for dep in metadata['packages']:
        if not dep['source']:
            continue
        folder = destination / 'licenses' / (dep['name'] + '-' + dep['version'])
        folder.mkdir()
        source = Path(dep['manifest_path']).parent
        for item in source.iterdir():
            if item.is_file() and re.match(r'^(LICENSE|LICENCE|COPYING|COPYRIGHT|NOTICE)', item.name):
                shutil.copy2(item, folder / item.name)
        if dep['name'] == 'sdl2-sys':
            shutil.copy2(source / 'SDL/LICENSE.txt', folder / 'SDL-LICENSE.txt')
        dependencies.append(dict(name=dep['name'], version=dep['version'], license=dep['license']))
    write_json(destination / 'licenses/dependencies.json', dependencies)
    files = sorted(p for p in destination.rglob('*') if p.is_file())
    forbidden = {'.gba', '.sav', '.ps1', '.bat', '.cmd', '.sh', '.c', '.csv', '.pdb', '.dll'}
    if any(p.suffix.lower() in forbidden for p in files):
        raise ValueError('Unexpected user/development content in release.')
    write_json(destination / 'release.json', dict(product='AdvanceLiberty: Enhanced', version=version, platform='linux-x86_64',
               files=[dict(path=p.relative_to(destination).as_posix(), sha256=digest(p)) for p in files]))
    with tarfile.open(archive, 'w:gz') as stream:
        stream.add(destination, arcname=destination.name)
    archive.with_name(archive.name + '.sha256').write_text(f'{digest(archive)}  {archive.name}\n')
    print(f'Release folder: {destination}\nArchive: {archive}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    for name in ['build', 'play', 'rebuild-native', 'collect-native', 'replay', 'package']:
        command = commands.add_parser(name)
        command.add_argument('--rom', help='path to your unmodified USA ROM')
        if name in ['build', 'play', 'replay', 'package']:
            command.add_argument('--offline', action='store_true')
        if name in ['play', 'replay', 'package', 'rebuild-native']:
            command.add_argument('--skip-build', action='store_true')
        if name in ['build', 'rebuild-native']:
            command.add_argument('--coverage-only', action='store_true', help='generate native profiles from versioned instruction snapshots without a ROM')
            command.add_argument('--force', action='store_true')
            command.add_argument('--cc', help='host C compiler executable')
        if name in ['replay', 'package']:
            command.add_argument('--output')
    commands.choices['build'].add_argument('--native', action='store_true', help='also compile Linux native profiles')
    play_parser = commands.choices['play']
    play_parser.add_argument('--no-record', action='store_true')
    play_parser.add_argument('--record-session', action='store_true')
    play_parser.add_argument('--original-audio', action='store_true')
    play_parser.add_argument('--polyphonic-audio', action='store_true')
    play_parser.add_argument('--original-renderer', action='store_true')
    play_parser.add_argument('--frames', type=int, default=0)
    play_parser.add_argument('--drop-lifetime', type=int, choices=range(301), default=30, metavar='0..300')
    commands.choices['replay'].add_argument('session')
    commands.choices['replay'].add_argument('--allow-interpreter', action='store_true')
    commands.choices['collect-native'].add_argument('--session', action='append', default=[])
    commands.choices['collect-native'].add_argument('--dry-run', action='store_true')
    args = parser.parse_args()
    if args.command == 'rebuild-native':
        if not args.skip_build:
            build(argparse.Namespace(offline=False, native=False))
        build_native(None if args.coverage_only else rom_path(args.rom), force=True, cc=args.cc, coverage_only=args.coverage_only)
    else:
        {'build': build, 'play': play, 'replay': replay, 'package': package, 'collect-native': collect}[args.command](args)


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        print(f'Error: {error}', file=sys.stderr)
        sys.exit(1)
