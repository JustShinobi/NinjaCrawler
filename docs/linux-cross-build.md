# Linux cross-build contract

## Current Linux path

NinjaCrawler targets Windows x64 while trusted CI and release builds run on the
ephemeral Proxmox JIT Linux runner. External pull requests retain a
GitHub-hosted `ubuntu-latest` fallback. `Tools/Build-NinjaCrawler.ps1` keeps the
Visual Studio/MSVC path for local Windows builds and selects
`cargo-xwin --target x86_64-pc-windows-msvc` on Linux.

The fixed Linux toolchain is:

- Rust stable and `cargo-xwin` 0.22.0;
- Node.js 22;
- LLVM/Clang/LLD 18.1.3 and NSIS 3.09 from Ubuntu 24.04;
- `libayatana-appindicator3-dev`, required by Tauri when `tray-icon` is enabled;
- PowerShell Core for repository scripts.

The target directory defaults to
`src-tauri/target/x86_64-pc-windows-msvc/release`. When `CARGO_TARGET_DIR` is
set, the build wrapper resolves the executable and bundles below that directory
instead. A full build produces the raw PE and an NSIS bundle. MSI is
intentionally unsupported because WiX remains a Windows-only path.

## Release contract

An app release contains exactly these public assets:

- `NinjaCrawler-<version>-windows-x64-portable.exe`;
- `NinjaCrawler-<version>-windows-x64-setup.exe`;
- `CHANGELOG.md`;
- `SHA256SUMS.txt`, covering the other three files.

The portable artifact is the renamed `ninjacrawler.exe`, not a ZIP. It contains
neither a README nor connector executables. Portable means no installer; the app
still stores state in `%LOCALAPPDATA%\NinjaCrawler` and requires internet during
first-run connector preparation.

## Connector preparation

The connector catalog pins the required versions in `connectors/manifest.json`.
On Windows, the app obtains the pinned GitHub release asset, requires the
Release Assets API `digest` field to contain a SHA-256 value, verifies the
download, probes `--version`, and atomically activates it below
`%LOCALAPPDATA%\NinjaCrawler\connectors`.

Missing digests, hash mismatches, invalid archives, failed probes, and interrupted
downloads fail closed. A failed staging file never becomes active. Existing
managed installs are reused. A custom executable counts as ready only after its
explicit path passes the version probe; connector discovery through `PATH` is
not supported.

## Workflow security boundary

Pull requests opened by repository owners, organization members, or explicit
collaborators execute the cross-build on the ephemeral Proxmox JIT runner. PRs
from every other author association, including public forks, remain on the
hosted runner. Both paths use read-only repository permissions,
`persist-credentials: false`, and no build secrets. Trusted pushes to `main` and
manual CI dispatches also use the JIT runner; the job is skipped for manual
dispatches against any other ref. The release workflow separates concerns:

1. `build` checks out the trusted release ref, compiles and uploads an Actions
   artifact with read-only permissions;
2. `publish` downloads that artifact and is the only job granted
   `contents: write`.

The official build job and its persistent compilation cache run on the JIT
runner. Publication remains hosted regardless of the build runner. Because the
LXC enforces `no_new_privs`, jobs cannot use `sudo` to mutate the image. The
golden must include `pkg-config` and `libayatana-appindicator3-dev`; workflows
verify both and fail before compilation with an explicit image-contract error.

## Release Please publication gate

A draft GitHub Release does not create its Git tag. Release Please therefore
must not generate the next release PR while a manifest version exists only as
an unpublished draft. The workflow applies two gates:

1. immediately after Release Please creates a release, all pre-tag release PRs
   are closed before the artifact build is dispatched;
2. every later `main` run detects a draft whose tag is still absent, closes any
   release PR that reappeared, and pauses that package until publication or an
   explicit recovery completes.

After successful publication, the hosted publish job closes all shared-manifest
release PRs and dispatches Release Please again. At that point the tag exists,
so the regenerated PR uses the correct changelog boundary. A failed build leaves
the draft available for recovery but cannot leave an inflated next-release PR
open.

## End-to-end release validation

`Windows release E2E validation` is a manual workflow and uses the same split as
production. The publication job is additionally restricted to `refs/heads/main`
so a workflow modified in another branch cannot receive `contents: write`:

1. a read-only JIT job runs lint/tests, builds portable plus NSIS, packages the
   four exact release assets, validates PE x64 and SHA-256, and uploads an
   Actions artifact;
2. a hosted job with temporary `contents: write` downloads that artifact,
   verifies it, creates an isolated published prerelease, downloads every asset
   again, compares the bytes and checksums, then deletes the prerelease and tag
   in an `always()` cleanup step.

The test tag is `release-e2e-v<app-version>-<run-id>-<attempt>`. It exposes the
version actually compiled while remaining outside the production `vX.Y.Z`
namespace, and it never updates the README, manifest, changelog, back-sync
branch, or latest-release marker. Before compilation, the workflow requires
`package.json`, the Tauri config, `Cargo.toml`, and the application entry in
`Cargo.lock` to carry the same version. Promotion automation also refuses to
move an older `develop` version over a newer `main` version; the draft release
state must be synced back into `develop` first.

Linux cross-compilation can emit diagnostics owned by upstream tools. The build
passes `/ignore:4099` only to the Windows cross-linker because the MSVC runtime
archives reference Microsoft-internal PDB files that are not distributed by
`cargo-xwin`; those missing debug symbols do not affect the optimized PE. The
Tauri experimental-cross-compilation notice, its unsigned-installer notice,
and NSIS warning 5202 about `-OUTPUTCHARSET` on non-Win32 hosts remain expected
tool capability notices. They do not indicate Rust source warnings. Signing is
a separate release capability and must use an explicit trusted signing command
and certificate rather than hiding that notice.

## Proxmox JIT validation

`Cross-build on Proxmox JIT runner` is an isolated manual workflow for the
first trusted LXC test. It requests the labels `self-hosted`, `proxmox-lxc`,
`crossbuild`, and `mode-ephemeral`, verifies the preinstalled golden toolchain,
and builds only the thin versioned portable executable. It does not download or
package connector runtimes, receive repository secrets, or publish a release.
The regular CI workflow may use the same runner labels for trusted collaborator
pull requests, but never for PRs from unknown author associations.

Each trusted workflow has a separate stable `CARGO_TARGET_DIR`, protected by
workflow concurrency without cancellation so two compiler processes cannot
write to the same target at the same time. The workspace and JIT runner remain
disposable. Manual publication validation is available only through
`workflow_dispatch` in this repository.

Keep the orchestrator in ephemeral mode, inspect the uploaded PE and checksum
on Windows, and confirm that each JIT LXC is removed while only the intended
build cache persists. The orchestrator must accept CI jobs only when GitHub has
already selected the restricted self-hosted labels.

The LXC must never:

- execute PR code from an author association other than `OWNER`, `MEMBER`, or
  `COLLABORATOR`;
- receive publication credentials or repository secrets;
- publish a GitHub Release;
- belong to a runner group shared with unrelated repositories or workflows.

Restrict its runner group to this repository and the intended build workflows.
Keep external and public-fork PR cross-builds hosted, and keep publication on a
GitHub-hosted job. A Windows machine remains required for the final runtime
check: launch both distribution forms, complete first-run preparation, restart
without downloads, and exercise all three connectors.

## Rollout and rollback

Before recovering an official draft, run the E2E workflow with publication
enabled from `main` and require both jobs to pass. Confirm that no
`release-e2e-*` release or tag survives cleanup. Then dispatch
`release-please.yml` with
`recover_package=app` and the original merged release SHA.

Before the first E2E run, update golden 9101 as root, install `pkg-config` and
`libayatana-appindicator3-dev`, verify
`pkg-config --exists ayatana-appindicator3-0.1`, and publish the refreshed
template through the orchestrator's normal golden-image rollout. Do not relax
the LXC `no_new_privs` policy to make package installation work inside a job.

If JIT builds regress, change only the production `build.runs-on` value back to
`ubuntu-latest` and restore its toolchain setup/cache steps; keep the hosted
publish job and publication gates unchanged. If publication verification fails,
do not publish the draft manually: retain the draft, repair the workflow, rerun
the E2E test, and use explicit recovery.
