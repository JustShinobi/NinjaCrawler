Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$configPath = Join-Path $repoRoot "release-please-config.json"
$workflowPath = Join-Path $repoRoot ".github\workflows\release-please.yml"
$appReleaseWorkflowPath = Join-Path $repoRoot ".github\workflows\release.yml"
$companionReleaseWorkflowPath = Join-Path $repoRoot ".github\workflows\release-companion.yml"
$promotionWorkflowPath = Join-Path $repoRoot ".github\workflows\release-pr.yml"
$releaseBackSyncWorkflowPath = Join-Path $repoRoot ".github\workflows\release-back-sync.yml"
$cargoLockPath = Join-Path $repoRoot "src-tauri\Cargo.lock"
$config = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
$workflow = Get-Content -LiteralPath $workflowPath -Raw
$appReleaseWorkflow = Get-Content -LiteralPath $appReleaseWorkflowPath -Raw
$companionReleaseWorkflow = Get-Content -LiteralPath $companionReleaseWorkflowPath -Raw
$promotionWorkflow = Get-Content -LiteralPath $promotionWorkflowPath -Raw
$releaseBackSyncWorkflow = Get-Content -LiteralPath $releaseBackSyncWorkflowPath -Raw
$cargoLock = Get-Content -LiteralPath $cargoLockPath -Raw

if ($config.'separate-pull-requests' -ne $true) {
    throw "Release Please packages must use separate pull requests."
}

foreach ($packagePath in @('.', 'NinjaCrawler.Companion')) {
    $package = $config.packages.$packagePath
    $pattern = $package.'pull-request-title-pattern'
    if (-not $pattern.Contains('${component}') -or -not $pattern.Contains('${version}')) {
        throw "Release PR title for '$packagePath' must preserve component and version."
    }
}

$cargoLockUpdater = @($config.packages.'.'.'extra-files') | Where-Object {
    $_.PSObject.Properties['type'] -and
    $_.type -eq 'generic' -and
    $_.path -eq 'src-tauri/Cargo.lock'
}
if (-not $cargoLockUpdater) {
    throw "Release Please must update the Cargo lockfile version."
}
if (-not $cargoLock.Contains('x-release-please-version')) {
    throw "Cargo.lock must mark the application package version for Release Please."
}

foreach ($requiredFragment in @(
    'recover_package:',
    'Recover untagged merged release',
    'GH_TOKEN: ${{ secrets.GH_TOKEN }}',
    'Reject untagged merged release PRs',
    "--label 'autorelease: pending'"
)) {
    if (-not $workflow.Contains($requiredFragment)) {
        throw "Release workflow is missing recovery safeguard: $requiredFragment"
    }
}

if (-not $appReleaseWorkflow.Contains('startswith("release-please--")')) {
    throw "App release re-anchoring must regenerate every PR that shares the release manifest."
}
if (-not $appReleaseWorkflow.Contains('Reconcile recovered release PR label')) {
    throw "App release recovery must reconcile the pending release PR label."
}
if (-not $appReleaseWorkflow.Contains("'src-tauri/Cargo.lock' = `$cargoLockVersion")) {
    throw "App release validation must reject a stale Cargo lockfile version."
}

foreach ($requiredFragment in @(
    'NINJACRAWLER_RELEASE_BUILD:',
    'NINJACRAWLER_RELEASE_VERSION:',
    'NINJACRAWLER_BUILD_SHA:'
)) {
    if (-not $appReleaseWorkflow.Contains($requiredFragment)) {
        throw "App release workflow is missing official build identity wiring: $requiredFragment"
    }
}

foreach ($requiredFragment in @(
    'Re-anchor Companion release-please',
    'Reconcile recovered Companion release PR label',
    'startswith("release-please--")',
    'gh workflow run release-please.yml'
)) {
    if (-not $companionReleaseWorkflow.Contains($requiredFragment)) {
        throw "Companion release workflow is missing shared-manifest re-anchoring: $requiredFragment"
    }
}

foreach ($requiredFragment in @(
    'git log --no-merges --oneline',
    'Closing empty release PR',
    'gh pr close "$existing"'
)) {
    if (-not $promotionWorkflow.Contains($requiredFragment)) {
        throw "Promotion workflow does not close merge-only release PRs: $requiredFragment"
    }
}

foreach ($releaseWorkflow in @($appReleaseWorkflow, $companionReleaseWorkflow)) {
    foreach ($requiredFragment in @(
        'gh workflow run release-back-sync.yml',
        '-f tag=${{ steps.version.outputs.tag }}'
    )) {
        if (-not $releaseWorkflow.Contains($requiredFragment)) {
            throw "Every package release must trigger branch back-sync: $requiredFragment"
        }
    }
}

foreach ($requiredFragment in @(
    '^(companion-)?v[0-9]+\.[0-9]+\.[0-9]+$',
    'git merge-base --is-ancestor',
    '--base "$BASE" --head "$branch"',
    '--merge --delete-branch'
)) {
    if (-not $releaseBackSyncWorkflow.Contains($requiredFragment)) {
        throw "Release back-sync workflow is missing a safety invariant: $requiredFragment"
    }
}

Write-Host "Release automation configuration tests passed."
