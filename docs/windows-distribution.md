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

- `bundle\nsis\*-setup.exe`

Each desktop GitHub release (`vX.Y.Z`) also contains
`NinjaCrawler-Companion-<companionVersion>.zip`. The Companion version is read
independently from `NinjaCrawler.Companion/manifest.json` at packaging time and
is not forced to match the desktop version. Companion-only releases under
`companion-vX.Y.Z` continue to publish the same ZIP shape for extension-only
updates.

Extract the archive, open the browser extension management page, enable developer
mode, and choose **Load unpacked** using the extracted folder.

Starting with Companion 0.3.0, the archive always contains a
`NinjaCrawler-Companion` folder and the manifest has a stable extension ID.
Remove a pre-0.3.0 installation once. Future updates can be extracted over the
same folder and applied with **Reload** on the browser extension management page.

The Companion archive contains only runtime files and installation documentation.
When co-shipped with the desktop app, its SHA-256 hash is included in that
release’s `SHA256SUMS.txt`.

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
6. Smoke-test `bundle\nsis\*-setup.exe` when an installer is part of the release.

## Auto-Update (tauri-plugin-updater)

The desktop app ships with `tauri-plugin-updater` v2 wired end to end:

- `src-tauri/tauri.conf.json` declares the updater endpoint
  `https://github.com/JustShinobi/NinjaCrawler/releases/latest/download/latest.json`
  and the minisign public key. `bundle.createUpdaterArtifacts` stays `false`:
  the installer is signed out-of-band in the release **publish** job, so the
  unprivileged self-hosted build never needs the signing key.
- The **About** panel shows an **Install update** button when a newer release is
  detected. It downloads the signed installer with a live progress indicator and
  relaunches the app.
- The lightweight "update available" check (`check_app_update`) still queries the
  GitHub API directly and is independent of the signed updater flow.

Signing is already configured for this repository: the real public key is
committed in `tauri.conf.json` and the CI secrets are set. If the pubkey were
ever reset to a placeholder, the app would still run and the "update available"
banner would still work; only the **Install update** action fails gracefully
with a clear "auto-update is not configured" message. Nothing panics.

### Operator setup (one-time, or when rotating the key pair)

Auto-update requires a minisign key pair. These steps are **not** automated
because the private key must never be committed.

1. **Generate the key pair** (uses the project-local Tauri CLI):

   ```powershell
   npx tauri signer generate -w $env:USERPROFILE\.tauri\ninjacrawler-updater.key
   ```

   This prints a **public key** and writes a password-protected **private key**.

2. **Store the private key safely.** Keep the `.key` file (and its password) in a
   secret manager. Losing it means you cannot ship updates that existing installs
   will accept.

3. **Publish the public key.** Set `plugins.updater.pubkey` in
   `src-tauri/tauri.conf.json` to the generated public key, then commit that
   change.

4. **Add the CI secrets.** In the GitHub repository settings add:
   - `TAURI_SIGNING_PRIVATE_KEY` — the contents of the generated `.key` file.
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` — the password chosen in step 1.

   Only the **publish** job of `.github/workflows/release.yml` consumes these
   secrets. They are deliberately kept out of the self-hosted **build** job,
   which runs unprivileged.

5. **Leave updater artifacts disabled.** `bundle.createUpdaterArtifacts` must
   stay `false` in `src-tauri/tauri.conf.json`. The build no longer signs
   anything, so it never needs the key and cannot fail on a missing secret.
   Signing happens in publish (see below).

### `latest.json` (automatic)

The updater manifest is generated and published automatically by the release
pipeline. In the **publish** job (`ubuntu-latest`, where the signing secrets
live), `Tools/New-UpdaterManifest.ps1` signs the versioned NSIS installer with
`tauri signer sign` — reading the key and password from
`TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in the
environment (never on the command line) — and writes a `latest.json` in the
tauri-plugin-updater v2 shape:

```json
{
  "version": "0.24.1",
  "pub_date": "2026-07-18T00:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "<contents of the .sig file>",
      "url": "https://github.com/JustShinobi/NinjaCrawler/releases/download/v0.24.1/NinjaCrawler-0.24.1-windows-x64-setup.exe"
    }
  }
}
```

- `signature` is the **contents** of the signature produced by
  `tauri signer sign` (not a path).
- `url` points at the **versioned** release asset (`.../download/vX.Y.Z/...`),
  using the exact name of the published NSIS setup.
- `latest.json` is added to the published release assets and appended to the
  existing `SHA256SUMS.txt`. The signature is embedded in the manifest, so the
  raw `.sig` file is not published separately.

If signing fails (the `TAURI_SIGNING_PRIVATE_KEY*` secrets are missing or
invalid), the **Sign installer and generate updater manifest** step of the
publish job **fails fast** with a clear error instead of publishing an unsigned
or manifest-less release. The self-hosted build job produces the installer but
never `latest.json`; the manifest always originates in publish.

**Troubleshooting**

- *Publish fails with "TAURI_SIGNING_PRIVATE_KEY is not set" or a signing
  error*: the signing secrets are missing or wrong. Confirm both are set on the
  repository (see the operator setup above).
- *Install update reports no manifest*: verify `latest.json` is attached to the
  published release and reachable at the configured `latest` endpoint URL.

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
