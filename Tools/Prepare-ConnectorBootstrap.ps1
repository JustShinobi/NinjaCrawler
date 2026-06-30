param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Net.Http

function Get-RepoRoot {
    return Split-Path -Parent $PSScriptRoot
}

function Invoke-DownloadRequest {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Uri,
        [Parameter(Mandatory = $true)]
        [string]$OutFile
    )

    $handler = [System.Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $true

    $client = [System.Net.Http.HttpClient]::new($handler)
    try {
        $client.DefaultRequestHeaders.UserAgent.ParseAdd("NinjaCrawler-Bootstrap/0.1.0")
        $client.DefaultRequestHeaders.Accept.ParseAdd("application/octet-stream")

        $response = $client.GetAsync($Uri).GetAwaiter().GetResult()
        if (-not $response.IsSuccessStatusCode) {
            throw "Download falhou para '$Uri' com status $([int]$response.StatusCode) $($response.ReasonPhrase)."
        }

        $destinationDirectory = Split-Path -Parent $OutFile
        New-Item -ItemType Directory -Force -Path $destinationDirectory | Out-Null

        $bytes = $response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult()
        [System.IO.File]::WriteAllBytes($OutFile, $bytes)
    } finally {
        $client.Dispose()
        $handler.Dispose()
    }
}

function Expand-ZipMember {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ArchivePath,
        [Parameter(Mandatory = $true)]
        [string]$MemberName,
        [Parameter(Mandatory = $true)]
        [string]$DestinationPath
    )

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($ArchivePath)
    try {
        $entry = $archive.Entries | Where-Object { $_.FullName -ieq $MemberName } | Select-Object -First 1
        if (-not $entry) {
            throw "Arquivo '$MemberName' nao encontrado em '$ArchivePath'."
        }

        $destinationDirectory = Split-Path -Parent $DestinationPath
        New-Item -ItemType Directory -Force -Path $destinationDirectory | Out-Null

        $source = $entry.Open()
        try {
            $target = [System.IO.File]::Open($DestinationPath, [System.IO.FileMode]::Create, [System.IO.FileAccess]::Write)
            try {
                $source.CopyTo($target)
            } finally {
                $target.Dispose()
            }
        } finally {
            $source.Dispose()
        }
    } finally {
        $archive.Dispose()
    }
}

$repoRoot = Get-RepoRoot
$manifestPath = Join-Path $repoRoot "connectors\bootstrap\manifest.json"
if (-not (Test-Path $manifestPath)) {
    throw "Manifesto de bootstrap nao encontrado em '$manifestPath'."
}

$manifest = Get-Content $manifestPath -Raw | ConvertFrom-Json
$bootstrapRoot = Join-Path $repoRoot "connectors\bootstrap"
$tempRoot = Join-Path $repoRoot "Temp\connector-bootstrap"
New-Item -ItemType Directory -Force -Path $tempRoot | Out-Null

foreach ($connector in $manifest.connectors) {
    $releaseTag = if ($connector.PSObject.Properties.Name -contains 'releaseTag') {
        [string]$connector.releaseTag
    } else {
        [string]$connector.bundledVersion
    }

    $targetDirectory = Join-Path $bootstrapRoot ([System.IO.Path]::Combine([string]$connector.key, [string]$connector.bundledVersion))
    $targetPath = Join-Path $targetDirectory ([string]$connector.executableName)
    if (Test-Path $targetPath) {
        Write-Host "Bootstrap pronto: $targetPath"
        continue
    }

    $repositoryPath = [string]$connector.releaseApiUrl `
        -replace '^https://api.github.com/repos/', '' `
        -replace '/releases(?:/latest)?$', ''
    if ($repositoryPath -notmatch '^[^/]+/[^/]+$') {
        throw "URL de releases invalida para '$($connector.key)': '$($connector.releaseApiUrl)'."
    }

    $assetName = [string]$connector.assetName
    $downloadUrl = "https://github.com/$repositoryPath/releases/download/$releaseTag/$assetName"
    if (
        $connector.PSObject.Properties.Name -contains 'assetPrefix' -and
        $connector.PSObject.Properties.Name -contains 'assetSuffix'
    ) {
        $releaseUrl = "$($connector.releaseApiUrl.TrimEnd('/'))/tags/$releaseTag"
        # Authenticate the GitHub API call when a token is present (CI). Anonymous
        # requests share a 60/hr-per-IP limit and intermittently fail on hosted
        # runners; an authenticated token raises this to 5,000/hr. Falls back to
        # anonymous locally where no token is set.
        $headers = @{
            "Accept" = "application/vnd.github+json"
            "User-Agent" = "NinjaCrawler-Bootstrap/0.1.0"
        }
        $bootstrapToken = if ($env:GITHUB_TOKEN) { $env:GITHUB_TOKEN } elseif ($env:GH_TOKEN) { $env:GH_TOKEN } else { $null }
        if ($bootstrapToken) {
            $headers["Authorization"] = "Bearer $bootstrapToken"
        }
        $release = Invoke-RestMethod -Uri $releaseUrl -Headers $headers
        $assets = @($release.assets | Where-Object {
            $_.name.StartsWith([string]$connector.assetPrefix) -and
            $_.name.EndsWith([string]$connector.assetSuffix)
        })
        if ($assets.Count -ne 1) {
            throw "Esperado um asset para '$($connector.key)', encontrados $($assets.Count) em '$releaseUrl'."
        }

        $assetName = [string]$assets[0].name
        $downloadUrl = [string]$assets[0].browser_download_url
    }
    $downloadPath = Join-Path $tempRoot $assetName

    Write-Host "Baixando $assetName..."
    Invoke-DownloadRequest -Uri $downloadUrl -OutFile $downloadPath

    if ($connector.PSObject.Properties.Name -contains 'archiveMemberName' -and $connector.archiveMemberName) {
        Expand-ZipMember -ArchivePath $downloadPath -MemberName ([string]$connector.archiveMemberName) -DestinationPath $targetPath
    } else {
        New-Item -ItemType Directory -Force -Path $targetDirectory | Out-Null
        Copy-Item -LiteralPath $downloadPath -Destination $targetPath -Force
    }

    Write-Host "Bootstrap preparado: $targetPath"
}
