Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$configPath = Join-Path $repoRoot "release-please-config.json"
$companionConfigPath = Join-Path $repoRoot "release-please-companion-config.json"
$workflowPath = Join-Path $repoRoot ".github\workflows\release-please.yml"
$ciWorkflowPath = Join-Path $repoRoot ".github\workflows\ci.yml"
$selfHostedWorkflowPath = Join-Path $repoRoot ".github\workflows\cross-build-self-hosted.yml"
$crossBuildValidationWorkflowPath = Join-Path $repoRoot ".github\workflows\cross-build-validation.yml"
$appReleaseWorkflowPath = Join-Path $repoRoot ".github\workflows\release.yml"
$companionReleaseWorkflowPath = Join-Path $repoRoot ".github\workflows\release-companion.yml"
$promotionWorkflowPath = Join-Path $repoRoot ".github\workflows\release-pr.yml"
$promoteMergeWorkflowPath = Join-Path $repoRoot ".github\workflows\promote-merge.yml"
$releaseBackSyncWorkflowPath = Join-Path $repoRoot ".github\workflows\release-back-sync.yml"
$cargoLockPath = Join-Path $repoRoot "src-tauri\Cargo.lock"
$buildScriptPath = Join-Path $repoRoot "Tools\Build-NinjaCrawler.ps1"
$versionTestPath = Join-Path $repoRoot "Tools\Test-NinjaCrawlerVersion.ps1"
$config = Get-Content -LiteralPath $configPath -Raw | ConvertFrom-Json
$companionConfig = Get-Content -LiteralPath $companionConfigPath -Raw | ConvertFrom-Json
$workflow = Get-Content -LiteralPath $workflowPath -Raw
$ciWorkflow = Get-Content -LiteralPath $ciWorkflowPath -Raw
$selfHostedWorkflow = Get-Content -LiteralPath $selfHostedWorkflowPath -Raw
$crossBuildValidationWorkflow = Get-Content -LiteralPath $crossBuildValidationWorkflowPath -Raw
$appReleaseWorkflow = Get-Content -LiteralPath $appReleaseWorkflowPath -Raw
$companionReleaseWorkflow = Get-Content -LiteralPath $companionReleaseWorkflowPath -Raw
$promotionWorkflow = Get-Content -LiteralPath $promotionWorkflowPath -Raw
$promoteMergeWorkflow = Get-Content -LiteralPath $promoteMergeWorkflowPath -Raw
$releaseBackSyncWorkflow = Get-Content -LiteralPath $releaseBackSyncWorkflowPath -Raw
$cargoLock = Get-Content -LiteralPath $cargoLockPath -Raw
$buildScript = Get-Content -LiteralPath $buildScriptPath -Raw
$versionTest = Get-Content -LiteralPath $versionTestPath -Raw

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
    "--label 'autorelease: pending'",
    'Detect unpublished draft releases',
    'Test-UnpublishedDraft',
    'Quarantine pre-tag release pull requests',
    "always() && steps.recovery.outputs.requested != 'true'",
    'Release Please remains paused until publication or explicit recovery completes',
    "steps.unpublished.outputs.app != 'true'",
    "steps.unpublished.outputs.companion != 'true'"
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

$e2eSections = [regex]::Split($crossBuildValidationWorkflow, '(?m)^  publish-and-verify:\s*$')
if ($e2eSections.Count -ne 2) {
    throw "Release E2E workflow must have one isolated publish-and-verify job."
}
foreach ($forbiddenFragment in @('contents: write', 'runs-on: ubuntu-latest', 'secrets.')) {
    if ($e2eSections[0].Contains($forbiddenFragment)) {
        throw "Release E2E build job contains privileged hosted-publish behavior: $forbiddenFragment"
    }
}

foreach ($requiredFragment in @(
    'Stage versioned portable',
    'NinjaCrawler-$version-windows-x64-portable.exe',
    'steps.portable.outputs.path',
    "github.event_name == 'pull_request' || github.ref == 'refs/heads/main'",
    '["OWNER","MEMBER","COLLABORATOR"]',
    'github.event.pull_request.author_association',
    '["self-hosted","proxmox-lxc","crossbuild","mode-ephemeral"]',
    '["ubuntu-latest"]',
    'CARGO_TARGET_DIR=/cache/target/ninjacrawler-ci'
    'Validate app version manifests'
    'Tools/Test-NinjaCrawlerVersion.ps1'
)) {
    if (-not $ciWorkflow.Contains($requiredFragment)) {
        throw "CI is missing versioned portable artifact staging: $requiredFragment"
    }
}

$appReleaseSections = [regex]::Split($appReleaseWorkflow, '(?m)^  publish:\s*$')
if ($appReleaseSections.Count -ne 2) {
    throw "App release workflow must have one isolated publish job."
}
foreach ($forbiddenFragment in @('contents: write', 'runs-on: ubuntu-latest', 'actions/cache@', 'secrets.')) {
    if ($appReleaseSections[0].Contains($forbiddenFragment)) {
        throw "App release build job contains privileged or hosted-only behavior: $forbiddenFragment"
    }
}

foreach ($requiredFragment in @(
    'workflow_dispatch:',
    'runs-on: [self-hosted, proxmox-lxc, crossbuild, mode-ephemeral]',
    "github.ref == 'refs/heads/main'",
    'persist-credentials: false',
    'cancel-in-progress: false',
    'CARGO_TARGET_DIR: /cache/target/ninjacrawler-smoke',
    'Tools/Build-NinjaCrawler.ps1',
    '-PortableOnly',
    'test ! -e connectors/bootstrap',
    'IMAGE_FILE_MACHINE_AMD64'
)) {
    if (-not $selfHostedWorkflow.Contains($requiredFragment)) {
        throw "Self-hosted validation workflow is missing a safety invariant: $requiredFragment"
    }
}

foreach ($requiredFragment in @(
    'workflow_dispatch:',
    'runs-on: [self-hosted, proxmox-lxc, crossbuild, mode-ephemeral]',
    'persist-credentials: false',
    'cancel-in-progress: false',
    'CARGO_TARGET_DIR: /cache/target/ninjacrawler-release-e2e',
    'Tools/Build-NinjaCrawler.ps1',
    'libayatana-appindicator3-dev',
    'Publish and verify isolated prerelease',
    "inputs.publish_test_release && github.ref == 'refs/heads/main'",
    'runs-on: ubuntu-latest',
    'contents: write',
    'gh release create "$TEST_TAG"',
    'gh release download "$TEST_TAG"',
    'gh release delete "$TEST_TAG"',
    '--cleanup-tag',
    '(cd downloaded && sha256sum --check SHA256SUMS.txt)'
    'release-e2e-v$version-'
    'Tools/Test-NinjaCrawlerVersion.ps1'
)) {
    if (-not $crossBuildValidationWorkflow.Contains($requiredFragment)) {
        throw "Full cross-build validation workflow is missing a safety invariant: $requiredFragment"
    }
}

foreach ($forbiddenFragment in @(
    'pull_request:',
    '-PortableOnly',
    'secrets.',
    'sudo apt-get'
)) {
    if ($crossBuildValidationWorkflow.Contains($forbiddenFragment)) {
        throw "Full cross-build validation workflow contains forbidden behavior: $forbiddenFragment"
    }
}

foreach ($forbiddenFragment in @(
    'pull_request:',
    'Prepare-ConnectorBootstrap.ps1',
    'secrets.',
    'contents: write'
)) {
    if ($selfHostedWorkflow.Contains($forbiddenFragment)) {
        throw "Self-hosted validation workflow contains forbidden behavior: $forbiddenFragment"
    }
}

if (-not $appReleaseWorkflow.Contains('startswith("release-please--")')) {
    throw "App release re-anchoring must regenerate every PR that shares the release manifest."
}
if (-not $appReleaseWorkflow.Contains('Reconcile recovered release PR label')) {
    throw "App release recovery must reconcile the pending release PR label."
}
if (-not $appReleaseWorkflow.Contains('Tools/Test-NinjaCrawlerVersion.ps1 -ExpectedVersion $version')) {
    throw "App release validation must enforce the shared version contract."
}

foreach ($requiredFragment in @(
    'src-tauri/Cargo.lock'
    'src-tauri/Cargo.toml'
    'src-tauri/tauri.conf.json'
    'package.json'
)) {
    if (-not $versionTest.Contains($requiredFragment)) {
        throw "Version contract test is missing a release manifest: $requiredFragment"
    }
}

if (-not $buildScript.Contains('link-arg=/ignore:4099') -or
    -not $buildScript.Contains('CrossLinkerPolicy=ignore-msvc-runtime-pdb-warning-4099')) {
    throw "Linux MSVC cross-builds must suppress the unavailable runtime PDB warning."
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
    'gh pr close "$existing"',
    'Refusing to promote $HEAD version $head_version over newer $BASE version $base_version.',
    'sort -V'
)) {
    if (-not $promotionWorkflow.Contains($requiredFragment)) {
        throw "Promotion workflow does not close merge-only release PRs: $requiredFragment"
    }
}

if ($appReleaseWorkflow.Contains('sudo apt-get')) {
    throw "The immutable JIT release job must not mutate the golden image with apt."
}

foreach ($requiredFragment in @(
    'runs-on: [self-hosted, proxmox-lxc, crossbuild, mode-ephemeral]',
    'CARGO_TARGET_DIR: /cache/target/ninjacrawler-release',
    'Verify self-hosted golden toolchain',
    'libayatana-appindicator3-dev',
    'publish:',
    'runs-on: ubuntu-latest',
    'contents: write'
)) {
    if (-not $appReleaseWorkflow.Contains($requiredFragment)) {
        throw "App release workflow is missing the split self-hosted build/hosted publish contract: $requiredFragment"
    }
}

foreach ($requiredFragment in @(
    "EVENT_PR_NUMBER: `${{ github.event.pull_request.number || '' }}",
    'if [ "$EVENT_NAME" = "pull_request" ]; then',
    'pr="$EVENT_PR_NUMBER"',
    'gh pr merge "$pr" --repo "$REPO" --merge'
)) {
    if (-not $promoteMergeWorkflow.Contains($requiredFragment)) {
        throw "Promote merge workflow is missing a label-event invariant: $requiredFragment"
    }
}

if ($promoteMergeWorkflow.Contains("--jq '.[0].number' || true")) {
    throw "Promote merge workflow must not hide PR lookup failures."
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
