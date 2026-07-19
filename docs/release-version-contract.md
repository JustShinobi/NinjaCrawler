# Release version contract

NinjaCrawler keeps a single public app version across several manifests. Only
**source** files are bumped by Release Please. Generated lockfiles stay in sync
through explicit tools, never via embedded Release Please markers.

## Sources of truth (app)

| File | Updated by |
|------|------------|
| `package.json` | Release Please (`release-type: node`) |
| `package-lock.json` | Release Please (node lock) |
| `src-tauri/tauri.conf.json` | Release Please (`extra-files` jsonpath) |
| `src-tauri/Cargo.toml` | Release Please (`extra-files`; keeps `# x-release-please-version`) |
| `.release-please-manifest.json` | Release Please |

Companion versions live on a separate track (`NinjaCrawler.Companion/`).

## Derived: `src-tauri/Cargo.lock`

`Cargo.lock` is owned by Cargo. Comments and hand-maintained annotations are
stripped whenever the lockfile is regenerated (`cargo add`, `cargo update`,
merge conflict resolution, etc.).

Therefore:

- Release Please **must not** list `src-tauri/Cargo.lock` in `extra-files`.
- The lockfile **must not** carry `# x-release-please-version`.
- After `Cargo.toml` is bumped on the app release PR, the root package entry
  (`name = "ninjacrawler"`) is rewritten by:

```powershell
pwsh -File Tools/Sync-NinjaCrawlerCargoLockVersion.ps1
```

On GitHub, `.github/workflows/sync-release-cargo-lock.yml` runs that script on
`release-please--branches--main--components--ninjacrawler` and commits when the
lock lags `Cargo.toml`.

## Validation

| Check | Role |
|-------|------|
| `Tools/Test-NinjaCrawlerVersion.ps1` | All sources + lock package version equal |
| `Tools/Test-ReleaseAutomation.ps1` | RP config omits lock; sync assets exist; lock has no marker |
| `Tools/Sync-NinjaCrawlerCargoLockVersion.ps1 -Check` | Lock matches `Cargo.toml` (no write) |
| `release.yml` prepare step | Same version contract before publishing artifacts |

## Operator notes

- Feature work may regenerate `Cargo.lock` freely; no marker to restore.
- If CI reports a lock/version mismatch, run the sync script and commit
  `src-tauri/Cargo.lock` only when a version bump is intentional (normally the
  release PR workflow does this).
- Do not re-add `# x-release-please-version` to `Cargo.lock`.
