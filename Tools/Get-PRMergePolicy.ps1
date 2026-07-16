param(
    [Parameter(Mandatory = $true)]
    [string]$Base,

    [Parameter(Mandatory = $true)]
    [string]$Head,

    [string]$Title = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Normalize-Ref([string]$value) {
    return ($value -replace '^refs/heads/', '').Trim()
}

$baseRef = Normalize-Ref $Base
$headRef = Normalize-Ref $Head

if ([string]::IsNullOrWhiteSpace($baseRef) -or [string]::IsNullOrWhiteSpace($headRef)) {
    throw "Base and Head refs are required."
}

# Policy matrix (source of truth for labels, sticky comments, and rulesets):
# - develop → main (promote): merge commit — preserves feat/fix for release-please
#   (incident 0.10.0: squashed promotes #60/#63 stalled the release).
# - sync/release-* → develop (back-sync): merge commit — published tag must become
#   an ancestor of develop.
# - release-please--* → main: merge commit — keep release PR version commits intact.
# - automation/* → main: merge commit — aligns with main merge-only ruleset.
# - * → develop (features, dependabot, etc.): squash — linear feature history.
# - default → squash when targeting develop-like integration; merge when targeting main.

$result = $null

if ($baseRef -eq 'main' -and $headRef -eq 'develop') {
    $result = [ordered]@{
        method = 'merge-commit'
        label = 'merge:merge-commit'
        critical = $true
        reason = 'develop → main promotion must preserve individual Conventional Commits for Release Please. Squashing collapses them into a single chore commit (stalled 0.10.0 after promotes #60/#63).'
        how = 'Do **not** use the green merge button. Add the `promote` label (or run the promote-merge workflow). CI must pass; the workflow merges with `gh pr merge --merge`.'
    }
}
elseif ($baseRef -eq 'develop' -and $headRef -match '^sync/release-') {
    $result = [ordered]@{
        method = 'merge-commit'
        label = 'merge:merge-commit'
        critical = $true
        reason = 'Release back-sync must create a merge commit so the published tag becomes an ancestor of develop (required by release-back-sync ancestry checks).'
        how = 'The release-back-sync workflow merges with `gh pr merge --merge`. Do not squash this PR manually.'
    }
}
elseif ($baseRef -eq 'main' -and $headRef -match '^release-please--') {
    $result = [ordered]@{
        method = 'merge-commit'
        label = 'merge:merge-commit'
        critical = $true
        reason = 'Release Please PRs should land as merge commits so version-bump commits stay intact on main.'
        how = 'Use **Create a merge commit** (or merge via the release automation). Do not squash.'
    }
}
elseif ($baseRef -eq 'main' -and $headRef -match '^automation/') {
    $result = [ordered]@{
        method = 'merge-commit'
        label = 'merge:merge-commit'
        critical = $false
        reason = 'Automation PRs into main use merge commits so the main branch ruleset can require merge-only and avoid the squash default foot-gun.'
        how = 'Automation merges with `gh pr merge --merge`. Prefer not to merge this PR manually.'
    }
}
elseif ($baseRef -eq 'develop') {
    $result = [ordered]@{
        method = 'squash'
        label = 'merge:squash'
        critical = $false
        reason = 'Feature and maintenance PRs into develop should squash to one Conventional Commit on the integration branch.'
        how = 'Use **Squash and merge**. Keep the commit message in Conventional Commit form (`feat:`, `fix:`, `ci:`, …).'
    }
}
elseif ($baseRef -eq 'main') {
    $result = [ordered]@{
        method = 'merge-commit'
        label = 'merge:merge-commit'
        critical = $false
        reason = 'PRs into main default to merge commits (main ruleset allows merge only).'
        how = 'Use **Create a merge commit**. Do not squash into main.'
    }
}
else {
    $result = [ordered]@{
        method = 'squash'
        label = 'merge:squash'
        critical = $false
        reason = "No specialized rule for $headRef → $baseRef; default to squash."
        how = 'Use **Squash and merge** unless release automation documents otherwise.'
    }
}

$result.base = $baseRef
$result.head = $headRef
if (-not [string]::IsNullOrWhiteSpace($Title)) {
    $result.title = $Title
}

$result | ConvertTo-Json -Compress
