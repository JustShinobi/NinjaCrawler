# Provider Account Flow

## Rules
- Provider accounts are first-class entities with stable IDs.
- Sources bind to account IDs, not provider-level defaults.
- Creating or updating a source without an explicit account binding is invalid.
- No silent fallback account is allowed.
- Multi-account support is mandatory in v1 for Instagram and TikTok.

## Auth strategy
- Primary onboarding path: embedded browser inside the desktop app.
- Fallback path: expert import for sessions or cookies when needed for support.
- Stored auth state must be separate from provider defaults and separate from source records.
- Imported session metadata lives in the workspace database, while sensitive payload material is stored in Windows-protected secure storage.

## Runtime shape
- `ProviderAccount` stores identity, auth mode, auth health, capabilities, and validation time.
- `SourceProfile` stores the monitored handle and the explicit account dependency.
- Connectors should consume normalized session data rather than raw UI-specific state.

## Extensibility boundary
- Supported providers come from the backend `ProviderRuntime` registry compiled into the desktop app.
- V1 does not expose a runtime-loaded provider plugin surface for operator drops.
- Adding a provider means shipping backend changes and republishing the desktop build; copying extra binaries into the publish folder is not enough.
- External tool settings remain part of provider execution policy, not provider discovery.

## Provider-specific expectations
### Instagram
- Multiple account profiles must coexist without shared implicit fallback.
- Session capture should preserve cookie and header material needed by the connector.

### TikTok
- Multiple account profiles must coexist in v1.
- Capability reporting should allow different backend paths for videos vs photos when auth support differs.

## Failure behavior
- If an account expires or degrades, dependent sources and scheduler plans should surface that state directly.
- The app must not transparently reroute work to another account.
- Validation must compute account health from persisted session material; UI edits alone cannot declare an account healthy.
