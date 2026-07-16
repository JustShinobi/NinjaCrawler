Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptPath = Join-Path $PSScriptRoot 'Get-CIBuildImpact.ps1'

function Assert-Decision {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [bool]$ExpectedWindows,
        [Parameter(Mandatory = $true)]
        [bool]$ExpectedQuality,
        [Parameter(Mandatory = $true)]
        [string]$EventName,
        [string]$HeadRef = '',
        [string[]]$ChangedPath = @()
    )

    $result = & $scriptPath `
        -EventName $EventName `
        -HeadRef $HeadRef `
        -ChangedPath $ChangedPath |
        ConvertFrom-Json
    if ($result.windowsBuild -ne $ExpectedWindows) {
        throw "$Name expected windowsBuild=$ExpectedWindows, got $($result.windowsBuild): $($result.reason)"
    }
    if ($result.quality -ne $ExpectedQuality) {
        throw "$Name expected quality=$ExpectedQuality, got $($result.quality): $($result.qualityReason)"
    }
}

Assert-Decision -Name 'manual dispatch' -ExpectedWindows $true -ExpectedQuality $true -EventName workflow_dispatch
Assert-Decision -Name 'main push' -ExpectedWindows $false -ExpectedQuality $false -EventName push
# Backend-only PRs still need the Windows cross-build, but not the hosted npm job.
Assert-Decision -Name 'Rust source' -ExpectedWindows $true -ExpectedQuality $false -EventName pull_request `
    -HeadRef 'fix/runtime' -ChangedPath 'src-tauri/src/main.rs'
Assert-Decision -Name 'frontend source' -ExpectedWindows $true -ExpectedQuality $true -EventName pull_request `
    -HeadRef 'feat/ui' -ChangedPath 'src/App.tsx'
Assert-Decision -Name 'Companion extension' -ExpectedWindows $false -ExpectedQuality $true -EventName pull_request `
    -HeadRef 'fix/companion' -ChangedPath 'NinjaCrawler.Companion/src/core.js'
Assert-Decision -Name 'dependency update' -ExpectedWindows $true -ExpectedQuality $true -EventName pull_request `
    -HeadRef 'dependabot/npm' -ChangedPath 'package-lock.json'
Assert-Decision -Name 'build tooling' -ExpectedWindows $true -ExpectedQuality $true -EventName pull_request `
    -HeadRef 'ci/build' -ChangedPath 'Tools/Build-NinjaCrawler.ps1'
Assert-Decision -Name 'CI workflow' -ExpectedWindows $true -ExpectedQuality $true -EventName pull_request `
    -HeadRef 'ci/build' -ChangedPath '.github/workflows/ci.yml'
Assert-Decision -Name 'documentation only' -ExpectedWindows $false -ExpectedQuality $false -EventName pull_request `
    -HeadRef 'docs/runbook' -ChangedPath 'docs/linux-cross-build.md'
Assert-Decision -Name 'docs image only' -ExpectedWindows $false -ExpectedQuality $false -EventName pull_request `
    -HeadRef 'docs/readme-image' -ChangedPath 'assets/docs/workspace-preview.png'
Assert-Decision -Name 'README structure' -ExpectedWindows $false -ExpectedQuality $true -EventName pull_request `
    -HeadRef 'docs/readme' -ChangedPath 'README.md'
Assert-Decision -Name 'README automation' -ExpectedWindows $false -ExpectedQuality $true -EventName pull_request `
    -HeadRef 'automation/readme-release-v1.2.3' -ChangedPath 'README.md'
Assert-Decision -Name 'Release Please changelog only' -ExpectedWindows $false -ExpectedQuality $false -EventName pull_request `
    -HeadRef 'release-please--branches--main--components--ninjacrawler' `
    -ChangedPath 'CHANGELOG.md'
Assert-Decision -Name 'Release Please metadata with package' -ExpectedWindows $false -ExpectedQuality $true -EventName pull_request `
    -HeadRef 'release-please--branches--main--components--ninjacrawler' `
    -ChangedPath @('CHANGELOG.md', 'package.json', 'src-tauri/Cargo.toml')
Assert-Decision -Name 'release back-sync metadata' -ExpectedWindows $false -ExpectedQuality $true -EventName pull_request `
    -HeadRef 'sync/release-v1.2.3' `
    -ChangedPath @('README.md', 'package-lock.json', 'src-tauri/Cargo.lock')
Assert-Decision -Name 'automation branch with source' -ExpectedWindows $true -ExpectedQuality $false -EventName pull_request `
    -HeadRef 'release-please--branches--main--components--ninjacrawler' `
    -ChangedPath @('CHANGELOG.md', 'src-tauri/src/main.rs')
Assert-Decision -Name 'merge-policy workflow only' -ExpectedWindows $false -ExpectedQuality $true -EventName pull_request `
    -HeadRef 'ci/merge-policy' -ChangedPath '.github/workflows/merge-policy.yml'

Write-Host 'CI build-impact tests passed.'
