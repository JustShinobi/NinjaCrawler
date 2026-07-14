param(
    [ValidateSet("Debug", "Release")]
    [string]$Configuration = "Debug",
    [switch]$SkipLint,
    [switch]$SkipTests,
    [switch]$PortableOnly,
    [string]$TargetTriple = ""
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$runningOnWindows = $env:OS -eq "Windows_NT"
if ([string]::IsNullOrWhiteSpace($TargetTriple) -and -not $runningOnWindows) {
    $TargetTriple = "x86_64-pc-windows-msvc"
}

function Invoke-DesktopCommand {
    param([Parameter(Mandatory)][string[]]$Command)

    Write-Host ("> " + ($Command -join " "))
    Push-Location $repoRoot
    try {
        if ($runningOnWindows) {
            $runner = Join-Path $PSScriptRoot "Run-InVsDevCmd.cmd"
            if (-not (Test-Path -LiteralPath $runner -PathType Leaf)) {
                throw "Run-InVsDevCmd.cmd was not found in '$PSScriptRoot'."
            }
            & $runner @Command
        } else {
            $executable = $Command[0]
            $arguments = @($Command | Select-Object -Skip 1)
            & $executable @arguments
        }
        if ($LASTEXITCODE -ne 0) {
            throw "Command failed: $($Command -join ' ')"
        }
    } finally {
        Pop-Location
    }
}

function Get-TargetRoot {
    $root = Join-Path $repoRoot "src-tauri/target"
    if (-not [string]::IsNullOrWhiteSpace($TargetTriple)) {
        $root = Join-Path $root $TargetTriple
    }
    return Join-Path $root $Configuration.ToLowerInvariant()
}

if (-not $SkipLint) {
    Invoke-DesktopCommand @("npm", "run", "lint")
}
if (-not $SkipTests) {
    Invoke-DesktopCommand @("npm", "test")
}

$tauriArguments = @()
if ($Configuration -eq "Debug") {
    $tauriArguments += "--debug"
}
if (-not [string]::IsNullOrWhiteSpace($TargetTriple)) {
    $tauriArguments += @("--runner", "cargo-xwin", "--target", $TargetTriple)
}
if ($PortableOnly) {
    $tauriArguments += "--no-bundle"
} else {
    $tauriArguments += @("--bundles", "nsis")
}

$buildCommand = @("npm", "run", "tauri:build", "--") + $tauriArguments
Invoke-DesktopCommand $buildCommand

$targetRoot = Get-TargetRoot
$executablePath = Join-Path $targetRoot "ninjacrawler.exe"
if (-not (Test-Path -LiteralPath $executablePath -PathType Leaf)) {
    throw "The build did not produce '$executablePath'."
}

Write-Host "BuildRoot=$targetRoot"
Write-Host "PortableArtifact=$executablePath"
if (-not $PortableOnly) {
    $nsisArtifacts = @(Get-ChildItem -LiteralPath (Join-Path $targetRoot "bundle/nsis") -Filter "*-setup.exe" -File -ErrorAction SilentlyContinue)
    if ($nsisArtifacts.Count -eq 0) {
        throw "The build did not produce an NSIS installer below '$targetRoot/bundle/nsis'."
    }
    $nsisArtifacts | ForEach-Object { Write-Host "NsisArtifact=$($_.FullName)" }
}
