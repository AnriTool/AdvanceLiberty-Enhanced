# Enhanced Output

**English** | [Русский](../ru/ENHANCED_AUDIO.md)

Normal `Play.cmd` launches enable Enhanced Output by default.
Use `Play.cmd -OriginalAudio` to compare it with the original hardware mix.
The explicit `Play.cmd -EnhancedAudio` flag is also supported. This remains
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
