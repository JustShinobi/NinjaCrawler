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

function New-Result {
    param(
        [bool]$WindowsBuild,
        [bool]$Quality,
        [string]$Reason,
        [string[]]$Paths,
        [string]$QualityReason = ''
    )

    [pscustomobject]@{
        windowsBuild = $WindowsBuild
        quality = $Quality
        reason = $Reason
        qualityReason = if ([string]::IsNullOrWhiteSpace($QualityReason)) { $Reason } else { $QualityReason }
        changedPaths = @($Paths)
    }
}

function Test-PathMatches {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string[]]$Patterns
    )

    return @($Patterns | Where-Object { $Path -match $_ }).Count -gt 0
}

# Portable / Rust / connector inputs that force a Windows cross-build.
$script:WindowsPatterns = @(
    '^src/',
    '^src-tauri/',
    '^public/',
    '^connectors/',
    '^index\.html$',
    '^package(-lock)?\.json$',
    '^(vite|vitest)\.config\.',
    '^tsconfig[^/]*\.json$',
    '^Tools/(Build-NinjaCrawler|Get-CIBuildImpact|Get-CIInputFingerprint|Get-CIPromotionValidation|Package-NinjaCrawlerRelease|Test-NinjaCrawlerVersion)\.ps1$',
    '^\.github/workflows/(ci|release|cross-build-validation|cross-build-self-hosted)\.yml$'
)

# Hosted "Frontend quality" job inputs: npm lint/test/build plus Companion and
# PowerShell release/CI regression suites that live in that job today.
$script:QualityPatterns = @(
    '^src/',
    '^NinjaCrawler\.Companion/',
    '^public/',
    '^connectors/',
    '^index\.html$',
    '^package(-lock)?\.json$',
    '^(vite|vitest|eslint)\.config\.',
    '^tsconfig[^/]*\.json$',
    '^Tools/',
    '^\.github/',
    '^README\.md$',
    '^release-please.*\.json$',
    '^\.release-please-manifest\.json$'
)

if ($EventName -eq 'workflow_dispatch') {
    New-Result `
        -WindowsBuild $true `
        -Quality $true `
        -Reason 'Manual CI dispatch always validates Windows and hosted quality.' `
        -Paths @() |
        ConvertTo-Json -Compress
    exit 0
}

if ($EventName -eq 'push') {
    New-Result `
        -WindowsBuild $false `
        -Quality $false `
        -Reason 'Pushes to main reuse pull-request validation; the official release performs the full build.' `
        -Paths @() |
        ConvertTo-Json -Compress
    exit 0
}

$paths = @($ChangedPath | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
if ($paths.Count -eq 0) {
    if ([string]::IsNullOrWhiteSpace($BaseSha) -or [string]::IsNullOrWhiteSpace($HeadSha)) {
        New-Result `
            -WindowsBuild $true `
            -Quality $true `
            -Reason 'The pull-request comparison SHAs are unavailable; validating fail-closed.' `
            -Paths @() |
            ConvertTo-Json -Compress
        exit 0
    }

    & git cat-file -e "$BaseSha^{commit}" 2>$null
    if ($LASTEXITCODE -ne 0) {
        New-Result `
            -WindowsBuild $true `
            -Quality $true `
            -Reason 'The pull-request base commit is unavailable; validating fail-closed.' `
            -Paths @() |
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
    $HeadRef -like 'automation/readme-companion-*' -or
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

$windowsPaths = @($paths | Where-Object { Test-PathMatches -Path $_ -Patterns $script:WindowsPatterns })
$qualityPaths = @($paths | Where-Object { Test-PathMatches -Path $_ -Patterns $script:QualityPatterns })

if ($metadataAutomationBranch -and $metadataOnly) {
    # Version bumps / README link rewrites must still exercise packaging + version
    # guards when they touch files the hosted job owns, but never force Windows.
    $quality = $qualityPaths.Count -gt 0
    $qualityReason = if ($quality) {
        "The automation branch '$HeadRef' changes release metadata that hosted quality validates: $($qualityPaths -join ', ')."
    } else {
        "The automation branch '$HeadRef' changes non-quality release metadata only."
    }
    New-Result `
        -WindowsBuild $false `
        -Quality $quality `
        -Reason "The automation branch '$HeadRef' contains release metadata only." `
        -QualityReason $qualityReason `
        -Paths $paths |
        ConvertTo-Json -Compress
    exit 0
}

$windows = $windowsPaths.Count -gt 0
$quality = $qualityPaths.Count -gt 0

$reason = if ($windows) {
    "Build-relevant paths changed: $($windowsPaths -join ', ')"
} elseif ($paths.Count -eq 0) {
    'No file changes in the pull-request comparison range.'
} else {
    'No application or Windows build inputs changed.'
}

$qualityReason = if ($quality) {
    "Hosted quality inputs changed: $($qualityPaths -join ', ')"
} elseif ($paths.Count -eq 0) {
    'No file changes in the pull-request comparison range.'
} else {
    'No hosted quality inputs changed.'
}

New-Result `
    -WindowsBuild $windows `
    -Quality $quality `
    -Reason $reason `
    -QualityReason $qualityReason `
    -Paths $paths |
    ConvertTo-Json -Compress
