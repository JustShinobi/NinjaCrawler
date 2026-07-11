# Linux Cross-Build (Debian LXC)

## Purpose

`NinjaCrawler` ships as a Windows x64 desktop app (Tauri v2 → `ninjacrawler.exe`,
WebView2). The default build/publish path runs on a Windows runner and uses the
MSVC toolchain via `Tools\Run-InVsDevCmd.cmd`.

This document describes an alternative path that produces the same Windows
artifacts **from a Debian LXC container** (e.g. on a Proxmox cluster), without a
Windows VM. It cross-compiles the MSVC target with `cargo-xwin` and drops the
Windows-only steps (the WiX/MSI bundler and the GUI smoke test).

> LXC is Linux (it shares the host kernel); it cannot run Windows. "Building on
> LXC" here means **cross-compiling** a Windows binary from Linux, not running a
> Windows environment.

## What this path produces vs. drops

| Artifact                         | Linux cross-build | Notes |
| -------------------------------- | ----------------- | ----- |
| `ninjacrawler.exe` (portable)    | ✅ Yes            | `tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc` |
| Connector bootstrap binaries     | ✅ Yes            | `Prepare-ConnectorBootstrap.ps1` only downloads third-party releases; OS-agnostic |
| Portable zip + Companion zip + SHA256SUMS | ✅ Yes   | Pure packaging; runs under `pwsh` on Linux |
| NSIS installer (`-setup.exe`)    | ✅ Likely         | Tauri's NSIS bundler uses `makensis`, available on Linux |
| MSI installer (WiX)              | ❌ No             | WiX (`candle.exe`/`light.exe`) is Windows-only; wine is fragile |
| GUI smoke test                   | ❌ No             | `SmokeTest-NinjaCrawler.ps1` needs a real WebView2 window; not reproducible headless |

The MSI is dropped intentionally: the project distributes portable-first and has
no managed-deployment requirement (no GPO/SCCM/Intune/winget usage in the repo),
so NSIS + portable zip cover installation fully.

## Recommended machine spec

The build cost is dominated by the Rust/Tauri compile (uses all cores, large
`target/`) plus the Windows SDK/CRT cache that `cargo-xwin` downloads on first
run. LXC adds negligible overhead over the host, so size for the compiler.

| Resource | Minimum | Recommended | Why |
| -------- | ------- | ----------- | --- |
| vCPU     | 4       | **8**       | `cargo`/`rustc` parallelize across cores; the cold compile is CPU-bound |
| RAM      | 6 GB    | **12 GB**   | Peak `rustc` codegen + `lld` linking scales with parallel jobs; 8 GB works, 12 GB gives headroom |
| Disk     | 25 GB   | **40–50 GB**| See breakdown below; leave room for artifacts and warm caches |
| Swap     | 2 GB    | 4 GB        | Safety margin for link-time peaks |

Disk breakdown (warm):

- `src-tauri/target/` (cross release): ~8–12 GB
- `~/.cargo` (registry + git): ~2 GB
- `cargo-xwin` cache (Windows SDK + CRT): ~2–3 GB
- Rust toolchain + target: ~1.5 GB
- `node_modules`: ~0.5 GB

Container notes:

- An **unprivileged** LXC is sufficient. No GPU/nesting required.
- Enable `nesting=1` only if you also run Docker *inside* the container (not
  needed to run a GitHub Actions runner directly).
- Persist `~/.cargo` and `src-tauri/target/` across runs — this is the single
  biggest speedup (cold compile: minutes; warm rebuild of the final crate:
  seconds to ~1 min).

## Container toolchain

Debian 12/13, headless. Install:

- **Rust** (stable) + `rustup target add x86_64-pc-windows-msvc`
- **`cargo-xwin`**: `cargo install cargo-xwin` (downloads the Windows SDK/CRT on
  demand and drives the MSVC link without Visual Studio)
- **LLVM/Clang + `lld`**: the linker `cargo-xwin` uses
- **Node.js 22** + npm (frontend: `tsc -b && vite build`)
- **PowerShell (`pwsh`)**: to reuse the bootstrap/packaging scripts unchanged
- **NSIS** (`nsis`, apt): only if generating the installer; not needed for the
  portable `.exe` alone

Dependency notes: the Windows-native crates (`winreg`, `rfd`, `trash`,
`tray-icon`) compile against the SDK that `cargo-xwin` provides. `reqwest`
already uses `rustls-tls` and `rusqlite` is `bundled`, so there is no OpenSSL or
other system-library requirement.

## Required repo changes

1. **Build layer** — `Tools\Build-NinjaCrawler.ps1` routes everything through
   `Run-InVsDevCmd.cmd` (Windows-only). Add a Linux branch (detect `$IsWindows`)
   that instead runs:
   - `npm run build` (frontend)
   - `tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --no-bundle`
     (or drop `--no-bundle` and add `--bundles nsis` for the installer)

   Keep the existing Windows branch intact for local Windows builds.

2. **Bundle targets** — `src-tauri/tauri.conf.json`: change `"targets": "all"`
   to `["nsis"]` (or an app/exe-only target) so the build does not require WiX.

3. **Packaging** — `Tools\Package-NinjaCrawlerRelease.ps1`: remove the MSI
   requirement (the `throw` when no `.msi` is found and the MSI copy step). The
   rest (portable zip, Companion zip, `SHA256SUMS.txt`) runs under `pwsh` on
   Linux without change.

4. No change needed to `Tools\Prepare-ConnectorBootstrap.ps1` — it only performs
   HTTP downloads of third-party binaries.

## CI integration

Two routes, both remove the Windows runner:

- **A — GitHub-hosted `ubuntu-latest`**: simplest; swap `runs-on: windows-latest`
  for `ubuntu-latest` and add the `cargo-xwin` setup. No self-hosted infra.
- **B — Self-hosted runner on the Debian LXC** (uses the Proxmox cluster):
  register the LXC as a self-hosted runner. Main benefit is a **persistent warm
  cache** (`~/.cargo`, `src-tauri/target/`), which cuts the Rust/Tauri compile
  time dramatically. Recommended when build minutes or cold-compile time matter.

The frontend `quality` job already runs on `ubuntu-latest` and is unaffected.

## Out of scope / limitations

- **GUI smoke test** is not available headless (WebView2 is Windows-only). A
  cheap sanity check (PE header inspection, `--version`-style invocation under
  wine) is not equivalent to launching the real window. Treat full app
  validation as a manual, pre-release step on a real Windows machine.
- **Code signing**, if ever added, uses `signtool` on Windows; `osslsigncode`
  is a Linux alternative but is a separate concern from this build path.
- The **MSI** remains the only genuinely Windows-only artifact. If a managed
  deployment need appears later, generate the MSI in a separate Windows job and
  keep the rest on Linux.
