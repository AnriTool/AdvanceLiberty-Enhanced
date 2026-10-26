AdvanceLiberty: Enhanced — Linux x86_64

No ROM, BIOS or extracted original game assets are included or downloaded.
Provide your own legally obtained, unmodified USA cartridge dump.

1. Extract the entire archive, keeping the directory structure.
2. Put your USA .gba dump in roms/ (any filename).
3. Run ./AdvanceLiberty from this folder, or launch it in your file manager.
   If execution permission was lost: chmod +x AdvanceLiberty

Requires a Linux x86_64 desktop (X11 or Wayland), glibc and normal system
sound/graphics libraries. SDL2 is linked into the application. No Rust,
Python, compiler, Windows compatibility layer or shell script is required
to play the packaged release. Linux packages should be built on the oldest
glibc distribution they intend to support; they are not universal binaries.

Esc > Enhanced Options opens settings. Saves, settings and recordings are
stored in data/ beside the executable; replays are stored in replays/.
Keep the extracted folder writable.
The existing Enhanced defaults also apply to this build.

Original project code: MIT (LICENSE). Third-party notices: NOTICE, licenses/.
This does not grant rights to the original game. See the repository's
Linux guide for source builds and the current testing status.

Replays: F2 start/stop recording; F3 play latest; F4 replay browser.
Esc/Enter stops playback and restores live gameplay. Files: replays/.
Diagnostic sessions are disabled by default. Enable with --record-session.
