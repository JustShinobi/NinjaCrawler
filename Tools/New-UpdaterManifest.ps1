<#
.SYNOPSIS
    Signs the Windows NSIS installer and emits the tauri-plugin-updater v2
    `latest.json` manifest for a packaged release.

.DESCRIPTION
    This runs in the privileged publish job of .github/workflows/release.yml
    (never in the unprivileged self-hosted build), so the minisign signing key
    never reaches the build runner. `bundle.createUpdaterArtifacts` stays
    disabled: the updater signature is a plain minisign signature over the
    installer and is produced here out-of-band with `tauri signer sign`.

    The script:
      1. Locates the `*-windows-x64-setup.exe` installer in -ReleaseDir.
      2. Signs it with `npx @tauri-apps/cli signer sign`, producing a detached
         `<installer>.sig`. The private key and its password are read from the
         TAURI_SIGNING_PRIVATE_KEY / TAURI_SIGNING_PRIVATE_KEY_PASSWORD
         environment variables (never passed on the command line, never logged).
      3. Writes `latest.json` (version, RFC3339 pub_date, platforms.windows-x86_64
         with the raw signature contents and the versioned download URL).
      4. Removes the `.sig` sidecar (the signature is embedded in the manifest).
      5. Appends `latest.json` to the existing SHA256SUMS.txt.

    Any failure (missing key, signing error, missing signature) throws.

.PARAMETER Version
    Release version in X.Y.Z form.

.PARAMETER ReleaseDir
    Directory holding the packaged release assets (installer + SHA256SUMS.txt).

.PARAMETER Repository
    GitHub owner/repo that hosts the versioned release assets. Keep in sync with
    `plugins.updater.endpoints` in src-tauri/tauri.conf.json.

.PARAMETER CliVersion
    npm version range for @tauri-apps/cli used to run `signer sign`.
#>
param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern("^\d+\.\d+\.\d+$")]
    [string]$Version,
    [Parameter(Mandatory = $true)]
    [string]$ReleaseDir,
    [string]$Repository = "JustShinobi/NinjaCrawler",
    [string]$CliVersion = "^2"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $ReleaseDir -PathType Container)) {
    throw "Release directory not found: '$ReleaseDir'."
}
$resolvedReleaseDir = (Resolve-Path -LiteralPath $ReleaseDir).Path

if ([string]::IsNullOrWhiteSpace($env:TAURI_SIGNING_PRIVATE_KEY)) {
    throw "TAURI_SIGNING_PRIVATE_KEY is not set; cannot sign the updater artifact."
}

$setup = Get-ChildItem -LiteralPath $resolvedReleaseDir -Filter "*-windows-x64-setup.exe" -File |
    Select-Object -First 1
if (-not $setup) {
    throw "No '*-windows-x64-setup.exe' installer found in '$resolvedReleaseDir'."
}

# Sign the installer. The key and password are read from the environment by the
# Tauri CLI (both flags are env-backed), so they never appear on the command
# line. Fail hard if signing does not succeed.
& npx --yes "@tauri-apps/cli@$CliVersion" signer sign $setup.FullName
if ($LASTEXITCODE -ne 0) {
    throw "tauri signer sign failed with exit code $LASTEXITCODE."
}

$signaturePath = "$($setup.FullName).sig"
if (-not (Test-Path -LiteralPath $signaturePath -PathType Leaf)) {
    throw "Updater signature not found after signing: '$signaturePath'."
}
$signature = (Get-Content -LiteralPath $signaturePath -Raw).Trim()
if ([string]::IsNullOrWhiteSpace($signature)) {
    throw "Updater signature file is empty: '$signaturePath'."
}

# The manifest references the versioned release asset (not `latest`) using the
# exact published installer file name.
$setupAssetName = $setup.Name
$downloadUrl = "https://github.com/$Repository/releases/download/v$Version/$setupAssetName"
$pubDate = [System.DateTimeOffset]::UtcNow.ToString(
    "yyyy-MM-ddTHH:mm:ssZ",
    [System.Globalization.CultureInfo]::InvariantCulture
)

$updaterManifest = [ordered]@{
    version   = $Version
    pub_date  = $pubDate
    platforms = [ordered]@{
        "windows-x86_64" = [ordered]@{
            signature = $signature
            url       = $downloadUrl
        }
    }
}

$latestJsonPath = Join-Path $resolvedReleaseDir "latest.json"
[System.IO.File]::WriteAllText($latestJsonPath, ($updaterManifest | ConvertTo-Json -Depth 5))

# The signature is embedded in latest.json; do not publish the raw sidecar.
Remove-Item -LiteralPath $signaturePath -Force

# Append latest.json to the existing checksum file, keeping every prior line so
# published checksums stay consistent.
$checksumPath = Join-Path $resolvedReleaseDir "SHA256SUMS.txt"
if (-not (Test-Path -LiteralPath $checksumPath -PathType Leaf)) {
    throw "SHA256SUMS.txt not found in '$resolvedReleaseDir'."
}
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $latestJsonPath).Hash.ToLowerInvariant()
$existing = [System.IO.File]::ReadAllText($checksumPath)
if ($existing.Length -gt 0 -and -not $existing.EndsWith("`n")) {
    $existing += "`n"
}
[System.IO.File]::WriteAllText($checksumPath, "$existing$hash  latest.json`n")

Write-Host "Generated latest.json for v$Version and appended it to SHA256SUMS.txt."
