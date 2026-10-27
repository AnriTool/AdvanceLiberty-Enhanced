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


## Roof clipping and frame-time diagnostics

Roof orientation uses the original model basis when clipping or integer UV
rounding collapses a diagonal axis. Both screen coordinates and the rotated,
clamped UV coordinates are checked. Native descriptor validation remains intact;
this only corrects the host presentation and its material cache.

Immutable ROM atlases and their orientation variants are shared within each
capture. RAM textures are read again on every native submission. The indexed
building layer is reused only for the same captured scene, camera displacement,
viewport and texture-mapping mode. Palette effects, ground, sprites and HUD still
update normally. The layer cache holds one image.

Opt-in diagnostic sessions include `frame-profile.csv`. `native_ns` includes
native execution and capture hooks, `render_ns` measures Enhanced presentation,
and `audio_ns` measures host DSP. `total_ns` includes presentation/pacing and
recording overhead. GBA FIFO counters distinguish cartridge audio starvation
from host output: `queued_audio_bytes` and `audio_underruns` describe the playback
queue. Host underruns count callbacks with insufficient game samples; cleared
or paused audio is excluded. FPS measures average presented frames, so brief
budget overruns can still occur near 60 FPS.

`present_interval_ns` records time between actual frame-submission calls;
`present_lateness_ns` records how late submission starts relative to its deadline.
The first frame after pause or replay transitions has interval zero. Headless
runs report zeros for both fields. Unlike `total_ns`, these values expose visible
cadence when frame preparation time varies. Frames are prepared and game audio
is queued before waiting for the submission deadline. A long stall rebases the
schedule to avoid a burst of catch-up frames; the last millisecond uses a short
spin wait after the coarse sleep to reduce scheduler overshoot.

The low-level runtime also accepts `--frame-profile NEW_CSV` and
`--roof-trace NEW_CSV START_FRAME END_FRAME`. Roof traces contain native,
model-derived and final orientations plus clipped endpoints. Normal launches
without diagnostic recording create neither report.

`--capture-profile NEW_CSV` additionally measures observation time and call count
per GBA frame, cumulative validation/graphics/geometry/composition times, and the
current scene's flat, 64×64 and 128×128 textured face counts. Use it with
`--camera-preview` for offline geometry investigations; cumulative timing columns
must be differenced between rows. It is off in ordinary session recording.

## Composition and audio buffering

At output sizes of at least 720 × 480 on a multicore system, Enhanced composition
uses two independent row regions: one worker thread and the calling thread.
Terrain, sprites and pixel effects keep the same sampling and drawing order.
Smaller outputs and single-core systems use one region. The prepared HUD layer
is cached until its source pixels, layout or viewport change.
The building layer also renders independent row regions on multicore systems,
preserving face order and identical affine/perspective UV calculations per region.

Read-only geometry and sprite passes fork the machine state. Cartridge bytes and
the decode memo table share storage until a writer changes them. RAM, devices and
CPU state remain independent. Runtime patches and language changes detach shared
cartridge data before writing, preserving existing snapshots. This avoids large
ROM/table copies at scene boundaries without changing guest clocks or rendering.

On multicore systems, the extra wide building pass runs on one persistent worker with its own
machine snapshot and an immutable native function table. The matching scene
upload waits for that result before validation and material merging. Its queue
holds at most one waiting scene; a busy or unavailable worker uses the synchronous
pass. Cancelled results are discarded; shutdown joins all outstanding work before
native libraries can unload. Single-core systems also use the synchronous pass. Background work
does not change the guest's geometry, simulation cadence or native coverage.

Host audio waits for one callback of samples after clearing its queue. If output
runs dry, it rebuilds a two-callback queue before resuming, retaining a small
reserve against the next late frame. This adds buffering, without changing sample
order or playback speed. Host menu clicks continue while game audio is buffering.
Rebuffering cannot compensate for sustained frame times above the game budget;
use the frame profile to distinguish rendering load from audio processing cost.
