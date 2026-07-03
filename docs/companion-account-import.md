# Companion Account Import

## Understanding summary

- The Companion exposes an explicit **Import account** action on Instagram, X/Twitter, and TikTok.
- Import captures the current browser session: cookies, provider identity, browser identity headers, and provider-specific authorization metadata.
- The operator reviews the capture and chooses whether to update a compatible account or create a new account.
- Updating an account replaces only its session and authorization metadata. Paths, source defaults, capabilities, and other account settings remain unchanged.
- The backend retains one previous imported session per account and exposes **Revert last import**.
- A failed validation leaves the imported session in a degraded state and allows manual reversion.

## Assumptions and non-functional requirements

- All secret-bearing data stays on the local machine, travels only to the loopback Companion API, and is stored using the existing DPAPI-backed secret store.
- Secret values never appear in logs, API responses, preview screens, or error messages.
- Capture is transient and user initiated. Network observers are removed after capture or timeout.
- Import and revert operations are transaction-like across SQLite and protected secret files, with staged writes and compensating cleanup.
- The expected scale is a local workspace with tens of accounts. Capture and persistence should complete in a few seconds.
- Provider-specific capture rules are isolated behind adapters because provider headers and identity endpoints change independently.

## Final design

### Extension

Each supported provider has a capture adapter defining its domains, required cookies, identity extraction, and optional authorization metadata. The generic capture layer reads cookies and browser identity. A provider adapter may initiate a harmless authenticated request while transient `webRequest` listeners collect the request and response headers required by that provider.

The popup uses these states:

1. capture;
2. review;
3. destination selection;
4. save;
5. validation result.

Review displays only provider, detected username, cookie count, and names/presence of captured parameters. Compatible destinations are restricted to the same provider. A stable provider user id is the primary match key; normalized username is a fallback that always requires confirmation.

### Backend and API

The Companion API exposes account-import preview and apply operations. Preview returns redacted metadata and compatible account choices. Apply creates an account or updates an explicitly selected account. Revert is also available through a Tauri command for the Accounts UI.

Sensitive Companion routes accept extension-origin requests only, enforce JSON content type and payload limits, validate a strict schema, and never return session values. Existing non-sensitive add/sync routes retain their behavior.

### Persistence

The current protected session payload contains cookies, provider identity, and authorization metadata. SQLite stores only non-secret import metadata and secret references. Before update, the current session and imported authorization state become the sole backup. A newer import replaces the older backup.

Secret payloads use unique references. A new secret is written before the SQLite transaction points to it; failed transactions remove staged files. Old unreferenced secrets are deleted after commit. Revert swaps the current and backup states so an accidental revert can itself be undone.

Account paths, defaults, capabilities, source assignments, and unrelated settings are not included in import backups and are never changed by import or revert.

### Provider scope

- Instagram: cookies, stable identity, User-Agent/client hints, CSRF token, app id, ASBD id, WWW claim, and optional LSD/DTSG values when available.
- X/Twitter: cookies, stable identity, and User-Agent.
- TikTok: cookies, stable identity, and User-Agent.

### Error handling

Missing required cookies or identity blocks import and instructs the operator to log in. Missing optional metadata produces a warning. Capture is cancelable and has a short timeout. Validation runs after persistence; failure marks the account degraded without automatic rollback. A newly created account has no backup until its first update.

### Testing

Tests cover provider adapters, cookie conversion, metadata redaction, destination matching, API validation/origin rules, create/update behavior, preservation of unrelated settings, backup replacement, reversible swaps, degraded validation, transient-listener cleanup, and regressions in existing Companion add/sync behavior.

## Decision log

1. **Explicit capture only.** Automatic capture was rejected to keep consent and data flow visible.
2. **Transient request observation.** Continuous traffic caching and direct browser-profile database access were rejected due to privacy and fragility.
3. **Backend-controlled destination.** The extension suggests matches but never silently chooses an account to overwrite.
4. **Authorization-only updates.** Operational account settings remain untouched.
5. **One rotating backup.** Full history was rejected as unnecessary for the first version.
6. **Manual rollback after failed validation.** Temporary provider failures should not erase a newly imported session automatically.
7. **Protected consolidated payload.** Newly captured secrets are kept in the DPAPI-backed store rather than introduced as plaintext settings.
8. **Provider adapters.** Instagram, Twitter, and TikTok evolve independently.
