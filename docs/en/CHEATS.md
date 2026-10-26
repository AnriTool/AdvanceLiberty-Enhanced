# Original cheats through the PC panel

**English** | [Русский](../ru/CHEATS.md)

Ctrl+F12 opens the panel in the upper-right corner. In the game world, it
automatically enters A+B+Start if the original cheat-mode flag is still zero.
Reopening the panel does not toggle an enabled mode off. Press 0 to toggle
cheat mode itself; Escape/Ctrl+F12 only closes the panel.

| Key | Action | Original sequence after Left, Right, Up, Down |
| --- | --- | --- |
| 1 | All weapons | A, A |
| 2 | Health | B, B |
| 3 | Armor | A, L |
| 4 | $15,000 | L, L |
| 5 | Lower wanted level | A, R |
| 6 | Raise wanted level | R, A |
| 7 | Maximum / zero wanted level | R, R |
| 8 | Gang hostility | B, R |
| 9 | Level select, main menu only | L, R, then Start+A |

Arrow keys + Enter and gamepad D-pad + A also work. The panel does not pause
the world. Other controls are captured so selecting a cheat cannot move the
player or fire a weapon. The queue stops advancing when the window loses
focus. A sequence already in progress finishes even if the panel is closed
manually. New commands cannot be added while a sequence is being sent.

After item 9, the panel closes. The game adds a fourth entry,
PICK LEVEL (CHEAT), below Erase Game; scroll down to find it. This is an
additional menu row, not a separate screen that opens immediately.

## Implementation

`src/cheats.rs` contains the GBA button queue and panel presentation. The host
does not change game memory, money, health, weapons or mission state. Buttons
pass through normal `m.bus.keys` before input recording, so existing replays
reproduce them without a separate cheat sidecar file.

The USA cheat table is at 08caa264..08caa2db. R is bit 0x100 and L is 0x200.
The mode flag is read at 02011020; its original handler toggles it at
0803893c..08038948. The menu is recognized by observing its original loop at
080023fc. The level-select code is checked at 0803b920 and retained in the
menu's local state. Start+A changes descriptor 08332d4c to 08333148, adding
a fourth item to the original three. Observations are read-only, and input
is permitted only in the appropriate context.

Each button is held for 6 GBA frames and released for 6 frames, separating
repeated buttons such as A,A and L,L. Enhanced/Classic bindings do not change
the injected sequence. F12 is reserved for the panel.

Labels and hints use the `cheats_*` keys in `resources/menu-en.json`.
Language packs can translate them in their `menu` object; the panel uses
the pack's external pixel font.
