# AdvanceLiberty: Enhanced

**English** | [Русский](README.ru.md)

## Bring your own ROM

**No ROM, BIOS or extracted original game assets are included in release packages.**
We do not distribute or download ROMs, BIOS files, original game graphics, music, fonts
or story text. Resources are read from your own legally obtained cartridge
dump. This is an unofficial project, unaffiliated with the game's rights
holders; it does not grant rights to the original game.

AdvanceLiberty: Enhanced is a native PC runtime that adds modern presentation
and controls while preserving the original game logic.
Build targets: **Windows x64** and **Linux x86_64**.
See the [Linux guide](docs/en/LINUX.md) for dependencies and source builds.

## Features

**Enhanced graphics**

- Smoother camera and sprite presentation, with camera modes switchable during play.
- 16:9 world rendering, perspective-correct textures and adjustable HUD sizes.
- Original rendering remains available for comparison.

**Enhanced controls**

- Context-aware WASD controls for walking and driving.
- Mouse aiming while holding RMB, plus separate sprint, jump and attack actions.
- Remappable keyboard/mouse bindings and an optional Classic GBA layout.

**Other improvements**

- Enhanced Audio with the original SFX/Music volume controls.
- A 30-second target lifetime for dropped money, weapons and corpses, within game resource limits.
- External language packs with custom pixel fonts and live language switching.
- [Gameplay replays](docs/en/REPLAYS.md): F2 records, F3 plays the latest, F4 opens the library.

## Quick start

1. Extract the complete release folder.
2. Put your **unmodified USA `.gba` dump** in `roms/`, under any filename.
3. Run **AdvanceLiberty.exe** (Windows) or **./AdvanceLiberty** (Linux).
   Packaged releases need no build tools.

New profiles default to Enhanced rendering/audio/controls, 16:9 and fullscreen.
Open **Esc → Enhanced Options** to configure the game. Existing saved choices
take precedence. European and patched ROMs are not supported by this native profile.

## Built on

- [JRickey/gba-recomp](https://github.com/JRickey/gba-recomp): static recompilation,
  ARM/GBA hardware-model components and the basis of Enhanced Audio.
- **Rust**: runtime, native-code dispatch, Enhanced renderer and PC features.
- **SDL2**: windowing, input, controllers and audio output.

Local dependency provenance is documented in [vendor/README.md](vendor/README.md).
The project uses ahead-of-time native profiles; uncovered instructions may
still use a measured interpreter fallback.

## License and documentation

Original project code is available under the [MIT License](LICENSE).
Third-party components retain their own licenses and notices; see [NOTICE](NOTICE).
This grant does not license the original game or ROM-derived content.

[User and developer guides](docs/en/README.md) ·
[Native coverage tools](docs/en/NATIVE_WORKFLOW.md) ·
[Building a portable release](docs/en/PORTABLE.md)

[CI builds and GitHub Releases](docs/en/RELEASING.md)
