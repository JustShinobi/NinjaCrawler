param(
    [string]$ExpectedVersion = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$packageVersion = (Get-Content (Join-Path $repoRoot "package.json") -Raw | ConvertFrom-Json).version
$tauriVersion = (Get-Content (Join-Path $repoRoot "src-tauri/tauri.conf.json") -Raw | ConvertFrom-Json).version
$cargoVersion = (
    Select-String -Path (Join-Path $repoRoot "src-tauri/Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' |
        Select-Object -First 1
).Matches[0].Groups[1].Value
$cargoLockContent = Get-Content (Join-Path $repoRoot "src-tauri/Cargo.lock") -Raw
$cargoLockMatch = [regex]::Match(
    $cargoLockContent,
    '(?ms)^\[\[package\]\]\s*name\s*=\s*"ninjacrawler"\s*version\s*=\s*"([^"]+)"'
)
if (-not $cargoLockMatch.Success) {
    throw "Could not resolve the NinjaCrawler package version from src-tauri/Cargo.lock."
}

$versions = [ordered]@{
    'package.json' = $packageVersion
    'src-tauri/tauri.conf.json' = $tauriVersion
    'src-tauri/Cargo.toml' = $cargoVersion
    'src-tauri/Cargo.lock' = $cargoLockMatch.Groups[1].Value
}
$requiredVersion = if ([string]::IsNullOrWhiteSpace($ExpectedVersion)) {
    $packageVersion
} else {
    $ExpectedVersion
}

foreach ($entry in $versions.GetEnumerator()) {
    if ($entry.Value -ne $requiredVersion) {
        throw "$($entry.Key) has version '$($entry.Value)', expected '$requiredVersion'."
    }
}

Write-Output $requiredVersion
