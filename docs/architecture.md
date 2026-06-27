# NinjaCrawler Architecture

## Stack
- Desktop shell: Tauri 2
- Backend: Rust
- Frontend: React + TypeScript + Vite
- Persistence target: SQLite for metadata, filesystem for media

## Main domains
- Provider accounts: first-class identities with health and capability state
- Sources: monitored profiles explicitly bound to provider accounts
- Scheduler: named scheduler sets and plans
- Feed: current session, archived sessions, and named collections
- Library: storage roots, file operations, and retention policies
- Settings: external tool paths and connector policy

## Provider Extensibility Boundary
- Provider support is exposed through the internal Rust `ProviderRuntime` registry.
- The boundary is compile-time, not a runtime plugin ABI.
- Adding a new provider requires backend registration, capability metadata, auth/runtime wiring, and a new desktop build.
- External tools configured in settings extend the execution path of supported providers; they do not register new providers by being copied into a publish folder.

## Current scaffold
- `src/`: React admin shell bound to Tauri commands through a shared `WorkspaceSnapshot`
- `src-tauri/`: Rust application layer, SQLite-backed workspace repository, provider catalog, and storage layout
- `src-tauri/migrations/0001_initial.sql`: baseline schema used by the live workspace bootstrap
- `src-tauri/target/debug/bundle/`: validated MSI and NSIS debug bundles

## Bootstrap contract
- Workspace bootstrap must be deterministic and empty-state safe.
- A fresh workspace may seed default settings required for tool paths and policy defaults, but it must not seed demo accounts, sources, scheduler sets, sessions, collections, media, filters, or views.
- If bootstrap cannot read or create the real workspace state, the desktop shell must fail fast instead of falling back to mock product data.
- Empty workspace is a valid product state. The frontend must render empty sections rather than synthetic operational data.

## Delivered MVP slices
1. SQLite-backed workspace bootstrap with persisted CRUD for accounts, sources, scheduler sets, plans, feed collections, saved filters, saved views, and settings.
2. Provider catalog surfaced to the frontend with explicit multi-account metadata and default capabilities.
3. React desktop shell wired to the persisted workspace contract without operational mock fallback.
4. Enforced source-to-account binding contract across backend validation and frontend workflow gating.
5. Secure imported-session storage for provider accounts, with Windows-protected secret material stored outside account identity records.
6. Account validation commands and session summaries exposed through the Accounts workspace.
7. Section UIs for accounts, sources, scheduler, feed, library, and settings operating against the same Tauri command surface.

## Remaining implementation slices
1. Broader notification surfaces and richer operator telemetry beyond the current parity baseline.
2. Release/update ergonomics beyond the current scripted Windows distribution workflow.
3. Additional provider/runtime coverage beyond the compiled V1 registry.
