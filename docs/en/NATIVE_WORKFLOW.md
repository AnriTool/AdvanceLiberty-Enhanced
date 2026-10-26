# Updating native coverage after play

**English** | [Русский](../ru/NATIVE_WORKFLOW.md)

After closing the game, run this from the project directory:

```powershell
.\UpdateNative.cmd
```

It collects new instructions from completed `data/sessions/` recordings,
updates `native/usa/coverage/sessions.csv`, rebuilds the `sessions` profile
with MinGW x64 GCC, then installs its DLL/C and updates the manifest. It uses
the same Rust, Cargo and MinGW tools as a normal project build; Python is not
required. The other three profiles are not rebuilt. Compilation is skipped
if there are no new variants and the count matches the installed profile.

The normal mode does not run long replays. It checks the USA ROM SHA-256,
instruction format/alignment, audited ROM bytes, generation without unsupported
instructions being skipped, and installed-file hashes. An incompatible recording
is excluded as a whole with a reason in the report. Incomplete recordings and
`fallbacks.partial.csv` are not used. Identical instructions are deduplicated;
different RAM variants at the same address are retained. Successful compilation
alone does not establish equivalent execution.

To validate the behavior of new variants:

```powershell
.\UpdateNative.cmd -Verify
```

Only recordings that contributed new variants are replayed before installation.
They must have zero interpretations and match every original video/audio hash.
Replays use copies of `initial.sav`, leaving gameplay saves unchanged. Generation
or validation failures leave the active DLLs intact. A full historical regression
is appropriate after generator, CPU, interrupt or timing changes, rather than
every CSV update.

To select one recording:

```powershell
.\UpdateNative.cmd -Session session-0123456789abcdef0123456789abcdef -Verify
```

For several recordings, call the PowerShell script directly with
`-Session @('session-a', 'session-b')`. An explicitly selected incompatible
recording fails collection instead of being automatically excluded. Very old
recordings may stop matching after font or application changes; investigate
the difference rather than assuming the added native code is at fault.

## Separate commands

```powershell
.\CollectNative.cmd              # Collect CSV only, without compiling
.\CollectNative.cmd -DryRun      # Preview without changing CSV
.\RebuildNative.cmd              # Rebuild and install sessions from CSV
```

`CollectNative.cmd` does not change the active DLL or manifest. Until rebuilding,
CSV may contain more variants than the installed library. `UpdateNative.cmd`
detects this pending update by row count. `-Force` forces a rebuild;
`-SkipBuild` uses existing Rust tools, and `-Gcc PATH` selects MinGW GCC.
If coverage was collected separately, `-Verify` checks one compatible recording.
Use `-Session` or `-VerifySession` below to choose the recording precisely.

Advanced build options are available through the existing script:

```powershell
.\tools\rebuild-native.ps1 -NativeProfile sessions -Install -VerifySession session-0123456789abcdef0123456789abcdef
.\tools\rebuild-native.ps1 -NativeProfile all -Install
```

Close the game before installing DLLs. Previous DLL/C files and the manifest
are backed up in `artifacts/native-build-.../before-install/`; an installation
failure restores modified files. Collection backs up the previous CSV and a
JSON report in `artifacts/native-collect-.../`. A failed compilation leaves
collected CSV in place for fixes or another build. To restore coverage, copy
`before-sessions.csv` from the report back to `native/usa/coverage/sessions.csv`.

Commit changed CSV files and `native/usa/manifest.json`; DLLs and C remain
local build outputs.

`test.ps1` includes collection and installation tests covering incompatible
audits, distinct RAM variants and rollback after partial installation failure.
They can also be run separately:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/tests/native-coverage.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tools/tests/native-install.ps1
```

## Why validation matters

Adding coverage and changing the execution model are separate operations.
Generator, CPU, interrupt and timing changes require regression checks: even
a few extra instructions beyond a frame boundary can alter video, sound and
events. Compilation therefore does not replace replay comparison. Audited
ROM bytes that do not match indicate an incompatible recording, which must
not enter accumulated coverage.
