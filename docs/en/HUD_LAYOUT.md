# HUD scaling

**English** | [Русский](../ru/HUD_LAYOUT.md)

Setting: Esc → Enhanced Options → HUD size. Available sizes are 100, 85, 75,
65 and 50 percent. All elements changes all three groups; the other entries
are independent. Save and return stores HudRadar, HudStatus and HudMessages
in enhanced.cfg. Missing fields in older files default to 100%; no manual
migration is required.

## Groups and coordinates

Source coordinates use the original 240×160 GBA screen. Rectangles are half-open.

| Group | Source region | Anchor | Contents |
| --- | --- | --- | --- |
| Radar | x=[0,64), y=[0,64) | (8,8) | BG1 map, frame and OBJ markers |
| Status | x=[120,240), y=[0,64) | (232,8) | Clock, money, health, wanted level, weapon OBJ |
| Messages | Remaining BG1 | (120,152) | District name and messages outside the first two regions |

Grouping was checked against seven captured game frames: on foot, driving,
attacking and returning from pause. These are group boundaries, not separate
digit/icon boundaries. Independent health and weapon sizes within Status
have not been added.

In widescreen, the radar and its OBJ markers share a left anchor, while
status and weapon use a right anchor. Messages are centered in the wide frame.

## Implementation

hud_layout.rs defines sizes, anchors and inverse coordinate tables.
object_projection.rs uses them when composing BG1 and screen-space OBJs.
The world is rendered first: shrinking the HUD reveals the world beneath it
rather than moving pixels from a finished frame or shrinking cars that happen
to lie under the HUD. Only priority-0 screen-space OBJs inside Radar/Status
are scaled. World objects retain their coordinates, scale and interpolation.

The normal USA HUD is identified by a 32×32 screen-space weapon OBJ at (200,8).
Without it, the transformation is disabled because BG1 may contain a dialog
or panel. Original rendering and unsupported Enhanced scenes use the original
HUD. At 100%, BG1 follows the original read path without extra lookup tables.
Guest memory and original buffers are not modified.
