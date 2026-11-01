# Enhanced Output

**English** | [Русский](../ru/ENHANCED_AUDIO.md)

Normal `Play.bat` launches enable Enhanced Output by default.
Use `Play.bat -OriginalAudio` to compare it with the original hardware mix.
The explicit `Play.bat -EnhancedAudio` flag is also supported. This remains
a launch option and is not yet stored in PC preferences.

The DSP algorithms come from JRickey/gba-recomp, base revision
`de4edf59b872d887046d6a3b005e2df551b6d44c`, `crates/recomp/src/main.rs`:
a 24-tap Hann-windowed sinc filter, separate Direct Sound FIFO A/B
interpolators, PSG filtering, a DC blocker around 10 Hz and soft peak clipping.
The adapted code is in `src/enhanced_audio.rs`; MIT/Apache-2.0 license texts
are retained in `vendor/`. This mode does not enable high-level MP2K/GAX/R mixers.

Processing runs on the output side on an exact 65536 Hz grid, without adjusting
pitch according to queue fill. Unlike upstream, output gain remains at unity
for comparison with our normal volume. SFX/Music levels and channel routing
come from the original game. Host menu and camera clicks use the existing
separate voice and follow SFX Volume.

The game's mixer still runs. Its original `audio_buf` is preserved: video/audio
hashes and session recordings validate the original canonical stream.
`info.txt` records `audio_output=EnhancedOutput`. Replaying without the flag
uses normal audio; `--enhanced-audio` enables processed audio. With the flag,
diagnostic `--audio-out FILE.pcm` exports processed stereo s16le at 65536 Hz;
without it, the command exports original PCM.

The output path follows upstream's approach: routing and FIFO volume fractions
are sampled at the end of each frame. Rapid register changes within a frame
may differ from the hardware path. Compare music, gunshots, engines and sirens
at identical SFX/Music settings when judging the result.

## Simultaneous effects

Normal EXE, `Play.bat` and `play.sh` launches enable simultaneous queued PCM
effects by default. Disable them with `--no-polyphonic-audio` (EXE/Linux) or
`Play.bat -NoPolyphonicAudio`, retaining Enhanced Output processing. The explicit
`--polyphonic-audio` / `-PolyphonicAudio` options remain accepted. This requires
Enhanced Audio; Original Audio takes precedence in the launchers. It is
independent of render settings. Low-level headless diagnostic exports remain
opt-in, and gameplay replay snapshots retain their recorded audio mode.

The USA driver maintains priority-sorted voice lists and renders the first voice
in each group. An effect can suppress another voice even while that voice remains
active and its position continues advancing. The sidecar mixes the unpaused raw
PCM voices behind the selected heads in effect groups 0 and 2. Group 2 voices
with independent pitch are mapped to the selected voice's timer grid. The original audible
head is excluded. Volume and voice allocation/lifetime still come from the game;
this does not expand the guest pool or revive discarded sounds.

Some player weapon sounds are rejected by a higher-level game check while a
competing vehicle sound is active, before a PCM voice is allocated. Enhanced
simultaneous effects also observes those rejected player requests and supplies
their raw PCM through the host sidecar. It preserves the guest check and return
value, follows the game's SFX attenuation table, and avoids duplicating samples
already audible through the original heads. Repeated requests retrigger one
host voice per sample; NPCs are not added at the player's loudness. These bounded
voices are saved/restored with gameplay replays. Original Audio is unchanged.

Only one copy of each PCM sample is audible through the sidecar. Samples already
selected by any original channel are excluded, and repeated queued copies are
deduplicated across both effect groups. This avoids stacking successive skid or
impact sounds. Different suppressed effects can still overlap, so this mode
intentionally sounds different from the normal mix.

Extra samples follow the driver's buffer prefix copies and the actual sound DMA
and FIFO events, including dropped writes, resets and held DAC values. They join
the existing sinc filter, routing, DC blocker and soft clip. Guest RAM, original
PCM, CPU execution and canonical session hashes are unchanged. The extra FIFO
and filter state is included in gameplay replay snapshots and restored with live
audio on return. Diagnostic sessions identify this mode with `polyphonic_audio=1`;
pass `--enhanced-audio --polyphonic-audio` explicitly when exporting their audio.

For driver research the native `multi` worker accepts `--sound-probe NEW_DIRECTORY`
(incompatible with camera write tracing). It writes `writes.csv` for the driver
region and `driver.bin`: repeated records of a little-endian u64 frame number
followed by bytes from EWRAM `0x02027330..0x02027f00`. These are observations only;
ordinary launches do not create these files.

The probe also writes `requests.csv` for one-shot/loop allocation requests and
the high-level weapon rejection check, including caller, actor and returned
read value. A rejected high-level request may have no allocation in `writes.csv`.
When the driver relocates its IWRAM buffers, obsolete shadow addresses are
discarded; queued FIFO values remain intact and snapshots stay bounded.
