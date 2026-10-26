AdvanceLiberty: Enhanced
Windows x64 / preview

Russian instructions: README.ru.txt

This is an unofficial native wrapper for a game image supplied by the player.
We do not distribute ROMs, BIOS files, saves or extracted game assets.
You must provide a legally obtained dump of your own cartridge. The application
does not download the game or grant rights to it. This project is not affiliated
with the original game's rights holders.

STARTING THE GAME
1. Extract the complete folder to a writable location.
2. Put your own unmodified USA ROM (.gba) in roms.
   Any filename is accepted. You can also drag a ROM onto the EXE.
3. Run AdvanceLiberty.exe. No scripts or compiler are required.

Supported SHA-256:
215c574d5afbda0aadbf89dc76b765cfaaeb35953eab63f7687d8bfe8a8e0f4d
European images and ROM patches are not supported yet.
A missing or incompatible ROM produces an explanatory dialog.

DEFAULTS
Enhanced rendering, Enhanced Audio and modern controls.
16:9, fullscreen and aiming while holding the right mouse button.
Existing saved choices take precedence.
Money, weapons and corpses have a target lifetime of 30 seconds. Resource
shortages may remove them earlier; original game-pool limits remain.

CONTROLS
On foot: WASD moves, Shift sprints, J/LMB fires or attacks,
Space jumps, F enters a vehicle, Tab changes weapon.
Vehicles: W/S accelerates or brakes/reverses, A/D steers, Space is the parking
brake, Shift sounds the horn/siren, F exits, Ctrl+A/D rotates the turret,
and Caps Lock starts a special mission.
V cycles Auto/Low/Medium/High camera. Esc/Enter opens the game menu.
Enhanced Options contains graphics, language and rebinding settings.
Mouse aim (exp.) in On-foot bindings can disable mouse aiming.
F6 switches Enhanced/Original rendering; F8 switches Enhanced/Classic controls;
F11 toggles fullscreen. Ctrl+F12 opens the cheat panel.

USER DATA
data/saves/usa.sav stores game progress; .bak is its previous version.
data/config contains preferences; data/logs contains launch logs.
data/sessions contains diagnostic gameplay recordings.
On first launch, languages/en.json is extracted from your own ROM.
Put extra language JSON packs in languages and select them in the menu.
Do not include personal ROMs, saves or session recordings in a public archive.

EXE OPTIONS
--original-audio       original hardware audio
--drop-lifetime 0      original item/corpse cleanup rules
--no-record            disable diagnostic session recording
--rom "path to ROM"    use a file outside the game folder

Native libraries must remain in native/usa. Do not mix libraries from different
versions or run two instances using the same save. Third-party license texts
and notices are in licenses.

Original project code is MIT-licensed; see LICENSE and NOTICE in this folder.
Third-party components keep their original terms. The grant does not cover
the original game or ROM-derived content.
