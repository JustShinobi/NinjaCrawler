param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$CompanionVersion,

    [Parameter(Mandatory = $true)]
    [string]$CompanionAssetPath,

    [Parameter(Mandatory = $true)]
    [string]$ChangelogPath,

    [Parameter(Mandatory = $true)]
    [string]$ChecksumPath
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

foreach ($path in @($CompanionAssetPath, $ChangelogPath, $ChecksumPath)) {
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        throw "Required release file does not exist: '$path'."
    }
}

$companionAsset = Get-Item -LiteralPath $CompanionAssetPath
$expectedAssetName = "NinjaCrawler-Companion-$CompanionVersion.zip"
if ($companionAsset.Name -ne $expectedAssetName) {
    throw "Companion asset '$($companionAsset.Name)' must be named '$expectedAssetName'."
}

$changelog = Get-Content -LiteralPath $ChangelogPath -Raw
$companionNotePattern = 'This desktop release (?:includes|provides) \*\*NinjaCrawler Companion \d+\.\d+\.\d+\*\* \(NinjaCrawler-Companion-\d+\.\d+\.\d+\.zip\)(?: as its current Companion asset)?\.'
if ($changelog -notmatch $companionNotePattern) {
    throw 'The latest NinjaCrawler changelog does not contain the expected Companion asset annotation.'
}

$companionNote = "This desktop release provides **NinjaCrawler Companion $CompanionVersion** ($expectedAssetName) as its current Companion asset."
$updatedChangelog = ([regex]::new($companionNotePattern)).Replace($changelog, $companionNote, 1)
$utf8WithoutBom = [System.Text.UTF8Encoding]::new($false)
[System.IO.File]::WriteAllText(
    [System.IO.Path]::GetFullPath($ChangelogPath),
    $updatedChangelog,
    $utf8WithoutBom
)

$preservedChecksums = @(
    Get-Content -LiteralPath $ChecksumPath |
        Where-Object {
            $_ -notmatch '\s{2}NinjaCrawler-Companion-\d+\.\d+\.\d+\.zip$' -and
            $_ -notmatch '\s{2}CHANGELOG\.md$'
        }
)
$changelogHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $ChangelogPath).Hash.ToLowerInvariant()
$companionHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $CompanionAssetPath).Hash.ToLowerInvariant()
$updatedChecksums = @(
    $preservedChecksums
    "$changelogHash  CHANGELOG.md"
    "$companionHash  $expectedAssetName"
)
[System.IO.File]::WriteAllLines(
    [System.IO.Path]::GetFullPath($ChecksumPath),
    $updatedChecksums,
    $utf8WithoutBom
)

Write-Host "Prepared latest NinjaCrawler release metadata for Companion $CompanionVersion."
