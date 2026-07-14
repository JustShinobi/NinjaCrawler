Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$configPath = Join-Path $repoRoot "release-please-config.json"
$companionConfigPath = Join-Path $repoRoot "release-please-companion-config.json"
$workflowPath = Join-Path $repoRoot ".github\workflows\release-please.yml"
$ciWorkflowPath = Join-Path $repoRoot ".github\workflows\ci.yml"
$appReleaseWorkflowPath = Join-Path $repoRoot ".github\workflows\release.yml"
$companionReleaseWorkflowPath = Join-Path $repoRoot ".github\workflows\release-companion.yml"
$promotionWorkflowPath = Join-Path $repoRoot ".github\workflows\release-pr.yml"
$releaseBackSyncWorkflowPath = Join-Path $repoRoot ".github\workflows\release-back-sync.yml"
$cargoLockPath = Join-Path $repoRoot "src-tauri\Cargo.lock"
$config = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
$companionConfig = Get-Content -LiteralPath $companionConfigPath -Raw | ConvertFrom-Json
$workflow = Get-Content -LiteralPath $workflowPath -Raw
$ciWorkflow = Get-Content -LiteralPath $ciWorkflowPath -Raw
$appReleaseWorkflow = Get-Content -LiteralPath $appReleaseWorkflowPath -Raw
$companionReleaseWorkflow = Get-Content -LiteralPath $companionReleaseWorkflowPath -Raw
$promotionWorkflow = Get-Content -LiteralPath $promotionWorkflowPath -Raw
$releaseBackSyncWorkflow = Get-Content -LiteralPath $releaseBackSyncWorkflowPath -Raw
$cargoLock = Get-Content -LiteralPath $cargoLockPath -Raw

foreach ($releaseConfig in @($config, $companionConfig)) {
    if ($releaseConfig.'separate-pull-requests' -ne $true) {
        throw "Release Please packages must use separate pull requests."
    }
}

foreach ($packageEntry in @(
    @{ Config = $config; Path = '.' },
    @{ Config = $companionConfig; Path = 'NinjaCrawler.Companion' }
)) {
    $packagePath = $packageEntry.Path
    $package = $packageEntry.Config.packages.$packagePath
    $pattern = $package.'pull-request-title-pattern'
    if (-not $pattern.Contains('${component}') -or -not $pattern.Contains('${version}')) {
        throw "Release PR title for '$packagePath' must preserve component and version."
    }
}

if (@($config.packages.PSObject.Properties).Count -ne 1 -or
    -not $config.packages.PSObject.Properties['.']) {
    throw "The app Release Please config must contain only the root package."
}
if (@($companionConfig.packages.PSObject.Properties).Count -ne 1 -or
    -not $companionConfig.packages.PSObject.Properties['NinjaCrawler.Companion']) {
    throw "The Companion Release Please config must contain only the Companion package."
}
if ('NinjaCrawler.Companion' -notin @($config.packages.'.'.'exclude-paths')) {
    throw "The root package must exclude Companion commits from app releases."
}
$appSections = $config.'changelog-sections' | ConvertTo-Json -Compress
$companionSections = $companionConfig.'changelog-sections' | ConvertTo-Json -Compress
if ($appSections -ne $companionSections) {
    throw "App and Companion Release Please configs must use the same changelog sections."
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

foreach ($requiredFragment in @(
    'Detect Companion release intent',
    'id: app_release',
    'config-file: release-please-config.json',
    'id: companion_release',
    'config-file: release-please-companion-config.json',
    'Close metadata-only Companion release pull requests',
    'Test-ReleaseCandidateIntegrity.ps1',
    'steps.companion_intent.outputs.eligible',
    "git tag --list 'companion-v*' --sort=-version:refname"
)) {
    if (-not $workflow.Contains($requiredFragment)) {
        throw "Release Please workflow is missing package release-intent isolation: $requiredFragment"
    }
}

foreach ($requiredFragment in @(
    'pull-requests: read',
    'Validate Companion release candidate',
    'Test-ReleaseCandidateIntegrity.ps1',
    'Test-ReleaseCandidateIntegrity.Tests.ps1'
)) {
    if (-not $ciWorkflow.Contains($requiredFragment)) {
        throw "CI is missing release candidate integrity coverage: $requiredFragment"
    }
}

foreach ($requiredFragment in @(
    'Stage versioned portable',
    'NinjaCrawler-$version-windows-x64-portable.exe',
    'steps.portable.outputs.path'
)) {
    if (-not $ciWorkflow.Contains($requiredFragment)) {
        throw "CI is missing versioned portable artifact staging: $requiredFragment"
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

if (-not $appReleaseWorkflow.Contains('gh workflow run release-back-sync.yml') -or
    -not $appReleaseWorkflow.Contains('-f tag=${{ needs.build.outputs.tag }}')) {
    throw "The app release must back-sync the tag produced by its isolated build job."
}
if (-not $companionReleaseWorkflow.Contains('gh workflow run release-back-sync.yml') -or
    -not $companionReleaseWorkflow.Contains('-f tag=${{ steps.version.outputs.tag }}')) {
    throw "The Companion release must back-sync its published tag."
}

foreach ($requiredFragment in @(
    '^(companion-)?v[0-9]+\.[0-9]+\.[0-9]+$',
    'git merge-base --is-ancestor',
    '--base "$BASE" --head "$branch"',
    '--merge --delete-branch',
    '--subject "chore(release): sync published version back to develop"'
)) {
    if (-not $releaseBackSyncWorkflow.Contains($requiredFragment)) {
        throw "Release back-sync workflow is missing a safety invariant: $requiredFragment"
    }
}

Write-Host "Release automation configuration tests passed."
