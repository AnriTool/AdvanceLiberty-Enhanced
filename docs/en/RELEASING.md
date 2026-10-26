# CI builds and GitHub Releases

**English** | [Русский](../ru/RELEASING.md)

`.github/workflows/build.yml` builds and tests Windows x64 on Windows 2022 and
Linux x86_64 on Ubuntu 22.04. Every branch push, pull request and manual workflow
run produces downloadable package artifacts, retained for 14 days. These runs
do not publish a GitHub Release.

Pushing a `v*` tag runs the same matrix and publishes a Release only after both
platforms succeed. Its tag must be exactly `v` plus the version in `Cargo.toml`.
Versions with a suffix, such as `0.1.0-preview.2`, are published as prereleases
and do not replace Latest. Stable versions are marked Latest.

## Creating a release

1. Set the intended version in `Cargo.toml`; update `Cargo.lock` if Cargo changes it.
2. Commit and push the source, including workflow, scripts and native coverage.
3. Create and push the matching tag. For version `0.1.0-preview.2`:

```sh
git tag v0.1.0-preview.2
git push origin v0.1.0-preview.2
```

Use a new version/tag for another published build. Do not move an existing tag.
The workflow creates the Release automatically; it does not require creating an
empty release in the GitHub UI beforehand. To test without publishing, run
**Actions → Build and release → Run workflow** on a branch.

## Published assets

- `AdvanceLiberty-Enhanced-VERSION-win64.zip`
- `AdvanceLiberty-Enhanced-VERSION-win64.zip.sha256`
- `AdvanceLiberty-Enhanced-VERSION-linux-x86_64.tar.gz`
- `AdvanceLiberty-Enhanced-VERSION-linux-x86_64.tar.gz.sha256`

The archives contain the application, four platform-native profiles, manifest,
notices and licenses. Users extract the whole folder, provide their own legal
unmodified USA ROM, and launch the application directly.

## ROM-free CI

The AOT generator's `--coverage-only` mode decodes instruction snapshots already
present in versioned CSV files. It needs no ROM file, reconstructed ROM, game
assets or ROM secrets. It checks ROM snapshot overlaps for contradictions and
retains separate guarded RAM variants. The generated C matches normal
ROM-validated generation for the same coverage.

Windows CI calls `scripts/windows/rebuild-native.ps1 -CoverageOnly`; Linux calls
`bash build.sh --native --coverage-only`. Generated DLL/SO/C files remain ignored.
Packaging excludes ROMs, BIOS, extracted assets, saves, recordings and developer
scripts. Gameplay replays still require a local user-supplied ROM and are separate
from the ROM-free synthetic and unit checks performed in public CI.

## Publication behavior

Only the publication job gets `contents: write`; build and PR jobs are read-only.
The repository-provided `GITHUB_TOKEN` is sufficient: no personal access token or
ROM secret is needed. The repository/organization must allow GitHub Actions and
permit the workflow's requested release-write access.

`scripts/ci/release.py` requires both archives and verifies their SHA-256 files.
It uploads assets to a draft and publishes after successful attachment. A failed
upload leaves a draft that a rerun can resume. It never replaces files in a
published Release; a rerun for the same commit leaves a complete public release
unchanged. A mismatching commit or incomplete public release fails explicitly.

[GitHub CLI release behavior](https://cli.github.com/manual/gh_release_create)
and [workflow permissions](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax#permissions).
