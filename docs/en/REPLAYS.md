# Gameplay replays

**English** | [Русский](../ru/REPLAYS.md)

- **F2** starts recording during gameplay; **REC** appears in the corner.
  Press F2 again to stop and save. Closing the game also saves the current clip.
- **F3** plays the newest saved replay. If recording, it saves that clip first.
- **F4** opens the replay browser and pauses the live game. Up/Down or PageUp/
  PageDown selects a clip; Enter plays it; Esc/F4 closes the list. Times are UTC.
- During playback, **Esc**, **Enter** or **F3** stops viewing. Reaching the end
  also stops automatically. Live gameplay resumes at exactly the point where
  viewing started. Gamepad Start/Guide also exits playback.

Replays live in `replays/replay-*/`, independently of saves and diagnostic
sessions. No screenshot sequence or external video encoder is required. The
camera, mouse aim and recorded language changes are reproduced through
the normal game runtime. Gameplay input and host menus are blocked during
playback; **F6 switches Enhanced/Original rendering** and F11 toggles fullscreen.
Viewing starts with the current renderer, regardless of how the clip was recorded.
Renderer changes during viewing do not modify the replay or the suspended live settings. F8 no longer switches control profiles;
select Enhanced/Classic in Enhanced Options.

A clip contains an initial CPU/device/memory snapshot, runtime ROM patches and
translation data, host audio/drop-policy state, and resolved controls for each
frame. It does not embed the original ROM: viewing requires the same compatible
ROM and runtime version. Enhanced presentation history warms up from the clip's
starting frame. Frame checksums stop a diverging replay and restore live play.
One clip is limited to one hour and saves automatically at the limit. Finished
clips are written atomically; unfinished staging folders are omitted from the list.

The current live machine, audio state and camera history stay in memory while
viewing. Replay progress is never written to the user's cartridge save. On
return, the held exit key must be released before gameplay input resumes.

## Diagnostic sessions

Automatic session recording is **off**. For native coverage collection use:

```text
AdvanceLiberty.exe --record-session
Play.bat -RecordSession
bash play.sh --record-session
```

The packaged Linux executable accepts `./AdvanceLiberty --record-session`.
Diagnostic recordings still go to `data/sessions/session-*`; the existing
CollectNative/UpdateNative and Replay tools continue to use that format.
Diagnostic session recording pauses while viewing a gameplay replay and resumes
on return; historical replay frames are excluded. Launch logs in `data/logs/`
remain available without enabling a session. The legacy `--no-record` option
is accepted for compatibility.
