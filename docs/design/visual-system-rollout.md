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

## Phase 2: shared primitives and tokens

1. Generalize `MainTitlebar` into `WindowTitlebar` while preserving the tested drag threshold and double-click behavior.
2. Add semantic tokens for window chrome, popovers, control heights, content gutters, typography roles, status colors, focus, shadows, and separators.
3. Standardize menu triggers, menu rows, cascades, buttons, fields, badges, tabs, toggles, tables, disclosures, and empty/loading/error states.
4. Use Bahnschrift for operational hierarchy, Corbel/Segoe UI for readable UI text, and Cascadia Mono only for paths, versions, identifiers, and data.
5. Remove decorative gradients, hover reflow, local font overrides, arbitrary stacking values, and duplicated dark-theme patches.

## Phase 3: window migration

Migrate in risk-based groups, completing tests and visual review before moving to the next group.

1. Utility windows: Runtime Log, Queue Status, Connector Debug, and Connector Runtimes.
2. Operational windows: Scheduler, Plans, Single Videos, and Import.
3. Editing windows: Profile Editor, Batch Editor, Accounts, and Settings.
4. Media-heavy windows: Profile View and any lightbox-owned auxiliary window.

For each window:

- set `decorations: false` only after the shared shell is mounted;
- grant only the Tauri window permissions actually used;
- provide an accessible title and minimize/maximize/close controls appropriate to that window;
- preserve the current minimum dimensions and make the content region the only scroll owner;
- verify that inputs, menus, tabs, and other interactive regions never initiate dragging;
- remove duplicate in-content titles or close buttons once the titlebar provides them;
- preserve commands, shortcuts, loading behavior, selection, and state restoration.

## Phase 4: Companion and cross-product consistency

1. Keep the Companion popup on the shared charcoal, cream, burnt-orange, teal, green, and red semantic palette.
2. Use the angular-trail symbol in the browser toolbar and a compact symbol plus wordmark in the popup header.
3. Align button hierarchy, focus treatment, status pills, panels, typography, and light/dark behavior with the desktop app.
4. Keep extension-specific density and native browser conventions; do not reproduce desktop window controls inside the popup.
5. Add a packaging check that rejects missing or stale Companion icon sizes.

## Phase 5: validation and rollout

- Unit-test window commands, maximized state, accessible names, drag isolation, menu keyboard behavior, Escape, and focus return.
- Test every window at its minimum/default size and maximized at Windows scaling of 100%, 125%, 150%, and 200%.
- Validate light and dark themes, long paths, large counts, localization-safe labels, overflow, and focus visibility.
- Confirm WCAG AA contrast and one clear primary action per context.
- Run Vitest, ESLint, TypeScript/Vite build, icon generation, Rust tests, Release build, NSIS packaging, and the Tauri smoke test.
- Migrate and review one window group per pull request so regressions are isolated and rollback remains straightforward.

## Completion criteria

- Every Tauri window uses the shared shell or has a documented native-decoration exception.
- Brand, typography, spacing, menus, controls, focus, and status semantics are consistent in both themes.
- No business API or persisted data contract changes are included in the visual migration.
- All supported window sizes and DPI scales remain free from clipping, overlapping controls, and hover-induced layout shifts.
