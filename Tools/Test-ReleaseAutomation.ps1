Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$configPath = Join-Path $repoRoot "release-please-config.json"
$workflowPath = Join-Path $repoRoot ".github\workflows\release-please.yml"
$appReleaseWorkflowPath = Join-Path $repoRoot ".github\workflows\release.yml"
$companionReleaseWorkflowPath = Join-Path $repoRoot ".github\workflows\release-companion.yml"
$config = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
$workflow = Get-Content -LiteralPath $workflowPath -Raw
$appReleaseWorkflow = Get-Content -LiteralPath $appReleaseWorkflowPath -Raw
$companionReleaseWorkflow = Get-Content -LiteralPath $companionReleaseWorkflowPath -Raw

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

if (-not $appReleaseWorkflow.Contains('endswith("--components--ninjacrawler")')) {
    throw "App release re-anchoring must only close the app release PR."
}
if (-not $appReleaseWorkflow.Contains('Reconcile recovered release PR label')) {
    throw "App release recovery must reconcile the pending release PR label."
}

foreach ($requiredFragment in @(
    'Re-anchor Companion release-please',
    'Reconcile recovered Companion release PR label',
    'endswith("--components--companion")',
    'gh workflow run release-please.yml'
)) {
    if (-not $companionReleaseWorkflow.Contains($requiredFragment)) {
        throw "Companion release workflow is missing isolated re-anchoring: $requiredFragment"
    }
}

Write-Host "Release automation configuration tests passed."
