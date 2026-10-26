# Widescreen Enhanced rendering

**English** | [Русский](../ru/WIDESCREEN.md)

First launch defaults to 16:9. Select the format in
Enhanced Options → Graphics → Aspect ratio and save with Save and return.
Original 3:2 remains available. World aspect ratio is independent of fullscreen.

## Frame geometry

`viewport.rs` defines an 854×480 Enhanced frame: the original 720×480 region
stays centered with 67 pixels on either side. World scale and vertical field
of view are preserved. Rounding to an even width differs from exact 16:9 by
less than 0.1%. The HUD anchors to the new edges without stretching. SDL
reuses 240×160, 720×480 and 854×480 textures.

The extension applies to building projection, ground, world sprites and
bridge/window masks. Menus, story panels, Original rendering and Camera preview
fallbacks may retain original proportions. Arbitrary 21:9/32:9 is unsupported.

## Ground map

VRAM holds a resident cache of visible tiles, not a complete map for sampling
the extended sides. The full USA ROM map is accessed through validated current
EWRAM pointers:

- `0201ce60` — block map;
- `0201ce64` — block definitions, 5×5 tiles and 50 bytes per block;
- `0201ce68` — indexed 8×8 tile atlas;
- `0201ce74` — log2 map width: 7 or 8.

| Island | ROM header | World size | Cell grid |
| --- | --- | --- | --- |
| Portland | `08cabe98` | 16384×16384 | 256×256 |
| Staunton | `08cabed4` | 8192×16384 | 128×256 |
| Shoreside | `08cabf10` | 8192×16384 | 128×256 |

The header is selected by matching map/blocks/atlas, not the mission number,
which may change before scene resources. Width and height are validated
independently: the last two maps are not square. Cell encoding is shared.
Sampling uses the current snapshot's palette. ROM data is shared between
snapshots through Arc; original game memory is unchanged.

## Materials and sprites beyond the original screen

The original pass may submit geometry without a clipped face's texture.
Capture retains validated atlas/UV mappings and rejects ambiguous matches.
Keys account for ROM atlases, orientation, special flag and original face.
The cache is limited to 128 materials and 512 faces. For validated special
faces with constant depth and axis-aligned UV bases, material can be recovered
directly from the main ROM atlas. Visible captured materials are not replaced.

Additional collection of existing world sprites and side building models
extends visibility; it does not create objects absent from game logic.
Original activation distances and object-pool limits remain. Composition
accounts for bridge masks, sprite order and transparency.

## Camera, freshness and diagnostics

Enhanced uses validated scene snapshots and smooths their presentation.
Material availability is checked over the full wide region. Unsupported or
stale scenes can temporarily use Original output. Fallback reasons, recovery
and settings are recorded in the session's `camera-events.csv`; totals are
written to `camera-summary.txt`.

`Aspect` is stored in `enhanced.cfg`. Runtime flag `--widescreen` enables
wide composition during diagnostic replays without a window.

[HUD scaling](HUD_LAYOUT.md) · [Mouse aiming](MOUSE_AIM.md) · [Menus](MENUS.md)
