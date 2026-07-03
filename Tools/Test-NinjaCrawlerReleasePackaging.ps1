Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$testOutputRoot = "artifacts\companion-package-test"
$testOutputPath = Join-Path $repoRoot $testOutputRoot
$manifest = Get-Content -LiteralPath (Join-Path $repoRoot "NinjaCrawler.Companion\manifest.json") -Raw |
    ConvertFrom-Json
$assetName = "NinjaCrawler-Companion-$($manifest.version).zip"

try {
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
} finally {
    if (Test-Path -LiteralPath $testOutputPath) {
        Remove-Item -LiteralPath $testOutputPath -Recurse -Force
    }
}

Write-Host "Companion release packaging test passed."
