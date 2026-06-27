param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern("^\d+\.\d+\.\d+$")]
    [string]$Version,
    [string]$CurrentRef = "HEAD",
    [string]$CurrentTag,
    [string]$Repository,
    [string]$OutputPath = "release\CHANGELOG.md"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Invoke-Git {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments
    )

    $output = @(& git @Arguments)
    if ($LASTEXITCODE -ne 0) {
        throw "Git command failed: git $($Arguments -join ' ')"
    }

    return $output
}

function Resolve-RepositoryName {
    if ($Repository) {
        return $Repository.Trim().TrimEnd("/")
    }
    if ($env:GITHUB_REPOSITORY) {
        return $env:GITHUB_REPOSITORY.Trim().TrimEnd("/")
    }

    $remoteUrl = (Invoke-Git -Arguments @("remote", "get-url", "origin") | Select-Object -First 1)
    if ($remoteUrl -match "github\.com(?::|/)(?<repository>[^/]+/[^/.]+)(?:\.git)?$") {
        return $Matches.repository
    }

    return ""
}

function Get-SectionName {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Type
    )

    switch ($Type) {
        "feat" { return "Features" }
        "fix" { return "Fixes" }
        "perf" { return "Performance" }
        "docs" { return "Documentation" }
        { $_ -in @("build", "ci") } { return "Build and CI" }
        { $_ -in @("refactor", "test", "chore") } { return "Maintenance" }
        default { return "Other changes" }
    }
}

$currentCommit = (
    Invoke-Git -Arguments @("rev-parse", "--verify", "$CurrentRef`^{commit}") |
        Select-Object -First 1
).Trim()
$releaseDate = (
    Invoke-Git -Arguments @("show", "-s", "--format=%cs", $currentCommit) |
        Select-Object -First 1
).Trim()
$repositoryName = Resolve-RepositoryName
$repositoryUrl = if ($repositoryName) { "https://github.com/$repositoryName" } else { "" }

$versionTags = @(
    Invoke-Git -Arguments @("tag", "--merged", $currentCommit, "--sort=-version:refname") |
        Where-Object { $_ -match "^v\d+\.\d+\.\d+$" }
)
$previousTag = $versionTags |
    Where-Object { -not $CurrentTag -or $_ -ne $CurrentTag } |
    Select-Object -First 1
$logRange = if ($previousTag) { "$previousTag..$currentCommit" } else { $currentCommit }

$format = "%H%x1f%s%x1f%b%x1e"
$rawLog = (Invoke-Git -Arguments @("log", "--no-merges", "--format=$format", $logRange)) -join "`n"
$records = @($rawLog -split [char]0x1e | Where-Object { $_.Trim() })

$sectionOrder = @(
    "Breaking changes",
    "Features",
    "Fixes",
    "Performance",
    "Documentation",
    "Build and CI",
    "Maintenance",
    "Other changes"
)
$sections = [ordered]@{}
foreach ($sectionName in $sectionOrder) {
    $sections[$sectionName] = [System.Collections.Generic.List[string]]::new()
}

foreach ($record in $records) {
    $fields = $record.Trim() -split [char]0x1f, 3
    if ($fields.Count -lt 2) {
        continue
    }

    $commit = $fields[0].Trim()
    $subject = $fields[1].Trim()
    $body = if ($fields.Count -ge 3) { $fields[2].Trim() } else { "" }
    $shortCommit = $commit.Substring(0, [Math]::Min(7, $commit.Length))
    $commitReference = if ($repositoryUrl) {
        "([$shortCommit]($repositoryUrl/commit/$commit))"
    } else {
        "($shortCommit)"
    }

    $type = ""
    $scope = ""
    $description = $subject
    $isBreaking = $body -match "(?im)^BREAKING[ -]CHANGE:"
    if ($subject -match "^(?<type>feat|fix|perf|refactor|docs|build|ci|test|chore)(?:\((?<scope>[^)]+)\))?(?<breaking>!)?:\s*(?<description>.+)$") {
        $type = [string]$Matches["type"]
        $scope = if ($Matches.ContainsKey("scope")) { [string]$Matches["scope"] } else { "" }
        $description = [string]$Matches["description"]
        $breakingMarker = if ($Matches.ContainsKey("breaking")) { [string]$Matches["breaking"] } else { "" }
        $isBreaking = $isBreaking -or $breakingMarker -eq "!"
    }

    $prefix = if ($scope) { "**${scope}:** " } else { "" }
    $line = "- $prefix$description $commitReference"
    $sectionName = if ($isBreaking) { "Breaking changes" } else { Get-SectionName -Type $type }
    $sections[$sectionName].Add($line)
}

$lines = [System.Collections.Generic.List[string]]::new()
$lines.Add("# NinjaCrawler $Version")
$lines.Add("")
$lines.Add("_Released on $releaseDate._")
$lines.Add("")

foreach ($sectionName in $sectionOrder) {
    if ($sections[$sectionName].Count -eq 0) {
        continue
    }

    $lines.Add("## $sectionName")
    $lines.Add("")
    foreach ($entry in $sections[$sectionName]) {
        $lines.Add($entry)
    }
    $lines.Add("")
}

if ($repositoryUrl -and $CurrentTag) {
    if ($previousTag) {
        $lines.Add("[Full changelog]($repositoryUrl/compare/$previousTag...$CurrentTag)")
    } else {
        $lines.Add("[Full commit history]($repositoryUrl/commits/$CurrentTag)")
    }
    $lines.Add("")
}

$resolvedOutputPath = [System.IO.Path]::GetFullPath((Join-Path (Get-Location) $OutputPath))
$outputDirectory = Split-Path -Parent $resolvedOutputPath
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
$utf8WithoutBom = New-Object System.Text.UTF8Encoding($false)
[System.IO.File]::WriteAllText($resolvedOutputPath, ($lines -join "`n"), $utf8WithoutBom)

Write-Host "Changelog=$resolvedOutputPath"
Write-Host "CommitRange=$logRange"
Write-Host "CommitCount=$($records.Count)"
if ($previousTag) {
    Write-Host "PreviousTag=$previousTag"
} else {
    Write-Host "PreviousTag=(initial release)"
}
