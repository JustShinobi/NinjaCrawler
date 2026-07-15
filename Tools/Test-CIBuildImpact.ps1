Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$scriptPath = Join-Path $PSScriptRoot 'Get-CIBuildImpact.ps1'

function Assert-Decision {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [bool]$Expected,
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
    if ($result.windowsBuild -ne $Expected) {
        throw "$Name expected windowsBuild=$Expected, got $($result.windowsBuild): $($result.reason)"
    }
}

Assert-Decision -Name 'manual dispatch' -Expected $true -EventName workflow_dispatch
Assert-Decision -Name 'main push' -Expected $false -EventName push
Assert-Decision -Name 'Rust source' -Expected $true -EventName pull_request `
    -HeadRef 'fix/runtime' -ChangedPath 'src-tauri/src/main.rs'
Assert-Decision -Name 'frontend source' -Expected $true -EventName pull_request `
    -HeadRef 'feat/ui' -ChangedPath 'src/App.tsx'
Assert-Decision -Name 'dependency update' -Expected $true -EventName pull_request `
    -HeadRef 'dependabot/npm' -ChangedPath 'package-lock.json'
Assert-Decision -Name 'build tooling' -Expected $true -EventName pull_request `
    -HeadRef 'ci/build' -ChangedPath 'Tools/Build-NinjaCrawler.ps1'
Assert-Decision -Name 'CI workflow' -Expected $true -EventName pull_request `
    -HeadRef 'ci/build' -ChangedPath '.github/workflows/ci.yml'
Assert-Decision -Name 'documentation only' -Expected $false -EventName pull_request `
    -HeadRef 'docs/runbook' -ChangedPath 'docs/linux-cross-build.md'
Assert-Decision -Name 'README automation' -Expected $false -EventName pull_request `
    -HeadRef 'automation/readme-release-v1.2.3' -ChangedPath 'README.md'
Assert-Decision -Name 'Release Please metadata' -Expected $false -EventName pull_request `
    -HeadRef 'release-please--branches--main--components--ninjacrawler' `
    -ChangedPath @('CHANGELOG.md', 'package.json', 'src-tauri/Cargo.toml')
Assert-Decision -Name 'release back-sync metadata' -Expected $false -EventName pull_request `
    -HeadRef 'sync/release-v1.2.3' `
    -ChangedPath @('README.md', 'package-lock.json', 'src-tauri/Cargo.lock')
Assert-Decision -Name 'automation branch with source' -Expected $true -EventName pull_request `
    -HeadRef 'release-please--branches--main--components--ninjacrawler' `
    -ChangedPath @('CHANGELOG.md', 'src-tauri/src/main.rs')

Write-Host 'CI build-impact tests passed.'
