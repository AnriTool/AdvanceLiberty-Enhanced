# Host menu audio

**English** | [Русский](../ru/MENU_AUDIO.md)

Enhanced Options and additional rows in the normal menu use original USA sound
0x105, called by 08005c14 for movement, confirmation and back actions.

08034db8 obtains PCM through table 08ca74dc[(id - 201)]. Sound 0x105 has its
header at 080eaa54: signed 8-bit mono, 2404 samples, 9556 Hz; field +4 stores
frequency << 10. menu_audio::load_click validates the BGTE ROM, bounds and
format, then converts PCM to stereo i16 at 65536 Hz. The sample is read from
the user's ROM; no binary game assets are embedded in the source tree.

options::Menu reports actions through take_click. Navigation, submenus, value
changes, binding capture/clearing, cancel and save each produce one signal.
Mouse motion, button releases, unsupported keys and repeated Enter holds do
not produce a click. Mouse and gamepad navigation use the same path.
game_menu::Frontend adds signals for host-owned rows without duplicating
the sound of ordinary original-cursor movement.

A successful camera switch through its gameplay key, V by default, uses the
same click and SFX Volume, including Off. The sound is emitted after the new
camera mode is applied. Holding the key or pressing it in menus/pause does
not trigger it. Remapping the key preserves this behavior.

The SDL callback mixes the game PCM queue with a separate click voice.
Clearing game audio during pause does not clear that voice. When play resumes,
the click mixes with game audio instead of delaying game PCM until the sample
finishes. Rapid actions restart the click without accumulating a queue. The
sum saturates to i16; volume follows SFX Volume at 02010c61, with Off producing
silence. Guest CPU, memory, audio buffers, recordings and native profiles are
not modified.

The scale matches the original: High = 100%, Medium = 50%, Low = 25%, Off = 0%.
080328cc uses the ROM shifts at 08ca665c: [32, 2, 1, 0]. Converting signed PCM
to host i16 uses a multiplier of 32, matching the game's half-volume FIFO.
Volume changes also affect a click already playing; Off stops it immediately.
