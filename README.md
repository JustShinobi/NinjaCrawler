# NinjaCrawler

NinjaCrawler is a Windows desktop application for organizing profiles, downloading media, and scheduling recurring synchronization across social platforms.

It is built with Rust, Tauri 2, React, and TypeScript. Metadata and operational state stay in a local SQLite database, while downloaded media remains directly accessible on disk.

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
- Manage bundled connector runtimes such as `gallery-dl`, `yt-dlp`, and Instaloader.
- Add or sync profiles from Chrome through the optional NinjaCrawler Companion extension.

## Provider support

| Provider | Current scope |
| --- | --- |
| Instagram | Multiple accounts, posts, reels, stories, highlights, tagged media, saved posts, profile metadata, and targeted story downloads |
| TikTok | Multiple accounts, videos, photo posts, stories, reposts, avatars, date ranges, and configurable file naming |
| X / Twitter | Profile media timeline, avatars, duplicate prevention, and handle recovery through stable user IDs |
| Reddit | In development; not currently supported for production use |

Provider behavior depends on the platform, account authentication, rate limits, and the capabilities of the bundled connector tools.

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
- Internet access during the first build to download connector bootstrap assets

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

The repository build wrapper prepares connector runtimes, runs lint and tests, and builds the Tauri application:

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

The optional extension in [`NinjaCrawler.Companion`](NinjaCrawler.Companion) can detect supported profile pages, add profiles, queue synchronization, and download the currently selected Instagram story.

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
| `connectors/bootstrap/` | Connector runtime manifest and downloaded build assets |
| `NinjaCrawler.Companion/` | Optional Chrome extension |
| `Tools/` | Windows development, build, smoke-test, and publishing scripts |
| `docs/` | Architecture and Windows distribution notes |

## Additional documentation

- [Architecture](docs/architecture.md)
- [Provider account flow](docs/provider-account-flow.md)
- [Windows distribution](docs/windows-distribution.md)
