Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$generator = Join-Path $PSScriptRoot 'New-NinjaCrawlerChangelog.ps1'
$fixtureRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("ninjacrawler-changelog-" + [guid]::NewGuid())

function Invoke-FixtureGit {
    param([Parameter(Mandatory)][string[]]$Arguments)

    $output = @(& git -C $fixtureRoot @Arguments 2>&1)
    if ($LASTEXITCODE -ne 0) {
        throw "Fixture git $($Arguments -join ' ') failed: $($output -join [Environment]::NewLine)"
    }
    return $output
}

function New-FixtureCommit {
    param(
        [Parameter(Mandatory)][string]$Message,
        [Parameter(Mandatory)][string]$Content
    )

    Set-Content -LiteralPath (Join-Path $fixtureRoot 'fixture.txt') -Value $Content -Encoding utf8
    Invoke-FixtureGit -Arguments @('add', 'fixture.txt') | Out-Null
    Invoke-FixtureGit -Arguments @('commit', '-m', $Message) | Out-Null
}

try {
    New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
    Invoke-FixtureGit -Arguments @('init') | Out-Null
    Invoke-FixtureGit -Arguments @('config', 'user.name', 'Changelog Test') | Out-Null
    Invoke-FixtureGit -Arguments @('config', 'user.email', 'changelog-test@example.com') | Out-Null

    New-FixtureCommit -Message 'chore: initialize fixture' -Content 'initial'
    Invoke-FixtureGit -Arguments @('tag', 'v1.0.0') | Out-Null
    New-FixtureCommit `
        -Message 'fix(companion): stop treating Instagram highlights as @highlights' `
        -Content 'mention'
    New-FixtureCommit `
        -Message 'docs: keep support@example.com as an email address' `
        -Content 'email'

    Push-Location $fixtureRoot
    try {
        & $generator `
            -Version '1.0.1' `
            -CurrentRef 'HEAD' `
            -CurrentTag 'v1.0.1' `
            -Repository 'JustShinobi/NinjaCrawler' `
            -OutputPath 'CHANGELOG.test.md' | Out-Null
    } finally {
        Pop-Location
    }

    $changelog = Get-Content -LiteralPath (Join-Path $fixtureRoot 'CHANGELOG.test.md') -Raw
    if (-not $changelog.Contains('as `@highlights`')) {
        throw 'Technical GitHub handles must be protected from mention expansion in release notes.'
    }
    if ($changelog.Contains('as @highlights')) {
        throw 'Release notes still contain a raw technical GitHub mention.'
    }
    if (-not $changelog.Contains('support@example.com')) {
        throw 'Mention protection must not rewrite email addresses.'
    }

    Write-Host 'NinjaCrawler changelog tests passed.'
} finally {
    if (Test-Path -LiteralPath $fixtureRoot) {
        Remove-Item -LiteralPath $fixtureRoot -Recurse -Force
    }
}
