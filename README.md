# NinjaCrawler

Windows-first desktop crawler manager built with Rust, Tauri 2, React, and TypeScript.

## Current scope
- First-class provider accounts with multi-account support for Instagram and TikTok.
- Source management with provider/account binding.
- Scheduler sets and plans inspired by SCrawler, but not tied to XML or raw threads.
- Feed sessions, collections, filters, and saved views backed by the same SQLite workspace snapshot as the desktop shell.
- Local-first workspace layout with metadata in SQLite and media on disk.
- Tauri commands and the React admin shell mutate the same persisted workspace state.

## Development
Run commands from the `NinjaCrawler\` directory:

```powershell
npm install
Tools\Dev-Desktop.cmd
```

## Build
```powershell
Tools\Build-Desktop.cmd
```

Release build:

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Build-NinjaCrawler.ps1 -Configuration Release
```

## Publish

Publish release artifacts to the default Windows drop folder:

```powershell
Tools\Publish-Desktop.cmd
```

Publish debug artifacts to a custom folder:

```powershell
powershell -ExecutionPolicy Bypass -File Tools\Publish-NinjaCrawler.ps1 -Configuration Debug -PublishRoot D:\Deploy\NinjaCrawler
```

## Provider Extensibility Boundary

- V1 provider support is compiled into the desktop backend through the internal Rust provider runtime registry.
- Adding or changing a supported provider requires a new application build and publish; dropping extra binaries into the publish folder does not extend the app.
- External tools such as `gallery-dl` and `yt-dlp` remain operator-configured dependencies surfaced through app settings, not runtime-loaded provider plugins.

## Windows Distribution Notes

- `Tools\Build-NinjaCrawler.ps1` runs lint and frontend tests by default before invoking the Tauri/MSVC build.
- `Tools\Publish-NinjaCrawler.ps1` publishes the portable app and installer bundles, backs up replaced files, blocks if a published app process is still running, and verifies copied files by SHA-256.
- The default publish root is `F:\NinjaCrawler`.
- Full workflow notes, operator smoke validation, and the provider boundary are documented in `docs\windows-distribution.md`.

## Notes
- `Tools\Run-InVsDevCmd.cmd` wraps commands with the Visual Studio C++ environment and the Rust user toolchain path.
- Architecture notes live in `docs\architecture.md` and `docs\provider-account-flow.md`.
- Windows build/publish workflow notes live in `docs\windows-distribution.md`.
- Desktop bundles are emitted under `src-tauri\target\<debug|release>\bundle\`.
