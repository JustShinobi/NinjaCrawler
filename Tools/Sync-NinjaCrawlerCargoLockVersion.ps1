# Syncs the root application package version in src-tauri/Cargo.lock from
# src-tauri/Cargo.toml. Cargo.lock is a derived file: Release Please must not
# annotate or bump it; this script (or the release-PR workflow) keeps the
# embedded ninjacrawler package version aligned after a Cargo.toml bump.
param(
    [string]$RepositoryRoot = "",
    [switch]$Check
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($RepositoryRoot)) {
    $RepositoryRoot = Split-Path -Parent $PSScriptRoot
}

$cargoTomlPath = Join-Path $RepositoryRoot "src-tauri\Cargo.toml"
$cargoLockPath = Join-Path $RepositoryRoot "src-tauri\Cargo.lock"

if (-not (Test-Path -LiteralPath $cargoTomlPath)) {
    throw "Cargo.toml not found at $cargoTomlPath"
}
if (-not (Test-Path -LiteralPath $cargoLockPath)) {
    throw "Cargo.lock not found at $cargoLockPath"
}

$cargoToml = Get-Content -LiteralPath $cargoTomlPath -Raw
$cargoTomlMatch = [regex]::Match(
    $cargoToml,
    '(?m)^version\s*=\s*"([^"]+)"'
)
if (-not $cargoTomlMatch.Success) {
    throw "Could not resolve the package version from src-tauri/Cargo.toml."
}
$expectedVersion = $cargoTomlMatch.Groups[1].Value
if ($expectedVersion -notmatch '^\d+\.\d+\.\d+$') {
    throw "Cargo.toml version '$expectedVersion' is not a stable semver X.Y.Z value."
}

$cargoLock = Get-Content -LiteralPath $cargoLockPath -Raw
# Match the root app package block and its version line (optional legacy marker).
$packagePattern = '(?ms)(^\[\[package\]\]\s*name\s*=\s*"ninjacrawler"\s*version\s*=\s*")([^"]+)("(?:[ \t]*#[^\r\n]*)?)'
$packageMatch = [regex]::Match($cargoLock, $packagePattern)
if (-not $packageMatch.Success) {
    throw "Could not resolve the ninjacrawler package entry in src-tauri/Cargo.lock."
}

$currentVersion = $packageMatch.Groups[2].Value
$versionSuffix = $packageMatch.Groups[3].Value
$hasLegacyMarker = $versionSuffix -match 'x-release-please-version'
$needsRewrite = ($currentVersion -ne $expectedVersion) -or $hasLegacyMarker -or ($versionSuffix -ne '"')

if (-not $needsRewrite) {
    Write-Output $expectedVersion
    return
}

if ($Check) {
    if ($currentVersion -ne $expectedVersion) {
        throw @"
src-tauri/Cargo.lock has ninjacrawler version '$currentVersion', expected '$expectedVersion' from Cargo.toml.
Run: pwsh -File Tools/Sync-NinjaCrawlerCargoLockVersion.ps1
"@
    }
    throw @"
src-tauri/Cargo.lock still carries a legacy version-line annotation.
Cargo.lock is derived; run: pwsh -File Tools/Sync-NinjaCrawlerCargoLockVersion.ps1
"@
}

$replacement = $packageMatch.Groups[1].Value + $expectedVersion + '"'
$updated = $cargoLock.Remove($packageMatch.Index, $packageMatch.Length).Insert($packageMatch.Index, $replacement)

# Normalize newlines the same way Cargo writes lockfiles on this repo (LF).
$updated = $updated -replace "`r`n", "`n"
if (-not $updated.EndsWith("`n")) {
    $updated += "`n"
}

$utf8NoBom = New-Object System.Text.UTF8Encoding $false
[System.IO.File]::WriteAllText($cargoLockPath, $updated, $utf8NoBom)

Write-Output $expectedVersion
