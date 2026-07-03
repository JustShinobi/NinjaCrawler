param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern("^\d+\.\d+\.\d+$")]
    [string]$Version,
    [string]$OutputRoot = "release",
    [switch]$CompanionOnly
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$releaseRoot = Join-Path $repoRoot "src-tauri\target\release"
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
    $msiPath = Get-ChildItem (Join-Path $releaseRoot "bundle\msi") -Filter "*.msi" -File |
        Select-Object -First 1
    $nsisPath = Get-ChildItem (Join-Path $releaseRoot "bundle\nsis") -Filter "*-setup.exe" -File |
        Select-Object -First 1
    $bootstrapPath = Join-Path $repoRoot "connectors\bootstrap"

    if (-not (Test-Path -LiteralPath $executablePath)) {
        throw "Release executable not found: '$executablePath'."
    }
    if (-not $msiPath) {
        throw "MSI bundle not found below '$releaseRoot\bundle\msi'."
    }
    if (-not $nsisPath) {
        throw "NSIS bundle not found below '$releaseRoot\bundle\nsis'."
    }
    if (-not (Test-Path -LiteralPath $bootstrapPath)) {
        throw "Connector bootstrap directory not found: '$bootstrapPath'."
    }

    $portableName = "NinjaCrawler-$Version-windows-x64"
    $portablePath = Join-Path $outputPath $portableName
    New-Item -ItemType Directory -Path $portablePath -Force | Out-Null

    Copy-Item -LiteralPath $executablePath -Destination (Join-Path $portablePath "ninjacrawler.exe")
    Copy-Item -LiteralPath (Join-Path $repoRoot "README.md") -Destination $portablePath
    $portableConnectorsPath = Join-Path $portablePath "connectors"
    New-Item -ItemType Directory -Path $portableConnectorsPath -Force | Out-Null
    Copy-Item -LiteralPath $bootstrapPath -Destination (Join-Path $portableConnectorsPath "bootstrap") -Recurse

    $portableZip = Join-Path $outputPath "$portableName-portable.zip"
    Compress-Archive -LiteralPath $portablePath -DestinationPath $portableZip -CompressionLevel Optimal
    Remove-Item -LiteralPath $portablePath -Recurse -Force

    $msiDestination = Join-Path $outputPath "NinjaCrawler-$Version-windows-x64.msi"
    $nsisDestination = Join-Path $outputPath "NinjaCrawler-$Version-windows-x64-setup.exe"
    Copy-Item -LiteralPath $msiPath.FullName -Destination $msiDestination
    Copy-Item -LiteralPath $nsisPath.FullName -Destination $nsisDestination
    $assets += @($portableZip, $msiDestination, $nsisDestination)
}

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

$companionZip = Join-Path $outputPath "$companionName.zip"
Compress-Archive -LiteralPath $companionStagingPath -DestinationPath $companionZip -CompressionLevel Optimal
Remove-Item -LiteralPath $companionStagingPath -Recurse -Force
$assets += $companionZip

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
