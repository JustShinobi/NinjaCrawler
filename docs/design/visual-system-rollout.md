# NinjaCrawler visual system rollout

## Objective

Extend the main window's custom titlebar and the Precision Stealth visual language to every Tauri surface without changing business flows, bridge contracts, persistence, or domain models.

## Target architecture

- Extract the main titlebar into a configurable `WindowTitlebar` primitive with compact and full lockups, optional menus, window title, and native controls.
- Keep window commands behind the existing injectable controller so browser tests remain independent from the Tauri runtime.
- Introduce a shared `WindowShell` for titlebar, content scrolling, safe minimum sizes, status/footer regions, and theme initialization.
- Make each Tauri entry point declare its shell configuration instead of duplicating window chrome and page spacing.
- Keep dialogs inside their owning window on `InternalDialog`; use the custom titlebar only for actual Tauri windows.

## Phase 1: inventory and contracts

1. Catalogue every configured and dynamically-created Tauri window, its entry point, minimum size, native decorations, menus, close behavior, and scroll owner.
2. Record window-specific commands and keyboard behavior, including Escape, Alt+F4, Win+Arrow, maximization, modal ownership, and focus restoration.
3. Capture light/dark baselines at minimum, default, and maximized sizes before modifying each window.
4. Classify surfaces as operational, editor, inspector, or utility so shell density and titlebar content are intentional.

### Inventory status (living)

| Label | Title | Class | Shell | Titlebar | `decorations: false` | Status |
|-------|-------|-------|-------|----------|----------------------|--------|
| `main` | NinjaCrawler | main | No (`.app-shell`) | `MainTitlebar` | Yes | Partial — reference chrome, not on `WindowShell` |
| `runtime-log` | Runtime Log | utility | Yes (`compact`) | Yes | Yes | Migrated |
| `source-sync-queue` | Queue Status | utility | Yes (`compact`) | Yes | Yes | Migrated |
| `connector-debug` | Connector Debugger | utility | Yes (`inspector`) | Yes | Yes | Migrated |
| `connector-runtimes` | Connector Runtimes | utility | Yes (`compact`) | Yes | Yes | Migrated |
| `accounts` | Accounts editor | editing | Yes | Yes | Yes | Migrated |
| `profile-editor` | Profile editor | editing | Yes | Yes | Yes | Migrated |
| `profile-view-*` | Profile View | media-heavy | Yes (`compact`) | Yes | Yes | Migrated |
| `scheduler-plans` | Scheduler | operational | Yes (`compact`) | Yes | Yes | Migrated |
| `plans` | Plans | operational | Yes (`compact`) | Yes | Yes | Migrated |
| `import` | Import | operational | Yes (`compact`) | Yes | Yes | Migrated |
| `batch-editor` | Change Parameters | editing | Yes (`compact`) | Yes | Yes | Migrated |
| `single-videos` | Single Videos | media-heavy | Yes (`compact`) | Yes | Yes | Migrated |

**Out of scope for Phase 3 shell migration**

| Surface | Reason |
|---------|--------|
| Preferences (ex-Settings) | `InternalDialog` on main — correct host for dialogs; **not** a Tauri window. Slim hub for cross-cutting prefs (Appearance / Desktop / Sync & safety / Media). Domain keys belong in Import, Accounts, Scheduler, etc. |
| Companion popup | Phase 4 only (browser extension; no desktop window controls) |
| TikTok likes ephemeral webview | Headless connector helper, not product chrome |
| MediaLightbox | In-window overlay, not a Tauri window |

### Preferences IA (complete hybrid)

| Surface | Contents |
|---------|----------|
| **Preferences** (dialog) | Appearance (theme), Desktop (close to tray, silent mode), Media naming (Instagram patterns only) |
| **Accounts → Workspace** | Session import, block duplicate user id, global inter-profile delay, archived session retention |
| **Plans → Notifications** | Workspace default notification style for **new** plans (`policy.notifications.default`) |
| **About → Local paths** | Editable default media root (`storage.media_root`) for new sources |
| **Import** | Scrawler disabled roots (`imports.*`) — never in Preferences |

**Hidden everywhere in Preferences:** `tool.*.path`, `instagram.sync.*`, `runtime.*`, plus all domain-homed keys above.

Shared controls: `src/features/settings/appSettingControls.tsx` (toggle / select / text + Save).

## Phase 2: shared primitives and tokens

1. Generalize `MainTitlebar` into `WindowTitlebar` while preserving the tested drag threshold and double-click behavior.
2. Add semantic tokens for window chrome, popovers, control heights, content gutters, typography roles, status colors, focus, shadows, and separators.
3. Standardize menu triggers, menu rows, cascades, buttons, fields, badges, tabs, toggles, tables, disclosures, and empty/loading/error states.
4. Use Bahnschrift for operational hierarchy, Corbel/Segoe UI for readable UI text, and Cascadia Mono only for paths, versions, identifiers, and data.
5. Remove decorative gradients, hover reflow, local font overrides, arbitrary stacking values, and duplicated dark-theme patches.

### Window chrome rules (stacked undecorated windows)

Undecorated Tauri windows share one continuous charcoal/cream field with their content and with sibling windows. Chrome must stay readable when windows overlap.

**Required for every custom-decorated product window**

1. Mount `WindowTitlebar` (or `MainTitlebar` on main). Do not invent a one-off drag bar.
2. Prefer `WindowShell` for standalone windows so the content region is the sole scroll owner and the shell receives the inset frame.
3. Set `decorations: false` only after shell + titlebar ship in the same change.
4. Grant only the window permissions actually used (see checklist below).
5. Keep `InternalDialog` on in-content chrome — never a second titlebar.

**Titlebar contrast contract**

| State | Background | Accent hairline (2px top) | Title / status text | Edge |
|-------|------------|---------------------------|---------------------|------|
| Focused | `--bg-titlebar` (elevated vs content) | Full `--titlebar-accent-bar` (brand accent) | `--titlebar-ink` | `--titlebar-edge` + soft bottom lift |
| Blurred | `--bg-titlebar-inactive` | Muted mix, lower opacity | `--titlebar-ink-muted` | No strip shadow |

Implementation hooks (must stay in sync):

- Classes on the titlebar: `is-window-focused` / `is-window-blurred`
- `data-window-focused` on the titlebar and `document.documentElement`
- Focus sources: DOM `focus`/`blur`, optional Tauri `isFocused` + `onFocusChanged` via injectable `WindowController`

**Semantic chrome tokens**

| Token | Role |
|-------|------|
| `--bg-titlebar` | Focused titlebar strip (distinct from content shell) |
| `--bg-titlebar-inactive` | Blurred titlebar strip |
| `--titlebar-ink` / `--titlebar-ink-muted` | Focused vs blurred title text |
| `--titlebar-edge` | Bottom separator / soft shadow mix |
| `--titlebar-accent-bar` | Top 2px brand hairline (primary stacking cue) |
| `--window-frame-border` | Inset 1px frame on `.window-shell` so stacked surfaces separate |

Light and dark values are intentional stacking contrast, not decorative theming.

**Window frame**

- `.window-shell` draws `box-shadow: inset 0 0 0 1px var(--window-frame-border)`.
- Main uses `.app-shell` + `MainTitlebar` and is a documented exception without the shell inset until/unless parity is added.

**Permissions checklist when `decorations: false`**

- `core:window:allow-close`
- `core:window:allow-is-focused`
- `core:window:allow-is-maximized`
- `core:window:allow-minimize`
- `core:window:allow-toggle-maximize`
- `core:window:allow-start-dragging`
- `core:window:allow-set-title` when the page updates the OS/taskbar title (e.g. Profile View)

**Tests**

- Injectable `WindowController` for browser tests.
- Assert focused/blurred classes and `document.documentElement.dataset.windowFocused`.
- Preserve drag threshold and double-click maximize coverage.

## Phase 3: window migration

Migrate in risk-based groups, completing tests and visual review before moving to the next group.

1. **Utility windows:** Runtime Log, Queue Status, Connector Debug, and Connector Runtimes.  
   *Status: done.*
2. **Operational windows:** Scheduler, Plans, Single Videos, and Import.  
   *Status: done (Single Videos under media-heavy below as well).*
3. **Editing windows:** Profile Editor, Batch Editor, Accounts, and Settings.  
   *Status: Profile Editor, Batch Editor, Accounts done; Settings stays `InternalDialog`.*
4. **Media-heavy windows:** Profile View and any lightbox-owned auxiliary window.  
   *Status: Profile View + Single Videos done (shared chrome + layered Escape).*

### Remaining follow-ups

1. Optional: bring main onto a frame-inset parity path if stacked-edge review needs all four edges on main.
2. Optional: align remaining entry Escape handlers on migrated utilities to always use `closeDesktopWindow()`.
3. Global Phase 2 cleanup of decorative gradients still present in non-window chrome CSS.

For each window:

- set `decorations: false` only after the shared shell is mounted;
- grant only the Tauri window permissions actually used;
- provide an accessible title and minimize/maximize/close controls appropriate to that window;
- preserve the current minimum dimensions and make the content region the only scroll owner;
- verify that inputs, menus, tabs, and other interactive regions never initiate dragging;
- remove duplicate in-content titles or close buttons once the titlebar provides them;
- preserve commands, shortcuts, loading behavior, selection, and state restoration;
- verify **stacked** chrome: open this window over main (or another utility) in light and dark — focused titlebar keeps accent hairline + elevated strip; blurred titlebar dims; shell frame remains visible;
- prefer layered Escape (overlay → mode → close window) with `stopImmediatePropagation` where Profile View is the reference;
- prefer `closeDesktopWindow()` over raw `window.close()` for consistent Tauri/browser fallback.

## Phase 4: Companion and cross-product consistency

1. Keep the Companion popup on the shared charcoal, cream, burnt-orange, teal, green, and red semantic palette.
2. Use the angular-trail symbol in the browser toolbar and a compact symbol plus wordmark in the popup header.
3. Align button hierarchy, focus treatment, status pills, panels, typography, and light/dark behavior with the desktop app.
4. Keep extension-specific density and native browser conventions; do not reproduce desktop window controls inside the popup.
5. Add a packaging check that rejects missing or stale Companion icon sizes.

Companion is **not** part of the Phase 3 desktop `WindowShell` / `decorations: false` migration.

## Phase 5: validation and rollout

- Unit-test window commands, maximized state, accessible names, drag isolation, menu keyboard behavior, Escape, focus return, and titlebar focus/blur chrome.
- Test every window at its minimum/default size and maximized at Windows scaling of 100%, 125%, 150%, and 200%.
- Validate light and dark themes, long paths, large counts, localization-safe labels, overflow, and focus visibility.
- Confirm WCAG AA contrast and one clear primary action per context.
- Visual review includes at least one **stacked pair** (main + Profile View, and main + one utility) focused and blurred.
- Run Vitest, ESLint, TypeScript/Vite build, icon generation, Rust tests, Release build, NSIS packaging, and the Tauri smoke test.
- Migrate and review one window group per pull request so regressions are isolated and rollback remains straightforward.

## Completion criteria

- Every Tauri product window uses the shared shell or has a documented native-decoration / shell exception (main `.app-shell`, Settings dialog, non-product webviews).
- Every undecorated product window mounts `WindowTitlebar` focus chrome (elevated strip, accent hairline, focused/blurred states).
- Every undecorated standalone window uses `.window-shell` (or a documented exception for main).
- Brand, typography, spacing, menus, controls, focus, and status semantics are consistent in both themes.
- No business API or persisted data contract changes are included in the visual migration.
- All supported window sizes and DPI scales remain free from clipping, overlapping controls, and hover-induced layout shifts.
- Stacked undecorated windows keep a readable titlebar edge and drag target in light and dark themes.
