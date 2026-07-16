# Pull request merge policy

## Why this exists

The repository default green button prefers **Squash and merge**. That is correct
for feature work into `develop`, but it is a foot-gun for release paths:

1. **Promote `develop` → `main`** must be a **merge commit**. Squashing collapses
   every `feat`/`fix` into one `chore: promote` commit, so Release Please reports
   *no user facing commits* and skips the release. This stalled **0.10.0** after
   squashed promotes #60/#63; recovery was a merge-commit re-promote (#67) and
   the `promote-merge` workflow (#70).
2. **Release back-sync** (`sync/release-*` → `develop`) must be a **merge
   commit** so the published tag becomes an ancestor of `develop`.

## Policy matrix

Source of truth: `Tools/Get-PRMergePolicy.ps1`.

| PR | Method | How |
| --- | --- | --- |
| `develop` → `main` | merge commit | Label `promote` (workflow `promote-merge`) |
| `sync/release-*` → `develop` | merge commit | `release-back-sync` uses `--merge` |
| `release-please--*` → `main` | merge commit | Create a merge commit |
| `automation/*` → `main` | merge commit | Automation uses `--merge` |
| `*` → `develop` (features, Dependabot, …) | squash | Squash and merge |
| other → `main` | merge commit | Create a merge commit |

## Layers

1. **Policy script** — classifies every PR.
2. **Automation** — `promote-merge` and `release-back-sync` always pass `--merge`.
3. **Merge policy bot** — workflow `merge-policy.yml` applies labels
   `merge:squash` / `merge:merge-commit` and a sticky PR comment.
4. **Rulesets** (via `Tools/Apply-MergePolicyRulesets.ps1`):
   - `develop-protection`: allows **squash** and **merge** (features squash;
     back-sync merges).
   - `main-protection`: allows **merge only** (no squash into `main`).
   - Admin repository role may bypass in emergencies.

## Applying rulesets

```powershell
# Preview
pwsh -File Tools/Apply-MergePolicyRulesets.ps1 -DryRun

# Apply to the current gh repository
pwsh -File Tools/Apply-MergePolicyRulesets.ps1
```

Repository flags are set to `allow_merge_commit=true`, `allow_squash_merge=true`
(needed for develop), `allow_rebase_merge=false`.

## Tests

```powershell
pwsh -File Tools/Test-PRMergePolicy.ps1
```
