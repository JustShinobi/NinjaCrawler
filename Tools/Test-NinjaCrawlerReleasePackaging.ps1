Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$testOutputRoot = "artifacts\companion-package-test"
$testOutputPath = Join-Path $repoRoot $testOutputRoot
$appTestOutputRoot = "artifacts\app-package-test"
$appTestOutputPath = Join-Path $repoRoot $appTestOutputRoot
$appBuildRoot = Join-Path $repoRoot "artifacts\app-build-fixture"
$manifest = Get-Content -LiteralPath (Join-Path $repoRoot "NinjaCrawler.Companion\manifest.json") -Raw |
    ConvertFrom-Json
$assetName = "NinjaCrawler-Companion-$($manifest.version).zip"

try {
    # -CompanionOnly and -SkipCompanion are contradictory and must be rejected.
    $mutualExclusionRejected = $false
    try {
        & (Join-Path $PSScriptRoot "Package-NinjaCrawlerRelease.ps1") `
            -Version "0.0.0" `
            -OutputRoot $testOutputRoot `
            -CompanionOnly `
            -SkipCompanion
    } catch {
        $mutualExclusionRejected = $true
    }
    if (-not $mutualExclusionRejected) {
        throw "Packaging must reject -CompanionOnly together with -SkipCompanion."
    }

    & (Join-Path $PSScriptRoot "Package-NinjaCrawlerRelease.ps1") `
        -Version "0.0.0" `
        -OutputRoot $testOutputRoot `
        -CompanionOnly

    $assetPath = Join-Path $testOutputPath $assetName
    $checksumPath = Join-Path $testOutputPath "SHA256SUMS.txt"
    if (-not (Test-Path -LiteralPath $assetPath -PathType Leaf)) {
        throw "Expected Companion asset was not generated: '$assetName'."
    }
    if (-not (Test-Path -LiteralPath $checksumPath -PathType Leaf)) {
        throw "SHA256SUMS.txt was not generated."
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($assetPath)
    try {
        $entries = @($archive.Entries | ForEach-Object { $_.FullName.Replace('\', '/') })
        $root = "NinjaCrawler-Companion/"
        foreach ($requiredEntry in @(
            "${root}manifest.json",
            "${root}popup.html",
            "${root}README.md",
            "${root}src/background.js",
            "${root}src/core.js",
            "${root}src/popup.js"
        )) {
            if ($entries -notcontains $requiredEntry) {
                throw "Required ZIP entry is missing: '$requiredEntry'."
            }
        }
        if (@($entries | Where-Object { $_ -like "*.test.js" }).Count -ne 0) {
            throw "Companion test files must not be included in the release ZIP."
        }
    } finally {
        $archive.Dispose()
    }

    $expectedHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $assetPath).Hash.ToLowerInvariant()
    $checksumContents = Get-Content -LiteralPath $checksumPath -Raw
    if ($checksumContents -notmatch [regex]::Escape("$expectedHash  $assetName")) {
        throw "Companion checksum is missing or incorrect."
    }

    $readmeFixture = Join-Path $testOutputPath "README.release-test.md"
    Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination $readmeFixture
    & (Join-Path $PSScriptRoot "Update-NinjaCrawlerReleaseReadme.ps1") `
        -Version "9.8.7" `
        -Path $readmeFixture

    $readmeContents = Get-Content -LiteralPath $readmeFixture -Raw
    foreach ($expectedLink in @(
        "releases/download/v9.8.7/NinjaCrawler-9.8.7-windows-x64-setup.exe",
        "releases/download/v9.8.7/NinjaCrawler-9.8.7-windows-x64-portable.exe",
        "releases/download/v9.8.7/SHA256SUMS.txt"
    )) {
        if (-not $readmeContents.Contains($expectedLink)) {
            throw "README release updater missed expected link '$expectedLink'."
        }
    }

    $releaseBlocks = [regex]::Matches(
        $readmeContents,
        '(?ms)<!--\s*ninjacrawler-release-start\s*-->(.*?)<!--\s*ninjacrawler-release-end\s*-->'
    )
    if ($releaseBlocks.Count -ne 2) {
        throw "Expected exactly 2 updated README release blocks."
    }
    foreach ($releaseBlock in $releaseBlocks) {
        $versions = @(
            [regex]::Matches(
                $releaseBlock.Value,
                '(?<!\d)\d+\.\d+\.\d+(?![\d.])'
            ) |
                ForEach-Object Value |
                Select-Object -Unique
        )
        if ($versions.Count -ne 1 -or $versions[0] -ne "9.8.7") {
            throw "README release block contains unexpected versions: $($versions -join ', ')."
        }
    }

    New-Item -ItemType Directory -Path (Join-Path $appBuildRoot "bundle\nsis") -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $appBuildRoot "ninjacrawler.exe") -Value "portable fixture"
    Set-Content -LiteralPath (Join-Path $appBuildRoot "bundle\nsis\fixture-setup.exe") -Value "setup fixture"
    $changelogFixture = Join-Path $repoRoot "artifacts\CHANGELOG.fixture.md"
    Set-Content -LiteralPath $changelogFixture -Value "# Fixture changelog"

    & (Join-Path $PSScriptRoot "Package-NinjaCrawlerRelease.ps1") `
        -Version "9.8.7" `
        -OutputRoot $appTestOutputRoot `
        -BuildRoot "artifacts\app-build-fixture" `
        -ChangelogPath "artifacts\CHANGELOG.fixture.md" `
        -SkipCompanion

    $expectedAppAssets = @(
        "NinjaCrawler-9.8.7-windows-x64-portable.exe",
        "NinjaCrawler-9.8.7-windows-x64-setup.exe",
        "CHANGELOG.md",
        "SHA256SUMS.txt"
    )
    $actualAppAssets = @(Get-ChildItem -LiteralPath $appTestOutputPath -File | ForEach-Object Name)
    foreach ($expectedAsset in $expectedAppAssets) {
        if ($expectedAsset -notin $actualAppAssets) {
            throw "Expected app release asset was not generated: '$expectedAsset'."
        }
    }
    if (@($actualAppAssets | Where-Object { $_ -like "*.zip" -or $_ -like "*.msi" }).Count -ne 0) {
        throw "App release must not contain ZIP or MSI assets."
    }
    $appChecksums = @(Get-Content -LiteralPath (Join-Path $appTestOutputPath "SHA256SUMS.txt"))
    if ($appChecksums.Count -ne 3) {
        throw "App checksums must cover the portable executable, NSIS installer, and changelog."
    }
} finally {
    if (Test-Path -LiteralPath $testOutputPath) {
        Remove-Item -LiteralPath $testOutputPath -Recurse -Force
    }
    foreach ($path in @($appTestOutputPath, $appBuildRoot, (Join-Path $repoRoot "artifacts\CHANGELOG.fixture.md"))) {
        if (Test-Path -LiteralPath $path) {
            Remove-Item -LiteralPath $path -Recurse -Force
        }
    }
}

Write-Host "Release packaging tests passed."
