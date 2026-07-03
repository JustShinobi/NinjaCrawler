# Windows Distribution and Operator Workflow

## Purpose

`NinjaCrawler` now has a repeatable Windows build/publish workflow aligned with the existing SCrawler automation style:

- build and bundle the desktop app from the checked-out repository
- optionally run lint and frontend tests before packaging
- publish the generated artifacts into a target folder with backup and hash verification
- document the operator-facing boundary between the compiled provider runtime and external tool configuration

## Working Directory

Run the commands in this document from the `NinjaCrawler\` directory.

## Prerequisites

- Node.js and npm available on `PATH`
- Rust toolchain installed under `%USERPROFILE%\.cargo\bin`
- Visual Studio 2022 Build Tools with the C++ workload
- WebView2 Runtime installed on the target machine

`Tools\Run-InVsDevCmd.cmd` is still the environment wrapper responsible for loading the MSVC toolchain before Tauri builds.

## Build Commands

Debug build with checks:

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Build-NinjaCrawler.ps1
```

Release build with checks:

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Build-NinjaCrawler.ps1 -Configuration Release
```

Skip lint and frontend tests:

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Build-NinjaCrawler.ps1 -Configuration Release -SkipLint -SkipTests
```

Legacy wrapper:

```powershell
Tools\Build-Desktop.cmd
```

By default, the build script runs:

- `npm run lint`
- `npm test`
- `npm run tauri:build` with the selected configuration

Use `-SkipLint` and `-SkipTests` only for packaging smoke checks or local workflow debugging. Keep them enabled for release candidates.

## Published Outputs

The build script emits artifacts under:

- `src-tauri\target\debug\` for `Debug`
- `src-tauri\target\release\` for `Release`

Portable artifacts:

- `ninjacrawler.exe`
- `ninjacrawler.pdb` when available

Bundled installers:

- `bundle\msi\*.msi`
- `bundle\nsis\*-setup.exe`

GitHub releases also contain `NinjaCrawler-Companion-<version>.zip`. The
Companion version is read independently from `NinjaCrawler.Companion/manifest.json`.
Extract the archive, open the browser extension management page, enable developer
mode, and choose **Load unpacked** using the extracted folder.

Starting with Companion 0.3.0, the archive always contains a
`NinjaCrawler-Companion` folder and the manifest has a stable extension ID.
Remove a pre-0.3.0 installation once. Future updates can be extracted over the
same folder and applied with **Reload** on the browser extension management page.

The Companion archive contains only runtime files and installation documentation.
Its SHA-256 hash is included in the release `SHA256SUMS.txt`.

## Publish Commands

Publish release artifacts into `F:\NinjaCrawler`:

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Publish-NinjaCrawler.ps1
```

Publish debug artifacts into a custom folder:

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Publish-NinjaCrawler.ps1 -Configuration Debug -PublishRoot D:\Deploy\NinjaCrawler
```

Publish already-built artifacts without rebuilding:

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Publish-NinjaCrawler.ps1 -SkipBuild -Configuration Release -PublishRoot D:\Deploy\NinjaCrawler
```

Legacy wrapper:

```powershell
Tools\Publish-Desktop.cmd
```

## Publish Layout

The publish root is organized as:

- `app\` for the portable desktop binary
- `bundle\msi\` for MSI packages
- `bundle\nsis\` for NSIS installers
- `_backup\<timestamp>\` for the replaced files from the previous publish

Before copying, the publish script:

- validates that all source artifacts exist
- blocks if a previously published `ninjacrawler.exe` is still running from the destination
- backs up overwritten files
- verifies every copied file with SHA-256

## Operator Workflow

Recommended operator flow for a candidate desktop drop:

1. Run a release build:

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Build-NinjaCrawler.ps1 -Configuration Release
```

2. Publish to the operator drop root:

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Publish-NinjaCrawler.ps1 -Configuration Release -PublishRoot F:\NinjaCrawler
```

3. Launch the published portable app from `F:\NinjaCrawler\app\ninjacrawler.exe`.
4. In `Settings`, confirm the configured external tool paths used by the compiled provider runtimes, especially `gallery-dl` and `yt-dlp`.
5. Validate at least one provider-account health check and one manual source sync from the published build.
6. If installers are part of the release, smoke-test both:
   - `bundle\msi\*.msi`
   - `bundle\nsis\*-setup.exe`

## Provider Extensibility Boundary

The current V1 provider boundary is intentionally narrow:

- Provider support is compiled into the backend via the internal Rust `ProviderRuntime` registry.
- The published app ships a fixed provider set for that build; the drop folder is not a plugin discovery surface.
- Extending provider support requires backend code changes, a new desktop build, and a new publish.
- External tools such as `gallery-dl` and `yt-dlp` remain operator-configured execution dependencies. They enable or degrade capabilities inside an already-supported provider, but they do not add new providers on their own.

Operational implication:

- treat `Settings` tool paths as runtime prerequisites for the supported providers in the current build
- do not expect publishing extra DLLs or executables into `app\` or `bundle\` to register a new provider dynamically

## Validation

Minimum scripted validation for this workflow:

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Build-NinjaCrawler.ps1 -Configuration Debug
```

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Publish-NinjaCrawler.ps1 -Configuration Debug -PublishRoot D:\Deploy\NinjaCrawler-Smoke
```

Wrapper smoke validation:

```powershell
Tools\Build-Desktop.cmd -Configuration Debug -SkipLint -SkipTests
```

```powershell
Tools\Publish-Desktop.cmd -Configuration Debug -SkipBuild -PublishRoot D:\Deploy\NinjaCrawler-Smoke
```

Manual validation after publish:

- confirm the published `app\` folder launches outside the repo
- confirm `_backup\<timestamp>\` is created on overwrite publishes
- confirm provider settings still resolve the expected external tools on the operator machine
- confirm `Close to tray` actually hides the main window instead of quitting the app
- confirm the tray menu can re-open the main window and toggle silent mode
- confirm route-driven foreground actions still land in the expected section after scheduler/runtime events
