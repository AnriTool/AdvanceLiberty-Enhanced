# Game menus and PC preferences

**English** | [Русский](../ru/MENUS.md)

## Presentation and actions

`game_menu::Frontend` replaces supported game lists with host presentation
using the font and backgrounds from the user's ROM. Pause, Options and Camera
entries remain connected to original game actions. Cursor selection and
confirmation are sent as recordable GBA button pulses. The adapter waits for
the required index; it neither assumes cyclic navigation nor confirms an
unverified transition.

Brief, Stats, Credits, startup screens, shops, safehouses and save-slot management
may use original presentation. This does not replace every ROM page. Menus keep
3:2; returning to the world restores the selected 16:9 view and HUD sizes.

## Navigation

Esc/Pause opens game pause. Arrows select, Enter confirms and Esc goes back;
mouse and gamepad navigation are supported. On original pages, Enter/Esc mean
A/B without also sending Start. P separately pauses the runtime; F6 changes
the renderer, F8 the control profile and F11 fullscreen.

Held menu buttons are not passed to gameplay until released. Focus loss blocks
input. Back can cancel a pending transition.

## Enhanced Options

PC preferences open from pause and Options. The editor works on copies of
Controls/Preferences: Save and return applies and saves edits, while Discard
and return cancels them. Guest CPU and game audio stop inside this editor.
Menu clicks play independently of pause and follow original SFX Volume,
including Off.

Options include graphics/aspect ratio, fullscreen, HUD scaling, languages,
Enhanced/Classic controls and keyboard/mouse rebinding. New profiles default
to 16:9, fullscreen and mouse aiming; existing saved choices take precedence.

## Backgrounds and localization

The background follows the current island: blue for Portland, green for
Staunton, red for Shoreside. These are three separate ROM images, each with
its own tiles/map/palette. The current island is read at `020303d0`; the
temporary unload value `0x7e7e` does not replace the last valid background.
An open editor refreshes its background and invalidates its cache on region changes.

Changing language rebuilds menu fonts and labels from the pack and in-memory
ROM. Additional PC labels live in the language JSON's `menu` section. ROM
graphics are not included in the distribution.

## Saving and exiting

Quit to Menu and Quit to Desktop have different meanings. Save and Quit or
Save game and exit first runs the original save procedure. After success,
the runtime atomically commits EEPROM to disk with a backup; only then can
it exit or return. A game error, timeout or disk error cancels exit and shows
a message.

Saving game progress and saving PC preferences through Save and return are
separate actions. Unsaved preference edits do not become a game save.

[Control settings](CONTROLS.md) · [Localization](LOCALIZATION.md) ·
[Menu audio](MENU_AUDIO.md) · [Native-test fixtures](../../test-data/README.md)
