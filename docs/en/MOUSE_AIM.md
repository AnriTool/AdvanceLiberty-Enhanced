# Mouse aiming

**English** | [Русский](../ru/MOUSE_AIM.md)

Mouse aiming is enabled by default for new profiles. Its toggle is in
**Esc → Enhanced Options → On-foot bindings → Mouse aim (exp.)**.
Choose **Save and return** afterward. Existing profiles are not overwritten.

On foot in Enhanced, hold the **right mouse button**: the cursor becomes a
pixel crosshair and the player faces it. J and the left mouse button fire in
that direction. Without the hold, the cursor does not affect facing.
WASD/arrows retain their original screen-relative movement axes independently
of facing. Remappings still apply. Sprint blocks Fire; Jump retains its own
action. When the cursor is directly at the player's feet, the last angle is kept.

This mode applies only to on-foot Enhanced controls. Classic, vehicles,
pause/settings menus and focus loss disable the override. Moving outside the
window or the game area within letterboxing also disables it. Active gamepad
buttons use the original layout without mouse aiming.

Configuration: `MouseAim = true` before `[GBA]` in controls.cfg. An absent
field means false; F5 reloads it. Discard and return cancels edits; Save and
return saves them. F8 → Classic disables the mode without erasing its setting.

Remap the hold button in **On-foot bindings → Aim (hold)**: Enter replaces,
Insert adds an alternative, Delete disables. The list scrolls when selecting
lower rows. The `[OnFoot]` field is `Aim = MouseRight`; older configs receive
RMB automatically. If RMB was already assigned to another on-foot action,
that binding is preserved and Aim must be assigned manually. As with other
Enhanced actions, a button held while leaving menus or changing context must
be released before it can activate.

## Turning response

The initial implementation put body angle in the same history as the camera,
adding 6 GBA frames, about 100 ms, plus the wait for an original pose update.
With the mode enabled, only validated parts of the controlled player now
receive the current angle immediately before each Enhanced frame is composed.
The pivot stays at the displayed feet position. Camera, movement, NPCs,
bullets and HUD retain their timelines. Each part keeps the angle actually
read during placement, so corrections do not accumulate.

Releasing Aim immediately restores visual direction from original physics
and stops supplying the mouse angle to game logic. Recorded angles contain
`none` outside active aiming. Original rendering retains its own update rate;
the faster visual turn applies to Enhanced. Frame/input rates and animation
updates are unchanged.

## Implementation

mouse_aim.rs selectively overrides seven native USA ROM angle reads. It checks
the ROM instruction, physics address, current-player pointer and entity type.
Overrides affect animation, part placement, shot initialization, and the
flamethrower's continuous direction and nozzle offset (including its 16-bit
integer heading read). Physics
reads that calculate WASD movement receive the original angle. The ROM, DLLs
and angle stored in guest memory are not rewritten. The RtApi hook also applies
to internal AOT exits through the shared active API. Interpreter mode is unsupported.

Desktop converts coordinates through SDL_RenderWindowToLogical, accounting for
window size, letterboxing and HiDPI. Enhanced retains the player's root projection,
scale and world position from the interpolated scene actually displayed. Inverse
projection gives a world-space cursor target; facing is calculated from the current
physical player position. This accounts for the existing 6-frame camera presentation
delay. HUD scaling does not participate. Original output uses its own dimensions
and projection without the widescreen offset.

Object projection retains the root projection and part angles for aiming.
While active, it applies rotation through inverse texture sampling with expanded
sprite bounds, preserving ordering and transparency. The path is inactive when
aiming is disabled. Materials, HUD and camera settings keep their algorithms.
Wide remains 854×480, with the central area offset by 67 pixels. The crosshair
is a native 32×32 SDL cursor with a transparent center, white pixel rays and
a black outline; releasing Aim restores the arrow cursor.

## Recording and replay

On the first active aim, Session creates `aim.gba-aim` beside input.gba-input.
It records changes in the effective angle by frame:

```text
gba-aim v1
0 none
730 128
1050 768
1600 none
```

Angles 0..1023 cover one full turn: 0 up, 256 right, 512 down, 768 left.
`none` restores original behavior. The file does not exist before aiming is
first enabled. Older recordings work without it.

The runtime automatically reads aim.gba-aim beside input.gba-input, so replay.ps1
supports these recordings without new mandatory options. `--aim-input FILE.gba-aim`
is available for separate experiments. Invalid headers, angles, frame ordering,
or a missing initial frame-0 row are rejected. The stream does not enable the
user preference or change controls.cfg.
