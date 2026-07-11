Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$testOutputRoot = "artifacts\companion-package-test"
$testOutputPath = Join-Path $repoRoot $testOutputRoot
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
        "releases/download/v9.8.7/NinjaCrawler-9.8.7-windows-x64-portable.zip",
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
} finally {
    if (Test-Path -LiteralPath $testOutputPath) {
        Remove-Item -LiteralPath $testOutputPath -Recurse -Force
    }
}

Write-Host "Release packaging tests passed."
