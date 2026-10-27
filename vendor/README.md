# Local dependencies

**English** | [Русский](README.ru.md)

`armv4t/`, `gba-core/` and `recomp-core/` come from a local checkout of
gba-recomp: https://github.com/JRickey/gba-recomp,
base revision `de4edf59b872d887046d6a3b005e2df551b6d44c`.

The working state was copied with local diagnostic, capture, instruction-level
AOT and GPU API changes. Files were compared byte-for-byte during migration.
These are self-contained dependencies of the current Cargo workspace; the
external checkout, its Git repository and its target directory are not needed
for building.

Original MIT/Apache-2.0 license texts and THIRD-PARTY.md are retained.

`gba-core` also contains a local observation-only PCM sidecar. It follows
DMA/FIFO timing for optional host effects without changing guest audio or memory.
The USA driver integration lives in the application's `polyphonic_audio` module.
