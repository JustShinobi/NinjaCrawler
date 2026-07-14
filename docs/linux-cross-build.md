# Linux cross-build contract

## Current hosted path

NinjaCrawler targets Windows x64 while CI builds on GitHub-hosted `ubuntu-latest`.
`Tools/Build-NinjaCrawler.ps1` keeps the Visual Studio/MSVC path for local Windows
builds and selects `cargo-xwin --target x86_64-pc-windows-msvc` on Linux.

The fixed Linux toolchain is:

- Rust stable and `cargo-xwin` 0.22.0;
- Node.js 22;
- LLVM/Clang/LLD 18.1.3 and NSIS 3.09 from Ubuntu 24.04;
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

Every pull request executes the hosted cross-build with read-only repository
permissions, `persist-credentials: false`, and no build secrets. The release
workflow separates concerns:

1. `build` checks out the trusted release ref, compiles and uploads an Actions
   artifact with read-only permissions;
2. `publish` downloads that artifact and is the only job granted
   `contents: write`.

`Windows cross-build validation` is a manual dispatch that builds portable plus
NSIS, rejects ZIP/MSI artifacts, validates checksums, and never publishes a
release.

## Proxmox JIT validation

`Cross-build on Proxmox JIT runner` is an isolated manual workflow for the
first trusted LXC test. It requests the labels `self-hosted`, `proxmox-lxc`,
`crossbuild`, and `mode-ephemeral`, verifies the preinstalled golden toolchain,
and builds only the thin versioned portable executable. It does not download or
package connector runtimes, receive repository secrets, publish a release, or
run for pull requests.

The persistent cache uses a stable `CARGO_TARGET_DIR`, protected by workflow
concurrency without cancellation so two compiler processes cannot write to it
at the same time. The workspace and JIT runner remain disposable. The workflow
must first exist on the default branch before GitHub allows a manual dispatch.

During the first test, keep the orchestrator in ephemeral mode with one active
runner, verify that the job is accepted only for `workflow_dispatch`, inspect
the uploaded PE and checksum on Windows, and confirm that the LXC is removed
while only the intended build cache persists.

The LXC must never:

- execute pull-request or fork code;
- receive publication credentials or repository secrets;
- publish a GitHub Release;
- belong to a runner group shared with unrelated repositories or workflows.

Restrict its runner group to this repository and the trusted validation
workflow. Keep the hosted PR cross-build unchanged and keep publication on a
GitHub-hosted job until the JIT path has been measured and validated. A Windows
machine remains required for the final runtime check: launch both distribution
forms, complete first-run preparation, restart without downloads, and exercise
all three connectors.
