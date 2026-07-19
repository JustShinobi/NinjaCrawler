param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern("^\d+\.\d+\.\d+$")]
    [string]$Version,
    [string]$OutputRoot = "release",
    [string]$BuildRoot = "",
    [string]$TargetTriple = "",
    [string]$ChangelogPath = "",
    [switch]$CompanionOnly,
    [switch]$SkipCompanion
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ($CompanionOnly -and $SkipCompanion) {
    throw "-CompanionOnly and -SkipCompanion are mutually exclusive."
}

# The tauri-plugin-updater `latest.json` manifest is NOT produced here. It is
# generated in the release workflow's publish job by Tools/New-UpdaterManifest.ps1,
# which signs the installer out-of-band (bundle.createUpdaterArtifacts is off) so
# the signing key never reaches the unprivileged build.

$repoRoot = Split-Path -Parent $PSScriptRoot
$releaseRoot = if ([string]::IsNullOrWhiteSpace($BuildRoot)) {
    $path = if ([string]::IsNullOrWhiteSpace($env:CARGO_TARGET_DIR)) {
        Join-Path $repoRoot "src-tauri/target"
    } elseif ([System.IO.Path]::IsPathRooted($env:CARGO_TARGET_DIR)) {
        [System.IO.Path]::GetFullPath($env:CARGO_TARGET_DIR)
    } else {
        [System.IO.Path]::GetFullPath((Join-Path $repoRoot $env:CARGO_TARGET_DIR))
    }
    if (-not [string]::IsNullOrWhiteSpace($TargetTriple)) {
        $path = Join-Path $path $TargetTriple
    }
    Join-Path $path "release"
} else {
    [System.IO.Path]::GetFullPath((Join-Path $repoRoot $BuildRoot))
}
$outputPath = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $OutputRoot))
$directorySeparator = [System.IO.Path]::DirectorySeparatorChar
$normalizedRepoRoot = [System.IO.Path]::GetFullPath($repoRoot).TrimEnd(
    [System.IO.Path]::DirectorySeparatorChar,
    [System.IO.Path]::AltDirectorySeparatorChar
) + $directorySeparator
$companionSourcePath = Join-Path $repoRoot "NinjaCrawler.Companion"
$companionManifestPath = Join-Path $companionSourcePath "manifest.json"

if (-not $outputPath.StartsWith($normalizedRepoRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "OutputRoot must stay inside the repository: '$outputPath'."
}

if (Test-Path -LiteralPath $outputPath) {
    Remove-Item -LiteralPath $outputPath -Recurse -Force
}
New-Item -ItemType Directory -Path $outputPath -Force | Out-Null

$assets = @()

if (-not $CompanionOnly) {
    $executablePath = Join-Path $releaseRoot "ninjacrawler.exe"
    $nsisPath = Get-ChildItem (Join-Path $releaseRoot "bundle/nsis") -Filter "*-setup.exe" -File |
        Select-Object -First 1

    if (-not (Test-Path -LiteralPath $executablePath)) {
        throw "Release executable not found: '$executablePath'."
    }
    if (-not $nsisPath) {
        throw "NSIS bundle not found below '$releaseRoot/bundle/nsis'."
    }

    $portableDestination = Join-Path $outputPath "NinjaCrawler-$Version-windows-x64-portable.exe"
    $nsisDestination = Join-Path $outputPath "NinjaCrawler-$Version-windows-x64-setup.exe"
    Copy-Item -LiteralPath $executablePath -Destination $portableDestination
    Copy-Item -LiteralPath $nsisPath.FullName -Destination $nsisDestination
    $assets += @($portableDestination, $nsisDestination)

    if (-not [string]::IsNullOrWhiteSpace($ChangelogPath)) {
        $resolvedChangelogPath = [System.IO.Path]::GetFullPath((Join-Path $repoRoot $ChangelogPath))
        if (-not (Test-Path -LiteralPath $resolvedChangelogPath -PathType Leaf)) {
            throw "Changelog not found: '$resolvedChangelogPath'."
        }
        $changelogDestination = Join-Path $outputPath "CHANGELOG.md"
        Copy-Item -LiteralPath $resolvedChangelogPath -Destination $changelogDestination
        $assets += $changelogDestination
    }
}

# App releases co-ship the Companion ZIP from the tree. Companion-only releases
# use -CompanionOnly. -SkipCompanion remains available for local app-only packaging.
if (-not $SkipCompanion) {
    if (-not (Test-Path -LiteralPath $companionManifestPath -PathType Leaf)) {
        throw "Companion manifest not found: '$companionManifestPath'."
    }

    try {
        $companionManifest = Get-Content -LiteralPath $companionManifestPath -Raw | ConvertFrom-Json
    } catch {
        throw "Companion manifest is not valid JSON: $($_.Exception.Message)"
    }

    $companionVersion = [string]$companionManifest.version
    if ($companionVersion -notmatch '^\d+\.\d+\.\d+$') {
        throw "Companion manifest version '$companionVersion' must use the X.Y.Z format."
    }

    $companionRequiredFiles = @("manifest.json", "popup.html", "README.md")
    foreach ($relativePath in $companionRequiredFiles) {
        $requiredPath = Join-Path $companionSourcePath $relativePath
        if (-not (Test-Path -LiteralPath $requiredPath -PathType Leaf)) {
            throw "Required Companion file not found: '$requiredPath'."
        }
    }

    # Include icons and other non-test runtime files under the Companion tree.
    $companionSourceFiles = @(Get-ChildItem -LiteralPath (Join-Path $companionSourcePath "src") -File -Recurse |
        Where-Object { $_.Name -notlike "*.test.js" })
    if ($companionSourceFiles.Count -eq 0) {
        throw "No Companion runtime files were found."
    }

    $companionName = "NinjaCrawler-Companion-$companionVersion"
    $companionArchiveRoot = "NinjaCrawler-Companion"
    $companionStagingPath = Join-Path $outputPath $companionArchiveRoot
    New-Item -ItemType Directory -Path $companionStagingPath -Force | Out-Null

    foreach ($relativePath in $companionRequiredFiles) {
        Copy-Item -LiteralPath (Join-Path $companionSourcePath $relativePath) -Destination $companionStagingPath
    }
    foreach ($sourceFile in $companionSourceFiles) {
        $relativePath = $sourceFile.FullName.Substring($companionSourcePath.Length).TrimStart('\', '/')
        $destinationPath = Join-Path $companionStagingPath $relativePath
        New-Item -ItemType Directory -Path (Split-Path -Parent $destinationPath) -Force | Out-Null
        Copy-Item -LiteralPath $sourceFile.FullName -Destination $destinationPath
    }

    $iconsPath = Join-Path $companionSourcePath "icons"
    if (Test-Path -LiteralPath $iconsPath -PathType Container) {
        Copy-Item -LiteralPath $iconsPath -Destination (Join-Path $companionStagingPath "icons") -Recurse
    }

    $companionZip = Join-Path $outputPath "$companionName.zip"
    Compress-Archive -LiteralPath $companionStagingPath -DestinationPath $companionZip -CompressionLevel Optimal
    Remove-Item -LiteralPath $companionStagingPath -Recurse -Force
    $assets += $companionZip
}

if ($assets.Count -eq 0) {
    throw "No release assets were produced."
}

$checksumLines = foreach ($asset in $assets) {
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $asset).Hash.ToLowerInvariant()
    "$hash  $([System.IO.Path]::GetFileName($asset))"
}
$checksumPath = Join-Path $outputPath "SHA256SUMS.txt"
[System.IO.File]::WriteAllLines($checksumPath, $checksumLines)

Write-Host "ReleaseAssets:"
Get-ChildItem -LiteralPath $outputPath -File |
    Sort-Object Name |
    ForEach-Object { Write-Host "$($_.Name) [$($_.Length) bytes]" }
