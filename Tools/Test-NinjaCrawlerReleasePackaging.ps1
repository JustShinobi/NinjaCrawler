Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$testOutputRoot = "artifacts\companion-package-test"
$testOutputPath = Join-Path $repoRoot $testOutputRoot
$appTestOutputRoot = "artifacts\app-package-test"
$appTestOutputPath = Join-Path $repoRoot $appTestOutputRoot
$appOnlyOutputRoot = "artifacts\app-only-package-test"
$appOnlyOutputPath = Join-Path $repoRoot $appOnlyOutputRoot
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
            "${root}icons/16.png",
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
        -CompanionVersion "1.2.3" `
        -Path $readmeFixture

    $readmeContents = Get-Content -LiteralPath $readmeFixture -Raw
    foreach ($expectedLink in @(
        "releases/download/v9.8.7/NinjaCrawler-9.8.7-windows-x64-setup.exe",
        "releases/download/v9.8.7/NinjaCrawler-9.8.7-windows-x64-portable.exe",
        "releases/download/v9.8.7/SHA256SUMS.txt",
        "releases/download/companion-v1.2.3/NinjaCrawler-Companion-1.2.3.zip"
    )) {
        if (-not $readmeContents.Contains($expectedLink)) {
            throw "README release updater missed expected link '$expectedLink'."
        }
    }

    $releaseBlocks = [regex]::Matches(
        $readmeContents,
        '(?ms)<!--\s*ninjacrawler-release-start\s*-->(.*?)<!--\s*ninjacrawler-release-end\s*-->'
    )
    if ($releaseBlocks.Count -ne 3) {
        throw "Expected exactly 3 updated README app release blocks, found $($releaseBlocks.Count)."
    }
    foreach ($releaseBlock in $releaseBlocks) {
        $versions = @(
            [regex]::Matches(
                $releaseBlock.Value,
                '(?<![\d.])\d+\.\d+\.\d+(?!\.\d)'
            ) |
                ForEach-Object Value |
                Select-Object -Unique
        )
        if ($versions.Count -ne 1 -or $versions[0] -ne "9.8.7") {
            throw "README app release block contains unexpected versions: $($versions -join ', ')."
        }
    }

    $companionBlocks = [regex]::Matches(
        $readmeContents,
        '(?ms)<!--\s*ninjacrawler-companion-release-start\s*-->(.*?)<!--\s*ninjacrawler-companion-release-end\s*-->'
    )
    if ($companionBlocks.Count -ne 3) {
        throw "Expected exactly 3 updated README Companion release blocks, found $($companionBlocks.Count)."
    }
    foreach ($companionBlock in $companionBlocks) {
        $versions = @(
            [regex]::Matches(
                $companionBlock.Value,
                '(?<![\d.])\d+\.\d+\.\d+(?!\.\d)'
            ) |
                ForEach-Object Value |
                Select-Object -Unique
        )
        if ($versions.Count -ne 1 -or $versions[0] -ne "1.2.3") {
            throw "README Companion release block contains unexpected versions: $($versions -join ', ')."
        }
    }

    # Companion-only update must not rewrite app version blocks.
    & (Join-Path $PSScriptRoot "Update-NinjaCrawlerReleaseReadme.ps1") `
        -CompanionVersion "4.5.6" `
        -Path $readmeFixture
    $readmeCompanionOnly = Get-Content -LiteralPath $readmeFixture -Raw
    if (-not $readmeCompanionOnly.Contains("releases/download/v9.8.7/NinjaCrawler-9.8.7-windows-x64-setup.exe")) {
        throw "Companion-only README update must preserve app download links."
    }
    if (-not $readmeCompanionOnly.Contains("releases/download/companion-v4.5.6/NinjaCrawler-Companion-4.5.6.zip")) {
        throw "Companion-only README update must rewrite Companion download links."
    }

    New-Item -ItemType Directory -Path (Join-Path $appBuildRoot "bundle\nsis") -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $appBuildRoot "ninjacrawler.exe") -Value "portable fixture"
    Set-Content -LiteralPath (Join-Path $appBuildRoot "bundle\nsis\fixture-setup.exe") -Value "setup fixture"
    $changelogFixture = Join-Path $repoRoot "artifacts\CHANGELOG.fixture.md"
    Set-Content -LiteralPath $changelogFixture -Value "# Fixture changelog"

    # Default app packaging co-ships the Companion ZIP from the tree.
    & (Join-Path $PSScriptRoot "Package-NinjaCrawlerRelease.ps1") `
        -Version "9.8.7" `
        -OutputRoot $appTestOutputRoot `
        -BuildRoot "artifacts\app-build-fixture" `
        -ChangelogPath "artifacts\CHANGELOG.fixture.md"

    $expectedAppAssets = @(
        "NinjaCrawler-9.8.7-windows-x64-portable.exe",
        "NinjaCrawler-9.8.7-windows-x64-setup.exe",
        $assetName,
        "CHANGELOG.md",
        "SHA256SUMS.txt"
    )
    $actualAppAssets = @(Get-ChildItem -LiteralPath $appTestOutputPath -File | ForEach-Object Name)
    foreach ($expectedAsset in $expectedAppAssets) {
        if ($expectedAsset -notin $actualAppAssets) {
            throw "Expected app release asset was not generated: '$expectedAsset'."
        }
    }
    if (@($actualAppAssets | Where-Object { $_ -like "*.msi" }).Count -ne 0) {
        throw "App release must not contain MSI assets."
    }
    $appChecksums = @(Get-Content -LiteralPath (Join-Path $appTestOutputPath "SHA256SUMS.txt"))
    if ($appChecksums.Count -ne 4) {
        throw "App checksums must cover portable, NSIS, Companion ZIP, and changelog."
    }
    if (-not ($appChecksums -match [regex]::Escape($assetName))) {
        throw "App SHA256SUMS.txt must include the co-shipped Companion ZIP."
    }

    # Companion-only releases must refresh the latest app release metadata
    # without invalidating checksums or leaving the old version referenced.
    $mirrorCompanionVersion = "9.9.9"
    $mirrorAssetName = "NinjaCrawler-Companion-$mirrorCompanionVersion.zip"
    $mirrorAssetPath = Join-Path $testOutputPath $mirrorAssetName
    $mirrorChangelogPath = Join-Path $testOutputPath "CHANGELOG.mirror-test.md"
    $mirrorChecksumPath = Join-Path $testOutputPath "SHA256SUMS.mirror-test.txt"
    Set-Content -LiteralPath $mirrorAssetPath -Value "new companion fixture"
    Set-Content -LiteralPath $mirrorChangelogPath -Value @(
        "# NinjaCrawler fixture"
        ""
        "This desktop release includes **NinjaCrawler Companion 1.2.3** (NinjaCrawler-Companion-1.2.3.zip)."
        "Companion-only updates continue to ship under independent companion-vX.Y.Z tags."
    )
    Set-Content -LiteralPath $mirrorChecksumPath -Value @(
        "aaaaaaaa  NinjaCrawler-9.8.7-windows-x64-portable.exe"
        "bbbbbbbb  CHANGELOG.md"
        "cccccccc  NinjaCrawler-Companion-1.2.3.zip"
    )

    & (Join-Path $PSScriptRoot "Update-NinjaCrawlerReleaseCompanionMetadata.ps1") `
        -CompanionVersion $mirrorCompanionVersion `
        -CompanionAssetPath $mirrorAssetPath `
        -ChangelogPath $mirrorChangelogPath `
        -ChecksumPath $mirrorChecksumPath

    $mirrorChangelog = Get-Content -LiteralPath $mirrorChangelogPath -Raw
    if (-not $mirrorChangelog.Contains("provides **NinjaCrawler Companion $mirrorCompanionVersion** ($mirrorAssetName) as its current Companion asset.")) {
        throw "Latest app release changelog was not updated to the mirrored Companion version."
    }
    if ($mirrorChangelog.Contains("NinjaCrawler Companion 1.2.3")) {
        throw "Latest app release changelog still references the stale Companion version."
    }

    $mirrorChecksums = @(Get-Content -LiteralPath $mirrorChecksumPath)
    $mirrorCompanionHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $mirrorAssetPath).Hash.ToLowerInvariant()
    $mirrorChangelogHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $mirrorChangelogPath).Hash.ToLowerInvariant()
    foreach ($expectedChecksum in @(
        "aaaaaaaa  NinjaCrawler-9.8.7-windows-x64-portable.exe",
        "$mirrorChangelogHash  CHANGELOG.md",
        "$mirrorCompanionHash  $mirrorAssetName"
    )) {
        if ($mirrorChecksums -notcontains $expectedChecksum) {
            throw "Mirrored release checksum is missing: '$expectedChecksum'."
        }
    }
    if (@($mirrorChecksums | Where-Object { $_ -match 'NinjaCrawler-Companion-1\.2\.3\.zip$' }).Count -ne 0) {
        throw "Latest app release checksums still reference the stale Companion asset."
    }

    # -SkipCompanion remains available for local app-only packaging.
    & (Join-Path $PSScriptRoot "Package-NinjaCrawlerRelease.ps1") `
        -Version "9.8.7" `
        -OutputRoot $appOnlyOutputRoot `
        -BuildRoot "artifacts\app-build-fixture" `
        -ChangelogPath "artifacts\CHANGELOG.fixture.md" `
        -SkipCompanion

    $appOnlyAssets = @(Get-ChildItem -LiteralPath $appOnlyOutputPath -File | ForEach-Object Name)
    if (@($appOnlyAssets | Where-Object { $_ -like "*.zip" }).Count -ne 0) {
        throw "-SkipCompanion must omit the Companion ZIP."
    }
    $appOnlyChecksums = @(Get-Content -LiteralPath (Join-Path $appOnlyOutputPath "SHA256SUMS.txt"))
    if ($appOnlyChecksums.Count -ne 3) {
        throw "-SkipCompanion checksums must cover only portable, NSIS, and changelog."
    }
} finally {
    foreach ($path in @(
        $testOutputPath,
        $appTestOutputPath,
        $appOnlyOutputPath,
        $appBuildRoot,
        (Join-Path $repoRoot "artifacts\CHANGELOG.fixture.md")
    )) {
        if (Test-Path -LiteralPath $path) {
            Remove-Item -LiteralPath $path -Recurse -Force
        }
    }
}

Write-Host "Release packaging tests passed."
