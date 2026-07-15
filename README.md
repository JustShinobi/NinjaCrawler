<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/brand/ninjacrawler-lockup-horizontal-dark.svg">
  <img src="assets/brand/ninjacrawler-lockup-horizontal.svg" alt="NinjaCrawler" width="520">
</picture>

**Windows desktop workspace for organizing social profiles, downloading media, and running recurring syncs.**

`Windows` · `x64` · `Instagram` · `TikTok` · `X / Twitter`

[![Latest release](https://img.shields.io/github/v/release/MetalDevOps/NinjaCrawler?include_prereleases&display_name=tag&sort=semver&label=latest&color=2ea44f)](https://github.com/MetalDevOps/NinjaCrawler/releases)
[![Downloads](https://img.shields.io/github/downloads/MetalDevOps/NinjaCrawler/total?label=downloads&color=1f6feb)](https://github.com/MetalDevOps/NinjaCrawler/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/MetalDevOps/NinjaCrawler/ci.yml?branch=develop&label=ci)](https://github.com/MetalDevOps/NinjaCrawler/actions/workflows/ci.yml)

<!-- ninjacrawler-release-start -->
<a href="https://github.com/MetalDevOps/NinjaCrawler/releases/download/v0.19.2/NinjaCrawler-0.19.2-windows-x64-setup.exe">
  <img src="https://img.shields.io/badge/Download%20for%20Windows-x64-2ea44f?style=for-the-badge&logo=windows&logoColor=white" alt="Download for Windows x64" height="46">
</a>

<sub>Current release: v0.19.2 · [portable executable](https://github.com/MetalDevOps/NinjaCrawler/releases/download/v0.19.2/NinjaCrawler-0.19.2-windows-x64-portable.exe) · [checksums](https://github.com/MetalDevOps/NinjaCrawler/releases/download/v0.19.2/SHA256SUMS.txt) · [all releases](https://github.com/MetalDevOps/NinjaCrawler/releases) · [changelog](CHANGELOG.md)</sub>
<!-- ninjacrawler-release-end -->

</div>

---

NinjaCrawler is built with Rust, Tauri 2, React, and TypeScript. Metadata and operational state stay in a local SQLite database, while downloaded media remains directly accessible on disk.

> [!IMPORTANT]
> NinjaCrawler is under active development. Configuration formats, provider behavior, and migration compatibility may change before the first stable release.

## Highlights

- Manage profiles and provider accounts from a native Windows workspace.
- Download and catalog media from Instagram, TikTok, and X/Twitter.
- Configure sync behavior per account and per profile.
- Run persistent download queues independently per provider.
- Pause, resume, cancel, retry, and reorder queued work.
- Create scheduler sets, plans, groups, filters, and date constraints.
- Import existing SCrawler media without duplicating files.
- Browse downloaded media by profile and date, open the original post, or reveal the local file.
- Download and manage verified connector runtimes such as `gallery-dl`, `yt-dlp`, and Instaloader.
- Add or sync profiles from Chrome through the optional NinjaCrawler Companion extension.

## Getting started

<!-- ninjacrawler-release-start -->
1. **[Download NinjaCrawler for Windows](https://github.com/MetalDevOps/NinjaCrawler/releases/download/v0.19.2/NinjaCrawler-0.19.2-windows-x64-setup.exe)**.
2. Windows SmartScreen may warn about an unknown publisher because the current builds are unsigned. Choose **More info** and then **Run anyway** if you trust this repository.
3. Install with the setup executable, or download the standalone [portable executable](https://github.com/MetalDevOps/NinjaCrawler/releases/download/v0.19.2/NinjaCrawler-0.19.2-windows-x64-portable.exe).
4. Optional: compare the file hash with [SHA256SUMS.txt](https://github.com/MetalDevOps/NinjaCrawler/releases/download/v0.19.2/SHA256SUMS.txt).
<!-- ninjacrawler-release-end -->

On first launch, an internet connection is required while NinjaCrawler downloads and verifies its connector runtimes. Application data and connectors remain under `%LOCALAPPDATA%\NinjaCrawler`; portable means that the app itself needs no installation.

## Provider support

| Provider | Current scope |
| --- | --- |
| Instagram | Multiple accounts, posts, reels, stories, highlights, tagged media, saved posts, profile metadata, and targeted story downloads |
| TikTok | Multiple accounts, videos, photo posts, stories, reposts, avatars, date ranges, and configurable file naming |
| X / Twitter | Profile media timeline, avatars, duplicate prevention, and handle recovery through stable user IDs |

Provider behavior depends on the platform, account authentication, rate limits, and the capabilities of the managed connector tools.

## How it works

```text
React workspace
      │
      ▼
Tauri command bridge
      │
      ▼
Rust application runtime ─── SQLite workspace
      │
      ├── provider queues and scheduler
      ├── internal provider connectors
      └── managed external tools
              │
              ▼
        Media folders on disk
```

NinjaCrawler uses compiled provider integrations rather than a drop-in plugin system. External executables provide selected extraction capabilities, but provider rules, persistence, queueing, and UI integration live in the application itself.

## Requirements

Development and local builds currently target Windows.

- Windows 10 or Windows 11
- Node.js LTS and npm
- Rust stable with the MSVC target
- Visual Studio 2022 Build Tools with the Desktop development with C++ workload
- Microsoft Edge WebView2 Runtime
- PowerShell 5.1 or newer
- Internet access during first-run connector preparation

## Development

Clone the repository and install the locked frontend dependencies:

```powershell
git clone https://github.com/MetalDevOps/NinjaCrawler.git
cd NinjaCrawler
npm ci
```

Start the Tauri development application:

```powershell
Tools\Dev-Desktop.cmd
```

The frontend can also run independently in a browser, although native commands will not be available:

```powershell
npm run dev
```

## Validation

Run the frontend checks independently:

```powershell
npm run lint
npm test
npm run build
```

The repository build wrapper runs lint and tests, then builds the Tauri application:

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Build-NinjaCrawler.ps1 -Configuration Debug
```

Before publishing a change, run the complete Release build and executable smoke test:

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Build-NinjaCrawler.ps1 -Configuration Release
powershell -ExecutionPolicy Bypass -File Tools\SmokeTest-NinjaCrawler.ps1 -Configuration Release
```

Portable artifacts are written to:

```text
src-tauri\target\release\
```

Installer bundles are written below:

```text
src-tauri\target\release\bundle\
```

Use `-PortableOnly` with the build wrapper when installers are not required.

## Continuous integration and releases

GitHub Actions runs lint, frontend tests, a production dependency audit, and a Windows x64 cross-build on `ubuntu-latest` for every pull request and changes to `main`.

Releases are tag-driven. The desktop app and the Chrome Companion are versioned on **independent** Release Please tracks so a release that only touches one of them never bumps the other:

- The **app** track bumps `package.json`, `src-tauri/tauri.conf.json`, and `src-tauri/Cargo.toml`, and tags `vX.Y.Z`.
- The **Companion** track bumps `NinjaCrawler.Companion/manifest.json` — driven only by commits under `NinjaCrawler.Companion/` — and tags `companion-vX.Y.Z`.

1. Merge Conventional Commit changes from `develop` into `main`.
2. Release Please opens or updates the release PR, bumping whichever track(s) changed.
3. Merging the release PR creates the draft GitHub Release(s) and dispatches the matching release workflow: the Windows build for the app, and the Companion packaging workflow for the extension.

The app release workflow validates that the app versions match the tag, builds on a read-only hosted Linux job, and publishes from a separate credentialed job:

- A generated changelog grouped from Conventional Commit messages since the previous version.
- A standalone portable Windows executable.
- An NSIS setup executable.
- SHA-256 checksums for both executables and the changelog.

The Companion release workflow packages and publishes, under the `companion-vX.Y.Z` tag:

- A NinjaCrawler Companion ZIP named from `NinjaCrawler.Companion/manifest.json`.
- SHA-256 checksums for the asset.

The generated changelog becomes the GitHub Release description and is also attached as `CHANGELOG.md`. Versions below `1.0.0` are published as GitHub prereleases. An existing tag can be republished manually from the **Release** workflow; its notes and assets are updated in place.

## Local data

By default, application data is stored under:

```text
%LOCALAPPDATA%\NinjaCrawler\
```

The SQLite database is stored at:

```text
%LOCALAPPDATA%\NinjaCrawler\data\ninjacrawler.db
```

Downloaded media defaults to:

```text
%USERPROFILE%\Pictures\NinjaCrawler\
```

The media root and provider-specific paths can be changed from the application. Back up the database and media folders before testing migrations or destructive maintenance operations.

Authentication material is stored locally and is not intended to be committed to the repository.

## Chrome companion

The optional extension in [`NinjaCrawler.Companion`](NinjaCrawler.Companion) can detect supported profile pages across all open tabs, add a selected batch, queue synchronization, and download the currently selected story or video.

For local installation:

1. Start NinjaCrawler.
2. Open `chrome://extensions`.
3. Enable **Developer mode**.
4. Choose **Load unpacked**.
5. Select the `NinjaCrawler.Companion` directory.

The extension communicates only with the local desktop API at `127.0.0.1`.

## Repository layout

| Path | Purpose |
| --- | --- |
| `src/` | React workspace, desktop windows, state, bridge, and frontend tests |
| `src-tauri/` | Rust backend, SQLite migrations, provider connectors, queues, and scheduler |
| `connectors/manifest.json` | Fixed connector versions and GitHub release asset metadata |
| `NinjaCrawler.Companion/` | Optional Chrome extension |
| `Tools/` | Windows development, build, smoke-test, and publishing scripts |
| `docs/` | Architecture and Windows distribution notes |

## Additional documentation

- [Architecture](docs/architecture.md)
- [Provider account flow](docs/provider-account-flow.md)
- [Windows distribution](docs/windows-distribution.md)
