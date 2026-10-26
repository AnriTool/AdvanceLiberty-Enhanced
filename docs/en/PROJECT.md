# Project architecture

**English** | [Русский](../ru/PROJECT.md)

AdvanceLiberty: Enhanced is a Cargo workspace containing the application,
an AOT tool and three local dependencies. `Cargo.lock` pins dependency versions.
Building and running do not require external experiment directories or a
separate gba-recomp checkout.

## Execution

The `advance-liberty` package builds `target/release/AdvanceLiberty.exe`.
The launcher validates the USA ROM and DLL hashes; the native runtime executes
game logic through an address table. `gba-core` provides the hardware model.
Uncovered instructions count as interpretations in the title and audit.
Zero interpretations on one route does not establish coverage of the entire game.

`native/usa/manifest.json` defines profile order: precise, extra, mission,
sessions. Each has accumulated CSV coverage in `native/usa/coverage/`.
Read current variant counts and hashes from the manifest rather than fixed
numbers in documentation. CSV and the manifest are versioned; generated C/DLL
files remain local build outputs.

## Modules

| Area | Files |
| --- | --- |
| Execution and hooks | `main`, `native_host`, `block_check` |
| World presentation | `camera_preview`, projection, material, mask and HUD modules |
| Window, input and preferences | `desktop`, `controls`, `gamepad`, `options` |
| Game menus | `game_menu`, `menu_text`, `menu_audio` |
| Audio | `gba-core` hardware path, `enhanced_audio`, SDL mixer |
| Language packs | `localization`, `language_font`, `resources/` |
| Persistence and diagnostics | `save`, `session`, `native_audit`, trace modules |

## Directories

`src/`, `resources/` and `vendor/` are build inputs. `tools/aot/` contains the
native instruction generator and disassemblers. `tools/diagnostics/` and
`tools/localization/` provide reusable analysis and translation utilities.
Original third-party licenses and authorship are retained in `vendor/`.

`data/` holds private settings, saves and session recordings; `roms/` holds
the user's ROMs. `languages/` stores language packs. Keep these separate from
build outputs. `target/` contains the Cargo cache and executables;
`artifacts/` holds new diagnostic reports and backups created before DLL
installation. Minimal private regression fixtures live in `test-data/`.

`release/` contains source instructions and notices for the distribution;
`dist/` contains packaged user folders and ZIPs. `package.ps1` uses an explicit
file list and excludes ROMs, user data, fixtures, research history and developer
tools. Documentation uses matching `docs/en/` and `docs/ru/` trees.

## Commands

Source builds require Rust/Cargo, MSVC Build Tools with the Windows SDK, and
CMake. SDL is built automatically and linked statically. The first build needs
access to Cargo dependencies; use `-Offline` only with a populated local cache.
Rebuilding native profiles additionally requires MinGW x64 GCC, separately
from Rust's MSVC toolchain. No separate gba-recomp checkout or patch step is needed.

```powershell
./build.ps1 -Offline -Tools
./test.ps1 -Offline -Sdl
./Play.cmd -SkipBuild
./package.ps1 -Offline
```

`Play.cmd` uses the development checkout's saves and preferences. The portable
release runs through its EXE without BAT/PowerShell or a compiler. CollectNative,
UpdateNative and RebuildNative retain their separate roles.

[Portable build](PORTABLE.md) · [Native coverage](NATIVE_WORKFLOW.md) ·
[Local fixtures](../../test-data/README.md)
