param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^\d+\.\d+\.\d+$')]
    [string]$Version,

    [string]$Path
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($Path)) {
    $Path = Join-Path $PSScriptRoot "..\README.md"
}

$resolvedPath = (Resolve-Path -LiteralPath $Path).Path
$content = Get-Content -LiteralPath $resolvedPath -Raw
$blockPattern = '(?ms)(?<start>^[ \t]*<!--\s*ninjacrawler-release-start\s*-->[^\r\n]*\r?\n)(?<body>.*?)(?<end>^[ \t]*<!--\s*ninjacrawler-release-end\s*-->[^\r\n]*$)'
$versionPattern = '(?<!\d)\d+\.\d+\.\d+(?![\d.])'
$blocks = [regex]::Matches($content, $blockPattern)

if ($blocks.Count -ne 2) {
    throw "Expected exactly 2 NinjaCrawler release blocks in '$resolvedPath', found $($blocks.Count)."
}

$updated = [regex]::Replace(
    $content,
    $blockPattern,
    {
        param($match)

        $versions = [regex]::Matches($match.Groups['body'].Value, $versionPattern)
        if ($versions.Count -eq 0) {
            throw "A NinjaCrawler release block in '$resolvedPath' contains no semantic versions."
        }

        $body = [regex]::Replace($match.Groups['body'].Value, $versionPattern, $Version)
        return $match.Groups['start'].Value + $body + $match.Groups['end'].Value
    }
)

$expectedLinks = @(
    "releases/download/v$Version/NinjaCrawler-$Version-windows-x64-setup.exe",
    "releases/download/v$Version/NinjaCrawler-$Version-windows-x64-portable.exe",
    "releases/download/v$Version/SHA256SUMS.txt"
)
foreach ($expectedLink in $expectedLinks) {
    if (-not $updated.Contains($expectedLink)) {
        throw "Updated README is missing expected release link '$expectedLink'."
    }
}

foreach ($block in [regex]::Matches($updated, $blockPattern)) {
    $unexpectedVersions = @(
        [regex]::Matches($block.Groups['body'].Value, $versionPattern) |
            ForEach-Object Value |
            Where-Object { $_ -ne $Version } |
            Select-Object -Unique
    )
    if ($unexpectedVersions.Count -gt 0) {
        throw "Release block still contains unexpected versions: $($unexpectedVersions -join ', ')."
    }
}

if ($updated -eq $content) {
    Write-Host "README release links already target v$Version."
    exit 0
}

[System.IO.File]::WriteAllText(
    $resolvedPath,
    $updated,
    [System.Text.UTF8Encoding]::new($false)
)
Write-Host "Updated README release links to v$Version."
