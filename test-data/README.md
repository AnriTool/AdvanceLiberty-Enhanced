# Local regression-test fixtures

**English** | [Русский](README.ru.md)

This directory holds private recordings needed for checks against a real ROM.
Data is excluded from Git and user ZIPs. Ordinary unit tests do not depend on it.

`menu-session/initial.sav` and `menu-session/input.gba-input` are the minimal
recorded-boot pair for native Camera/Save and Quit checks. The route must reach
the pause menu at frame 850. Full dumps, images and logs are unnecessary for
these two tests.

Set `ADVANCE_LIBERTY_MENU_SESSION` to another directory containing that pair.
The tests also require your own USA ROM at `roms/usa.gba` and built DLLs from
`native/usa/manifest.json`:

```powershell
cargo test --release --offline --locked --package advance-liberty --bin AdvanceLiberty real_native_camera_roundtrip_resume_and_quit -- --ignored
cargo test --release --offline --locked --package advance-liberty --bin AdvanceLiberty real_native_save_and_exit_writes_and_reloads_progress -- --ignored
```

Recordings used by CollectNative remain in `data/sessions/`. Coverage collection
does not read this fixture directory.
