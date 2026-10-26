# Linux builds and script layout

**English** | [Русский](../ru/LINUX.md)

The Linux target is x86_64 with glibc and an X11 or Wayland desktop. It uses
native ELF `.so` profiles generated from the same versioned instruction
coverage as Windows. Wine and a Windows DLL loader are not involved.
Linux game execution still needs your own exact, unmodified USA ROM.

## Source layout

| Path | Purpose |
| --- | --- |
| `Build.bat`, `Play.bat` | Windows build and development launch |
| `Test.bat`, `Package.bat`, `Replay.bat` | Windows checks, packaging and replay |
| `CollectNative.bat`, `RebuildNative.bat`, `UpdateNative.bat` | Windows coverage tools |
| `scripts/windows/` | PowerShell implementations, helpers and their tests |
| `build.sh`, `play.sh`, `test.sh`, `package.sh`, `replay.sh` | Linux entry points |
| `collect-native.sh`, `rebuild-native.sh` | Linux coverage tools |
| `scripts/linux/` | Linux implementation (Python standard library and Bash) |
| `scripts/tests/` | Tests for collection, profile isolation, rollback and packaging |
| `native/usa/coverage/` | Shared versioned coverage for both systems |
| `native/usa/*.dll`, `native/usa/manifest.json` | Windows generated binaries and manifest |
| `native/linux-x86_64/` | Local Linux `.so` files, hashes and coverage fingerprints |

Root BAT/SH files are developer conveniences. Packaged users launch the native
application directly. PowerShell requires no Python. Linux developer tools
require Python 3.9+, Bash, Rust/Cargo, a host C compiler, CMake and pkg-config.
Generated binaries, ROMs, user data and generated C files are not versioned.

## Build and run on Linux

On Ubuntu/Debian, install the build dependencies first:

```sh
sudo apt-get install build-essential cmake pkg-config python3 \
  libasound2-dev libpulse-dev libx11-dev libxext-dev libxrandr-dev \
  libxcursor-dev libxi-dev libxss-dev libxinerama-dev libgl1-mesa-dev \
  libudev-dev libdbus-1-dev libwayland-dev libxkbcommon-dev
```

Install a current stable Rust toolchain separately. SDL2 is built from the
locked bundled dependency and linked into the executable; sound, display and
OpenGL drivers remain system dependencies. The first build needs access to
the Cargo registry. `--offline` requires an already populated Linux Cargo cache.

From the project root:

```sh
bash build.sh                         # runtime and AOT tools; no ROM needed
# Put your own unmodified USA ROM in roms/usa.gba, then:
bash build.sh --native                # generate all four native .so profiles
bash play.sh --skip-build             # development saves/settings in data/
bash test.sh --offline
bash package.sh --skip-build          # new folder and .tar.gz in dist/
```

`bash play.sh` builds missing/outdated components automatically. Use `--rom FILE`
when your ROM has another path. `--skip-build` refuses stale coverage, corrupt
libraries or missing builds. `CC` or `--cc /path/to/gcc` chooses the host compiler
for native builds. Do not use a Windows cross compiler.

To invoke entry points as `./build.sh` etc., first run `chmod +x ./*.sh` if your
checkout has no execution bits. Invoking them with `bash` works either way.

New development profiles retain Enhanced graphics/audio/controls, widescreen,
fullscreen, mouse aiming and the 30-second drop lifetime. For diagnostics,
`bash play.sh --frames 120 --no-record` limits the run.

## Coverage and replay

```sh
bash collect-native.sh --dry-run
bash collect-native.sh --session session-ID
bash rebuild-native.sh --skip-build
bash replay.sh session-ID --skip-build
```

Collection validates ROM instruction bytes, keeps RAM variants, deduplicates
instructions and rejects a whole incompatible session. It never edits the ROM.
Rebuilding produces all libraries in a staging directory and installs them only
after every compilation succeeds; an installation failure restores the old
profile directory. Previous profiles are kept under `artifacts/`.

Windows and Linux reuse the CSV files, not binaries. A rebuild on Linux does not
replace Windows DLLs or their manifest. Rebuild Windows profiles separately after
collecting new coverage. Replays copy the initial save and compare video/audio
hashes; their default requires zero interpreter steps. Add `--allow-interpreter`
when investigating uncovered routes.

## Distribution and verification

The Linux packager produces `AdvanceLiberty`, four `.so` files, a platform-local
manifest, notices and license texts, and empty ROM/language directories. It copies
no ROM, saves, recordings, development scripts or extracted assets. Permissions
are preserved in the `.tar.gz`. Existing output directories are never overwritten.
Packaged users need no Rust, C compiler, Python or shell scripts.

Build on the oldest glibc distribution you intend to support; a build made on a
newer distribution may not run on an older one. This is not a universal Linux
binary or a musl build. Controller/audio/display behavior still needs testing on
real Linux desktops, including X11 and Wayland.

`.github/workflows/linux.yml` builds the runtime and runs unit, script and
synthetic compiled-instruction tests on Ubuntu without a ROM. Full native-profile
builds and game replays require a local user-supplied ROM and are not generated
or distributed by public CI.
