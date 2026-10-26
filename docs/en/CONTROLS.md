# Configurable Enhanced controls

**English** | [Русский](../ru/CONTROLS.md)

Enhanced keyboard/mouse controls are enabled in `data/config/controls.cfg`.
F8 temporarily switches Enhanced/Classic independently of the F6 renderer
switch. For a persistent choice, use **Esc → Enhanced Options → Control mode
→ Save and return**, or change `Mode = Enhanced` to `Mode = Classic`.
F5 reloads the file; an invalid file leaves the previous working settings intact.

During play, **V** cycles the original camera positions:
**Auto → Low → Medium → High → Auto**. It works on foot and in vehicles,
with Classic/Enhanced controls and both renderers. Holding the key does not
repeat the switch. It is inactive in menus, during pause and without window
focus. Configure it in **Enhanced Options → Graphics → Cycle camera key**:
Enter replaces the binding, Insert adds an alternative, Delete disables it.
Choose **Save and return** afterward. The file uses a separate section:

```ini
[Shortcuts]
CycleCamera = V
```

Multiple keys/mouse buttons can be separated by commas; `None` disables the
action. The shortcut is shared by all profiles, so conflicts with any gameplay
binding are rejected. Older files without this section receive V automatically.
If V is already assigned to a custom action, camera switching stays disabled
until explicitly assigned. Changes are recorded as `# camera FRAME MODE` and
replayed at the same frame boundary. This is the original USA camera setting
at `02010c66`, values 0–3, rather than the Enhanced/Original renderer switch.
The menu handler at `08008028` changes only that byte.

## Bindings

`[GBA]` defines all ten original buttons: A, B, Select, Start, Right, Left,
Up, Down, R and L. This section is used in Classic and menus. Existing custom
J/Space/F/H bindings are retained; the pre-Enhanced layout is backed up in
`data/config/controls-gba-backup.cfg`.

`[OnFoot]` supports Up, Down, Left, Right, Fire, Sprint, Jump, EnterVehicle,
ChangeWeapon, Pause, AuxiliaryR and Aim. `[Vehicle]` supports Accelerate,
BrakeReverse, SteerLeft, SteerRight, ParkingBrake, Horn, SpecialMission,
TurretModifier, ExitVehicle, Pause and ChangeWeapon. Profiles are independent:
`[GBA]` is used only in Classic and menus, while Enhanced uses actions from
the current gameplay profile. Pause defaults to Return, on-foot AuxiliaryR
to H, and vehicle ChangeWeapon to Tab, the original Select button. Optional
fields may be omitted to use their defaults. Assigning GBA R to S or Select
to Shift does not conflict with Enhanced movement or the horn.

Values are physical SDL key names; separate alternatives with commas. The
Windows keyboard layout does not change the physical key position.
`MouseLeft`/`LMB`, `MouseMiddle`/`MMB` and `MouseRight`/`RMB` are supported.
`None` disables an individual Enhanced action. Example edits:

```ini
[OnFoot]
Fire = J, MouseLeft
Sprint = Left Shift, Right Shift
Jump = C
EnterVehicle = F
ChangeWeapon = Tab
Pause = Return
AuxiliaryR = H
Aim = MouseRight

[Vehicle]
Accelerate = W, Up
BrakeReverse = S, Down
SteerLeft = A, Left
SteerRight = D, Right
ParkingBrake = Space
Horn = Left Shift, Right Shift
SpecialMission = CapsLock
TurretModifier = Left Ctrl, Right Ctrl
ExitVehicle = F
Pause = Return
ChangeWeapon = Tab
```

This is an excerpt from an existing file: keep `[GBA]`. Omitted Enhanced
actions use defaults. A key may have different actions on foot, in a vehicle
and in Classic, but conflicts between simultaneous actions in one context are
rejected. TurretModifier uses the current SteerLeft/SteerRight bindings, so
remapping steering also remaps turret direction. Caps Lock acts as a physical
press, not a persistent toggle state.

P, Escape, F5, F6, F8, F11 and F12 are reserved for host functions. By default,
the left mouse button sends Fire. **Mouse aim (exp.)** is enabled for new
profiles; its toggle is at the end of On-foot bindings. Holding Aim, right
mouse button by default, separates facing/firing from WASD. Without the hold,
the cursor does not turn the player. Aim is remappable and sends no GBA buttons.
The setting is stored as `MouseAim = true` beside Mode before `[GBA]`.
An absent field in an older profile means false. F5 reloads it with bindings;
F8 → Classic temporarily disables aiming. [Implementation](MOUSE_AIM.md).
SDL GameController retains the original GBA layout; Enhanced profiles currently
apply to keyboard and mouse.

On foot in Enhanced, holding Sprint blocks Fire, including mouse Fire, so
running plus attacking does not become a jump. Explicit Jump still works
while running. The rule follows actions after remapping; Classic retains
the original game combinations.

## Context detection

`control_context.rs` reads the USA controlled-entity pointer at `02000f10`.
ROM code at `08001484..080014a0` reads that pointer and the type byte at +8:
2 means player, 4 means vehicle. Entering a vehicle changes the global pointer
to that vehicle; leaving changes it back to the player.

Additional checks validate EWRAM ranges/alignment, the ROM descriptor type,
the physical-position relationship and the animation back-reference. Menus
with world BG2/OBJ disabled retain ordinary GBA buttons. A brief upload-phase
forced blank does not change the control layout. Detection is independent
of camera position, interpolation and renderer choice. Guest memory is unchanged.

Actions map to original active-low GBA buttons:

| Action | Sent to the game |
| --- | --- |
| Fire / Sprint | A / B |
| Jump | A+B |
| EnterVehicle / ExitVehicle | L |
| ChangeWeapon | Select |
| Accelerate / BrakeReverse | A / B |
| SteerLeft / SteerRight | Left / Right |
| ParkingBrake | R |
| Horn | Up+R |
| SpecialMission | L+R |
| TurretModifier + direction | A+B+Left/Right |

`InputLatch` prevents a held Shift from changing sprint into horn, Space from
changing jump into parking brake, or F from changing enter into exit. Release
and press again to activate the new action. Matching movement directions
continue across on-foot/vehicle changes. Opening a menu does not carry a held
action into navigation. These new restrictions do not apply in Classic.
