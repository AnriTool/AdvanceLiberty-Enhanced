# Agent guide

## Project and scope

AdvanceLiberty: Enhanced is a native PC runtime for the original GBA game,
targeting Windows x64 and Linux x86_64. It preserves guest game logic while
adding Enhanced rendering, controls, audio, menus, localization and replays.
The root Cargo package is `advance-liberty`; its executable is `AdvanceLiberty`.
Read the current release version from `Cargo.toml`, not from examples or this file.

The workspace is self-contained. Local `vendor/armv4t`, `vendor/gba-core` and
`vendor/recomp-core` originate from JRickey/gba-recomp and include project changes.
See `vendor/README.md` for provenance. An external recompilation checkout or an
old experiment directory must not be required to build or run the project.

Users must supply their own legally obtained ROM. Do not distribute or download
ROMs, BIOS files or extracted original game graphics, audio, fonts or story text.
Do not add private ROMs, saves, recordings or generated English text to Git or
release packages. Preserve the existing versioned instruction coverage used by
ROM-free builds. Original project code is MIT; preserve third-party licenses,
attribution and `NOTICE`. Do not imply affiliation with the game's rights holders.

The native profile supports only the exact unmodified 16 MiB USA ROM; validation
lives in `src/rom_profile.rs`. European and patched/translated ROMs are unsupported.
Translations are external packs applied in memory to the USA runtime. Do not
weaken identity checks to accept an incompatible ROM.

## Repository map

| Location | Responsibility |
| --- | --- |
| `src/main.rs`, `launcher.rs` | Worker execution, CLI parsing, portable startup and validation |
| `src/native_host.rs`, `block_check.rs`, `coverage.rs`, `native_audit.rs` | Native dispatch, equivalence checks, interpreter accounting and audits |
| `src/camera_preview.rs`, `scene_replay.rs`, projection/material/mask modules | Enhanced world capture and rendering |
| `src/desktop.rs`, `frame_pacing.rs`, `hud_layout.rs`, `viewport.rs` | SDL presentation, frame submission, HUD and aspect ratio |
| `src/controls.rs`, `control_context.rs`, `gamepad.rs`, `mouse_aim.rs` | Input mapping, on-foot/vehicle context and aiming |
| `src/options.rs`, `render_settings.rs`, `game_menu.rs`, `menu_text.rs`, `menu_audio.rs` | Preferences, menus, translated UI and menu sounds |
| `src/enhanced_audio.rs`, `polyphonic_audio.rs`, `vendor/gba-core` | Host audio processing and guest hardware/DMA/FIFO timing |
| `src/localization.rs`, `language_font.rs`, `languages/` | Language pack discovery, validation, text and custom fonts |
| `src/replay.rs`, `replay_panel.rs`, `session.rs`, `save.rs` | Gameplay replays, diagnostic sessions and cartridge saves |
| `tools/aot/` | AOT generation and instruction analysis; package `advance-aot-supplement` |
| `tools/font-editor.html`, `tools/localization/`, `tools/diagnostics/` | Font editing, translation and reusable diagnostic tools |
| `scripts/windows/`, `scripts/linux/`, `scripts/ci/`, `scripts/tests/` | Platform tools, release automation and script tests |
| `native/usa/coverage/`, `native/usa/manifest.json` | Shared versioned coverage and Windows profile manifest |
| `native/linux-x86_64/` | Generated Linux libraries and platform-local manifest |
| `resources/`, `build.rs` | Application resources, icon and Windows EXE metadata |
| `release/`, `dist/` | Distribution source notices/instructions; generated packages |
| `data/`, `roms/`, `replays/`, `test-data/`, `artifacts/`, `target/` | Private user data, ROMs, clips, fixtures, diagnostics and build outputs |

`README.md` is the concise English introduction; `README.ru.md` is its Russian
counterpart. Detailed guides are indexed by `docs/en/README.md`, with matching
`docs/ru/` guides. Keep relevant English/Russian documentation in sync. Describe
current mechanisms and workflows rather than appending session repair histories.

## Build and run

Run commands from the repository root. Root BAT/SH files are developer entry
points; implementation belongs under `scripts/windows/` or `scripts/linux/`.
Packaged users launch the executable directly and need no developer scripts.

Windows needs Rust/MSVC, Windows SDK and CMake. SDL2 is built from the bundled
Cargo dependency and linked statically. Native DLL generation additionally needs
MinGW x64 GCC; it is separate from the MSVC Rust toolchain.

```powershell
./scripts/windows/build.ps1 -Offline -Tools
./Play.bat -SkipBuild
./Play.bat -SkipBuild -RecordSession
./scripts/windows/package.ps1 -Offline
```

`-Offline`/`--offline` requires cached dependencies; omit it for a first online
build. Keep `--locked` in reproducible build scripts. Development launch defaults
to `roms/usa.gba`; the portable launcher discovers a valid `.gba` in `roms/`.
Development saves/preferences are under `data/`. Use explicit private copies for
experiments and replay diagnostics; do not overwrite a player's save.

Windows PowerShell does not reliably wait for GUI-subsystem executables invoked
with `&`. For worker diagnostics, dot-source `scripts/windows/Project.ps1` and use
`Invoke-GameRuntime` with `-Executable`, an argument array and optional `-LogPath`.
Use `Get-NativeLibraries` for manifest-ordered libraries. Close the game before
replacing its executable or DLLs; do not terminate a player's session without
authorization.

Linux needs Bash, Python, Rust, CMake, pkg-config, a host C compiler and SDL system
build dependencies listed in `docs/en/LINUX.md` and the CI workflow.

```sh
bash build.sh --native --coverage-only
bash play.sh --skip-build
bash play.sh --skip-build --record-session
bash test.sh --offline
bash package.sh --skip-build
```

Linux uses native `.so` profiles, not Windows DLLs or Wine. Both platforms share
coverage CSVs but rebuild their own binaries. Do not claim Linux validation from
a Windows build; report separately what ran locally and what CI must verify.

## Native coverage and execution correctness

The runtime uses address-based AOT dispatch with a measured interpreter fallback
for uncovered instructions. Title percentages measure dispatch coverage on the
executed route, not static coverage of the entire game. Never conceal fallback
counts or describe a partly covered route as fully native.

Profile order comes from the manifest: precise, extra, mission, sessions. Keep
CSV coverage and the manifest versioned. Generated C, DLL and SO files are build
outputs. Do not manually replace manifest hashes or hard-code variant counts.

Diagnostic recording is opt-in (`--record-session` / `-RecordSession`). After a
completed session, Windows users can run:

```powershell
./CollectNative.bat -DryRun  # inspect additions
./UpdateNative.bat          # collect, rebuild sessions profile, install
./UpdateNative.bat -Verify  # also compare contributing sessions
```

`CollectNative` collects CSV only; `RebuildNative` compiles collected coverage;
`UpdateNative` combines them. Advanced Windows options belong to
`scripts/windows/rebuild-native.ps1`. Linux has `collect-native.sh` and
`rebuild-native.sh`. Consult `docs/en/NATIVE_WORKFLOW.md` before changing this flow.

Validate ROM identity, audited instruction bytes, alignment, RAM variants and
installed hashes. Reject incompatible recordings as a whole. Do not consume
partial recordings, silently skip unsupported instructions, or discard distinct
guarded RAM variants. Stage installation and retain rollback/backups on failure.
`--coverage-only` builds from versioned audited snapshots without a ROM; public CI
must remain ROM-free.

Routine collection need not replay every historical session. Generator, CPU,
interrupt, memory or timing changes require meaningful equivalence regression.
Compare canonical video/audio hashes on compatible recordings and investigate
the first divergence. Compilation alone proves neither timing nor semantic
equivalence. Use copies of initial saves, preserving live progress.

## Runtime invariants and investigation

- Preserve canonical guest execution and hardware timing when changing host
  presentation/audio. Enhanced render output and processed audio can differ
  intentionally; compare the correct canonical or Enhanced reference.
- Original rendering and Classic controls must remain available. New profiles
  default to Enhanced rendering/controls/audio, simultaneous PCM effects,
  fullscreen, 16:9, mouse aiming and a 30-second drop lifetime. Saved preferences
  take precedence. Update all relevant launchers when changing defaults.
- `camera_preview` is the established Enhanced renderer despite its internal
  name. Do not reintroduce its diagnostic label into the window title. Retain
  FPS, native dispatch and interpreted-operation reporting.
- Check all three regions, driving, sprinting, composite sprite animations and
  bridge transparency for renderer changes. Keep world/HUD alignment and valid
  Original fallback behavior. Do not assume every region has the same map data
  or menu background.
- Judge smoothness using actual submission intervals and audio health, not FPS
  alone. `--frame-profile` exposes `present_interval_ns` and
  `present_lateness_ns`; `--capture-profile` diagnoses capture work. Expensive
  preview exports affect timings. Use repeatable clips and comparable settings.
- Scene captures own isolated guest state. Shared ROM/decode data uses copy on
  write. Preserve snapshot isolation, painter order, bounded worker queues and
  worker shutdown before native libraries unload.
- Polyphonic audio adds observation-only PCM voices using actual DMA/FIFO
  timing. Preserve guest memory/original audio, paused voice handling and sample
  deduplication. Do not stack repeated skid sounds. Original Audio takes
  precedence; replay snapshots retain the recorded audio mode.
- Language packs define their own alphabets and normal/color fonts. Do not
  hard-code Cyrillic or another language in Rust. Keep live switching in memory,
  preserve in-flight text allocations, and retain English fallback entries.
  Generated `languages/en.json` remains private and ignored.
- Gameplay clips in `replays/` are separate from diagnostic sessions in
  `data/sessions/`. F2 records, F3 plays latest, F4 opens the browser. Exiting
  playback restores live guest/audio/presentation state without writing replay
  progress into cartridge saves. Viewing-time renderer switches must not modify
  the clip or suspended live settings. F8 is retired.

Detailed behavior and flags are documented in `WIDESCREEN.md`, `ENHANCED_AUDIO.md`,
`LOCALIZATION.md`, `REPLAYS.md`, `CONTROLS.md` and related guides under `docs/en/`.

## Verification

Choose checks for the change. Documentation-only edits need link/path and diff
review; do not rebuild the entire game for prose changes. For runtime or script
changes, use relevant tests and the platform wrappers:

```powershell
./scripts/windows/test.ps1 -Offline
./scripts/windows/test.ps1 -Offline -Sdl
python -m unittest discover -s scripts/tests -v
```

SDL integration tests are normally ignored and run individually with dummy video,
dummy audio and the software renderer; the `-Sdl` wrapper configures them.
Ignored tests requiring private ROMs/native fixtures are not public CI checks.
See `test-data/README.md`. Vendor CPU/hardware changes also need the relevant
vendor package tests, not just application tests.

For restricted Windows environments, use a writable temporary directory under
`artifacts/` for both `TEMP` and `TMP` if system-temp permissions interfere. If
Git reports dubious ownership, use per-command `-c safe.directory=<checkout>`
rather than changing global configuration. Keep private reports under
`artifacts/`; avoid whole-workspace formatting of unrelated/vendored files.
Report the checks actually run, failures and remaining platform limitations.

## Releases and icons

Read `docs/en/RELEASING.md` and `PORTABLE.md` before packaging or publication.
After changing the root package version, synchronize and validate the lockfile:

```sh
cargo update --offline --package advance-liberty
python scripts/ci/release.py check-version
```

Commit `Cargo.toml` and `Cargo.lock` together. Do not remove `--locked` to work
around a stale lockfile. Keep unrelated dependency upgrades out of version bumps.

`resources/icon.png` is the 1024×1024 source. Run
`python tools/prepare-icon.py` with Pillow after replacing it; commit `icon.png`,
`icon.ico` and `icon.rgba` together. Normal builds/CI need no Pillow. `build.rs`
embeds Windows icon/version metadata using the SDK resource compiler (or
`WINDRES` for GNU). SDL embeds the RGBA icon on both platforms.

CI in `.github/workflows/build.yml` builds/tests/packages Windows and Linux.
Branch/PR runs create artifacts. Pushing `v` plus the exact package version
publishes a GitHub Release after both platforms succeed; stable versions become
Latest, suffixed versions are prereleases. Preparing code does not itself
authorize publishing: push release tags only when the user requests it. Do not
move published tags or overwrite published assets.

Packages use explicit file lists and contain the runtime, native profiles,
manifest and required notices/licenses, with user ROM/language directories.
Keep ROMs, extracted game assets, saves, sessions, replays, diagnostic reports
and developer tools out of release archives. Preserve checksum verification and
platform-specific manifests.
