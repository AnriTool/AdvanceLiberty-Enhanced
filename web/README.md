# Browser build

The browser runs the shared GBA core as a WebAssembly interpreter. It includes
the Enhanced renderer with 16:9 and 3:2 modes, perspective-correct textures,
HUD scaling, Enhanced Audio with simultaneous effects, the 30-second drop
policy, and contextual controls with mouse aiming. Original graphics, audio
and controls are available from the browser settings. The browser uses its
own pause panel for these settings. Desktop AOT profiles and gameplay
replays are outside the browser build.

```sh
rustup target add wasm32-unknown-unknown
python web/build.py
python -m http.server 8000 --bind 127.0.0.1 --directory dist/web
```

Open http://127.0.0.1:8000 and select your own unmodified USA ROM. Optionally
select one or more existing JSON language packs alongside it. The Language
selector applies them during gameplay, including packs with custom glyphs.
The page reads both kinds of file locally into memory. Select the ROM and
translation files again after reloading the page.

Browser preferences, the selected language filename and the cartridge EEPROM
save are stored in localStorage for that origin. Save from the game's menu
and wait for the confirmation before closing the tab. Reopen using the same
browser profile and URL to resume from that save. Serve the contents of
`dist/web` with any static HTTP server.

GitHub Actions builds this package without a ROM and uploads the static
files as the `web-build` workflow artifact.

Press Escape or use Pause / Settings to suspend the game and open the
settings. The game's Start action still opens the original menu. In
Keyboard and mouse bindings, select a profile and an action, then press a
key or click that binding with the desired mouse button. A new binding
replaces the action's previous keys. Escape cancels a pending change.
Reset this profile restores its default bindings. Changes are saved locally.

Standard-mapped gamepads use the desktop GBA layout. A/B, Back/Start, and
the shoulders map to their GBA equivalents. Use the D-pad or left stick to
move. Keyboard and controller inputs work together. When the browser
exposes the Guide button, it opens or closes the browser pause panel.
