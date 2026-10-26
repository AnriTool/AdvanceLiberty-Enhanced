# AdvanceLiberty: Enhanced

**English** | [Русский](README.ru.md)

An unofficial native wrapper for a game image supplied by the player.
This project does not distribute or download ROMs, BIOS files, saves or
extracted game assets. You must provide a legally obtained dump of your own
cartridge. The project is not affiliated with the original game's rights
holders and does not grant rights to the game.

## Portable release

Extract the complete folder, put your unmodified USA `.gba` dump in `roms/`
under any filename, and run **AdvanceLiberty.exe**. Dragging a ROM onto the
EXE also works. Players do not need Rust, a compiler, BAT or PowerShell.
Supported SHA-256:
`215c574d5afbda0aadbf89dc76b765cfaaeb35953eab63f7687d8bfe8a8e0f4d`.
Other regions and modified ROMs are rejected before the native profile starts.

Enhanced Audio and a 30-second drop lifetime are enabled by default. First
launch also defaults to 16:9, fullscreen and aiming while holding RMB.
Existing saved choices take precedence. Use `AdvanceLiberty.exe --original-audio`
for original audio and `AdvanceLiberty.exe --drop-lifetime 0` for original
cleanup rules. Saves/preferences are created in `data/` beside the EXE.
Sessions are recorded by default; `--no-record` disables recording. Launching
does not depend on the current working directory.

Build a distributable folder with `./package.ps1 -Offline`. It creates a new
folder and ZIP in `dist/`, excluding ROMs and user data.
[Portable build details](docs/en/PORTABLE.md).

## Running from source

**Play.cmd** also defaults to Enhanced Audio and a **30-second** target lifetime
for money, weapons and corpses. Use `Play.cmd -OriginalAudio` for original audio
or `Play.cmd -DropLifetime 0` for original cleanup rules.
[Audio processing](docs/en/ENHANCED_AUDIO.md).

Double-click **Play.cmd**, or run from PowerShell:

```powershell
.\play.ps1
```

The script works from any working directory and performs an incremental build
before launch; `-SkipBuild` uses the existing executable. It uses the checkout's
existing saves and controls, so starting a new game is unnecessary.
The development ROM path is `roms/usa.gba`, a local copy supplied by the user.
This native profile does not support the European release.

Packs in `languages/` appear automatically in **Esc → Enhanced Options → Language**.
First launch creates editable `languages/en.json` with 1774 recognized English
entries covering story text, side missions, statistics and menus. Catalog
updates add missing IDs while preserving your values. Select a language, go
back and choose **Save and return** to apply it during play. Later launches
can use normal Play.cmd; **Original game / ROM** disables the pack.
Pack v2 contains text, alphabet, regular and color pixel fonts, so adding a
language requires no Rust changes or rebuilding. Missing translations use
English. [Formats, extraction and limitations](docs/en/LOCALIZATION.md).

| Location | Purpose |
| --- | --- |
| `src/` | Runtime, Enhanced renderer, camera, input, audio and diagnostics |
| `tools/aot/` | Static recompiler and supporting utilities |
| `tools/localization/` | Language-table extraction and UTF-8 translation tools |
| `languages/` | External packs, including a Russian example |
| `vendor/` | Local ARM, GBA hardware-model and base-recompiler sources |
| `native/usa/` | Four active DLLs, generated C and loading manifest |
| `native/usa/coverage/` | Accumulated inputs for rebuilding native DLLs |
| `roms/` | User-supplied local ROMs |
| `data/saves/usa.sav` | Current game save; `.bak` is its backup |
| `data/config/controls.cfg` | Keyboard/mouse bindings and control mode |
| `data/config/enhanced.cfg` | Saved Enhanced Options |
| `data/sessions/` | Input, initial-save, video/audio-hash and fallback recordings |
| `target/` | Cargo outputs; runtime: `target/release/AdvanceLiberty.exe` |
| `dist/` | Portable user folders and ZIPs |
| `artifacts/` | Replay/check results and build logs |
| `docs/en/`, `docs/ru/` | Matching English and Russian feature/developer guides |
| `test-data/` | Minimal private fixtures excluded from Git and distribution |

## Controls and presentation

**Ctrl+F12** opens the right-hand cheat panel and enables original cheat mode
if necessary. **1–8** apply listed cheats; arrows + Enter or gamepad D-pad + A
also work. **0** toggles cheat mode itself; **Esc / Ctrl+F12** only closes the
panel. Reopening does not disable an already enabled mode. The world keeps
running while ordinary controls are captured. Sequences are sent to the original
game and recorded as input.

**9**, available only in the main menu, unlocks original **PICK LEVEL (CHEAT)**.
The panel then closes; scroll below **Erase Game** to find the added entry.
Panel labels use `cheats_*` keys in a pack's `menu` object.
[Cheat implementation](docs/en/CHEATS.md).

Enhanced rendering is enabled by default. Menus and unsupported scenes use
original output. The title shows FPS, native dispatch percentage, interpreted
instruction count and Camera preview status. Widescreen ground sampling
accounts for all three islands' map dimensions. Pause/settings backgrounds
follow the island: blue Portland, green Staunton, red Shoreside.
[Menu architecture](docs/en/MENUS.md).

Experimental cash protection against premature cleanup under affine-matrix
pressure is enabled with DropLifetime. To test it separately without extending
timers, use `Play.cmd -DropLifetime 0 -PreserveCash`. Pickup behavior and the
four-cash-pickup limit remain original.

Money, weapons and corpses have a default target lifetime of 30 seconds.
`Play.cmd -DropLifetime N` selects 1–300 seconds; `-DropLifetime 0` restores
original rules. It also enables cash protection. Resource shortages may remove
old corpses earlier. At most eight dropped weapons are retained, while the
money pool still has four slots. The game performs collection. Session metadata
records this option and restores it for replay; F6 changes rendering only.
[Retention rules and limits](docs/en/DROP_LIFETIME.md).

Enhanced keyboard controls are enabled by default and follow the currently
controlled entity:

| Action | On foot | In a vehicle |
| --- | --- | --- |
| Movement | WASD / arrows | W/S accelerate and brake/reverse; A/D steer |
| Fire / attack | J / left mouse button | — |
| Sprint | Shift | — |
| Jump | Space | — |
| Enter / exit vehicle | F | F |
| Change weapon | Tab | Original Select on Tab |
| Parking brake | — | Space |
| Horn / siren | — | Shift |
| Special mission | — | Caps Lock |
| Rotate turret | — | Ctrl + A/D |
| Game pause | Enter | Enter |

| Key | Shared action |
| --- | --- |
| F6 | Enhanced / fully Original renderer |
| F8 | Enhanced / original GBA controls |
| F11 | Fullscreen |
| F5 | Reload controls.cfg |
| Esc | Shared pause menu / back on menu pages |
| P | Pause runtime execution |

**Esc**, the **Pause** action, Enter by default, and gamepad Start/Guide open
the shared pause menu. Host lists for pause, Options, camera, audio and exit
confirmation use the USA ROM's regional backgrounds and yellow font. Arrows
select, Enter/J confirms, Esc goes back; mouse row selection also works.
Gamepad D-pad / A / B selects / confirms / goes back. Original game logic
performs actions, recorded as GBA commands. Brief, Stats, Credits, startup and
save-slot pages retain original presentation. Enter or Esc closes Brief/Stats/
Credits. Menus keep 3:2; returning to the world restores 16:9 and HUD sizes.

**Enhanced Options** is available from pause and Options. Settings paths below
start with **Esc → Enhanced Options**. In this editor, game execution and game
audio stop completely; edits apply only when saved.
**Quit to Menu → Save and Quit** saves progress and returns to startup.
**Quit to Desktop** offers **Save game and exit**, **Exit without saving** and
**Back**. The first runs the original save procedure, commits the file with a
backup, then closes. If saving is forbidden or a disk error occurs, the game
stays open with an explanation. This choice is available in a loaded game.
Unsaved PC edits are discarded on exit and saved separately with Save and
return. Host-menu and preference labels can be translated through a pack's
optional `menu` object. [Format and example](docs/en/LOCALIZATION.md#translating-the-host-menu).

On original startup pages, Enter confirms and Esc goes back; configured GBA
A/B bindings remain alternatives. Guide opens Enhanced Options there. During
P-pause, Esc opens Enhanced Options directly. Navigation, selection and back
actions play the original game click, including added host rows. Volume follows
SFX Volume, including Off; clicks also work during P-pause. Opening and closing
settings during P-pause preserves that pause. Release menu buttons and press
P to resume.

For diagnostic comparison, `./play.ps1 -OriginalMenus` restores original menus
without changing saved preferences. `tools/diagnostics/record-menu.ps1` also
uses this mode.

Options include Enhanced/Classic controls, three independent binding pages,
Enhanced/Original rendering, Perspective / experimental PS1-style Affine
textures, and fullscreen. Enter replaces all bindings for an action; Insert
adds an alternative; Delete clears it. After Enter/Insert, press the desired
key or mouse button. Esc cancels capture. Conflicts/reserved keys are rejected
without changing the profile. Classic GBA buttons cannot be left unbound.

During play, **V** cycles **Auto → Low → Medium → High** camera positions.
Rebind it in **Graphics → Cycle camera key**: Enter replaces, Insert adds an
alternative, Delete disables; then Save and return. It works on foot/in a
vehicle with either control mode and renderer.

**Graphics → Aspect ratio → 16:9 (experimental)** extends world visibility
horizontally without stretching sprites or textures. Radar/status move toward
the wide edges; messages stay centered. Save and return applies the choice.
Windowed mode adopts its proportions; F11 toggles fullscreen. Original menus,
Original rendering and Camera preview fallbacks retain 3:2. **Original 3:2**
restores the original aspect; first launch defaults to 16:9. Unsupported scenes
may temporarily fall back to Original, with the reason recorded in session
logs. Texture style is in the same Graphics submenu.
[Widescreen implementation and limits](docs/en/WIDESCREEN.md).

**HUD size** offers 85%, 75%, 65% or 50% in addition to 100%. **All elements**
sets all groups; **Radar and markers** controls radar/markers; **Status and
weapon** controls clock, money, health, wanted level and weapon icon;
**Messages** controls background-UI labels/messages. Left/Right changes sizes;
**Reset HUD sizes** restores 100%. Anchors are upper left for radar, upper right
for status, and lower center for messages. Scaling applies to Enhanced world
presentation; Original output and game menus retain original sizes. Values
are saved with other preferences.

**Save and return** applies edits and persists them for future launches. Esc in
a submenu returns to the settings root; Esc there or **Discard and return**
cancels unsaved edits. Graphics settings use data/config/enhanced.cfg, controls
use controls.cfg, and controls.cfg.options-backup is created before saving.
Opening settings alone does not modify files. Held buttons are suppressed
after closing until released. Exiting through the menu asks for confirmation
and discards unsaved preference edits.

Manual binding edits go in data/config/controls.cfg: `[GBA]` holds ten original
buttons; `[OnFoot]` and `[Vehicle]` hold independent actions. Press F5 after
editing. Mode = Enhanced/Classic selects the next-launch mode; F8 is temporary.
The old layout is backed up in data/config/controls-gba-backup.cfg. Changing GBA
R to S or Select to Shift does not affect Enhanced controls. Pause and auxiliary
actions are also remappable. Held actions cannot turn into different actions
across contexts without release, while matching WASD movement continues when
entering/leaving vehicles.

New profiles enable **Mouse aim (exp.)**. Hold **RMB** on foot in Enhanced:
the cursor becomes a pixel crosshair, the player faces it, and J/LMB fires
there. Without the hold, normal movement-facing behavior applies. Remap RMB
with **On-foot bindings → Aim (hold)**. WASD remains independent of facing;
Sprint still blocks Fire. It works with 16:9 and scaled HUD. Vehicles and
Classic retain their original controls. The mode remains experimental; not
all weapons and scenarios have been checked. [Mouse aiming](docs/en/MOUSE_AIM.md).
Gamepads retain the original GBA layout. [Binding format](docs/en/CONTROLS.md).

SDL GameController is supported. Launch-script `-OriginalRenderer` and
`-AffineTextures` override saved rendering/texture choices. `-Rom`, `-SavePath`
and `-ControlsPath` select alternative paths.

By default each launch creates a new data/sessions/ recording. Exit normally
through the menu or window close button to finalize its report; partial audits
are updated during play. camera-events.csv records Enhanced fallback reasons,
recovery and graphics changes; camera-summary.txt contains final counters.
Ordinary game pause is classified as a menu. `-NoRecord` disables recording;
`-SessionDirectory NEW_DIRECTORY` selects its location.

## Building and validation

Development builds need Rust/Cargo, MSVC Build Tools and CMake. SDL builds
automatically through Cargo; no separate SDL DLL is needed at runtime. The
first build needs Cargo dependency access or a populated local cache.

```powershell
.\build.ps1
.\build.ps1 -Tools
.\test.ps1 -Sdl
```

`-Tools` also builds the recompiler/disassembly utilities. `-Offline` makes
builds and tests use only cached dependencies. vendor/ already contains the
necessary changes; no separate project checkout or patch step is needed.
Cargo uses sequential builds to avoid parallel MSBuild hangs while building
SDL in this Windows environment.

Replay a completed recording with current DLLs:

```powershell
.\replay.ps1 -Session session-0123456789abcdef0123456789abcdef
```

Replay uses a copy of the initial save in a new artifacts/ directory and
compares every video/audio hash. Zero interpretations are required by default.
`-AllowInterpreter` permits investigating an uncovered route;
`-OutputDirectory NEW_DIRECTORY` chooses its output location.

## Native code

Library order and paths are defined in native/usa/manifest.json. No code is
compiled during gameplay. Uncovered instructions may enter the counted
interpreter fallback; zero on tested routes does not establish whole-game
coverage. Rebuilding DLLs requires **MinGW x64 GCC**, separately from Rust's MSVC.

After playing, run **UpdateNative.cmd** to accumulate coverage and rebuild/install
the sessions DLL. Long replays are not run by default. **UpdateNative.cmd -Verify**
validates contributing recordings before installation. CollectNative.cmd only
collects CSV; RebuildNative.cmd only rebuilds/installs the sessions profile.
[Commands, validation and backups](docs/en/NATIVE_WORKFLOW.md).

For a separate rebuild:

```powershell
.\tools\rebuild-native.ps1 -NativeProfile sessions
```

Results default to a new artifacts/native-build-.../ directory. `-Install`
installs successfully built DLL/C files in native/usa/; `-NativeProfile all`
rebuilds all four, and `-Gcc PATH` selects the compiler. New fallbacks are first
merged into the corresponding accumulated CSV, then rebuilt and replay-checked.
All four coverage CSVs and the manifest are versioned. Commit modified coverage
and native/usa/manifest.json together so variants can be rebuilt on another
machine with the same USA ROM. ROMs, generated C/DLL build outputs, saves and
recordings belong to local development data. Dependency licenses are in vendor/.

[Architecture](docs/en/PROJECT.md) · [Native coverage](docs/en/NATIVE_WORKFLOW.md) ·
[Language packs](docs/en/LOCALIZATION.md) · [Menus](docs/en/MENUS.md) ·
[Widescreen rendering](docs/en/WIDESCREEN.md)
