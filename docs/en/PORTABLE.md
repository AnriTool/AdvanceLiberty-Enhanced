# Portable AdvanceLiberty: Enhanced build

**English** | [Русский](../ru/PORTABLE.md)

## User folder

`AdvanceLiberty.exe` is the only entry point. The distribution contains no
BAT/PS1 scripts, compiler, ROM, BIOS, saves, gameplay recordings or extracted
language packs. SDL and the C runtime are statically linked; neither an external
Visual C++ Redistributable nor SDL2.dll is required. Platform: Windows x64.

Extract the whole folder to a writable location, place your own legally obtained
USA `.gba` dump in `roms/`, then start the EXE. Any filename is accepted.
ROMs in the folder root, drag and drop onto the EXE, and `--rom FILE` are also
supported. With several files, the first exact SHA-256 match is selected.
European and modified ROMs are rejected.

```text
AdvanceLiberty.exe
README.txt
README.ru.txt
LICENSE
NOTICE
release.json
native/usa/manifest.json
native/usa/{precise,extra,mission,sessions}.dll
roms/PUT_YOUR_ROM_HERE.txt
languages/README.txt
licenses/
```

At startup, the EXE validates the ROM against the supported profile and checks
each DLL's SHA-256. It does not download a ROM or missing libraries. Errors show
an explanatory dialog. Native code is not executed for an incompatible ROM.

## Paths, preferences and diagnostics

Paths are relative to the EXE, not the current working directory.
`data/saves/usa.sav` stores game progress, with `.bak` as its backup.
`data/config/` stores preferences. There is no need to copy controls.cfg from
a development checkout: the first launch creates a standard Enhanced profile.

Defaults are Enhanced rendering, Enhanced Audio, a 30-second drop lifetime,
16:9, fullscreen, aiming while holding RMB, and session recording. Explicitly
saved choices are not overwritten. `--original-audio`, `--drop-lifetime 0` and
`--no-record` disable the corresponding features. `--help` displays launch
options. Language packs live in `languages/`; the English template is extracted
from the user's ROM on first launch.

The launcher starts the same EXE in game-runtime mode without a shell. Runtime
output goes to `data/logs/launch-*.log`, and recordings to
`data/sessions/session-*`. A child-process failure shows the end of its log
and the log path. Closing the game also ends the waiting launcher. The low-level
CLI remains available for replays and development tools.

## Packaging a release

`./scripts/windows/package.ps1 -Offline` builds the release EXE and creates a new folder and ZIP
at `dist/AdvanceLiberty-Enhanced-VERSION-win64`. `-SkipBuild` uses an existing
EXE; `-OutputDirectory PATH` selects a new location. Existing output folders
are never cleared or overwritten, protecting any ROMs/saves added later.

Packaging copies only the EXE, four validated DLLs, a reduced manifest,
instructions and license notices. Native profiles must already have been built
with CollectNative/UpdateNative/RebuildNative; those tools remain in the project.
Generated C, coverage, research history and user data are excluded. Do not
redistribute a manually zipped folder after testing it with personal data;
run `scripts/windows/package.ps1` again with a new output directory.

`release.json` records the version and file hashes; a `.sha256` file accompanies
the ZIP. Third-party notices retain gba-recomp authorship and dependency
licenses. Third-party project names are not changed by the application's rebranding.

[Linux builds and tar.gz packaging](LINUX.md) use separate platform binaries and shell entry points.
