[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('pull_request', 'push', 'workflow_dispatch')]
    [string]$EventName,

    [string]$HeadRef = '',

    [string]$BaseSha = '',

    [string]$HeadSha = '',

    [string[]]$ChangedPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function New-Result([bool]$WindowsBuild, [string]$Reason, [string[]]$Paths) {
    [pscustomobject]@{
        windowsBuild = $WindowsBuild
        reason = $Reason
        changedPaths = @($Paths)
    }
}

if ($EventName -eq 'workflow_dispatch') {
    New-Result $true 'Manual CI dispatch always validates the Windows build.' @() |
        ConvertTo-Json -Compress
    exit 0
}

if ($EventName -eq 'push') {
    New-Result $false 'Pushes to main reuse pull-request validation; the official release performs the full build.' @() |
        ConvertTo-Json -Compress
    exit 0
}

$paths = @($ChangedPath | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
if ($paths.Count -eq 0) {
    if ([string]::IsNullOrWhiteSpace($BaseSha) -or [string]::IsNullOrWhiteSpace($HeadSha)) {
        New-Result $true 'The pull-request comparison SHAs are unavailable; building fail-closed.' @() |
            ConvertTo-Json -Compress
        exit 0
    }

    & git cat-file -e "$BaseSha^{commit}" 2>$null
    if ($LASTEXITCODE -ne 0) {
        New-Result $true 'The pull-request base commit is unavailable; building fail-closed.' @() |
            ConvertTo-Json -Compress
        exit 0
    }

    $paths = @(& git diff --name-only "$BaseSha...$HeadSha")
    if ($LASTEXITCODE -ne 0) {
        throw "Could not determine changed paths for $BaseSha...$HeadSha."
    }
}

$metadataAutomationBranch =
    $HeadRef -like 'release-please--*' -or
    $HeadRef -like 'automation/readme-release-*' -or
    $HeadRef -like 'sync/release-*'
$releaseMetadataPaths = @(
    '.release-please-manifest.json',
    'CHANGELOG.md',
    'README.md',
    'package-lock.json',
    'package.json',
    'src-tauri/Cargo.lock',
    'src-tauri/Cargo.toml',
    'src-tauri/tauri.conf.json'
)
$metadataOnly = $paths.Count -gt 0 -and
    @($paths | Where-Object { $_ -notin $releaseMetadataPaths }).Count -eq 0
if ($metadataAutomationBranch -and $metadataOnly) {
    New-Result $false "The automation branch '$HeadRef' contains release metadata only." $paths |
        ConvertTo-Json -Compress
    exit 0
}

$buildPatterns = @(
    '^src/',
    '^src-tauri/',
    '^public/',
    '^connectors/',
    '^index\.html$',
    '^package(-lock)?\.json$',
    '^(vite|vitest)\.config\.',
    '^tsconfig[^/]*\.json$',
    '^Tools/(Build-NinjaCrawler|Package-NinjaCrawlerRelease|Test-NinjaCrawlerVersion)\.ps1$',
    '^\.github/workflows/(ci|release|cross-build-validation|cross-build-self-hosted)\.yml$'
)
$buildPaths = @($paths | Where-Object {
    $path = $_
    @($buildPatterns | Where-Object { $path -match $_ }).Count -gt 0
})

if ($buildPaths.Count -gt 0) {
    New-Result $true "Build-relevant paths changed: $($buildPaths -join ', ')" $paths |
        ConvertTo-Json -Compress
    exit 0
}

New-Result $false 'No application or Windows build inputs changed.' $paths |
    ConvertTo-Json -Compress
