<#
.SYNOPSIS
  Applies repository rulesets that encode the PR merge-method policy.

.DESCRIPTION
  develop: allow squash + merge (features squash; release back-sync must merge).
  main: allow merge only (promote, release-please, automation all use merge commits).

  Admin bypass is enabled so emergency recovery remains possible. Prefer the
  documented merge method and automation paths over bypassing rulesets.

.PARAMETER Repository
  owner/name. Defaults to the gh remote for the current directory.

.PARAMETER DryRun
  Print the planned ruleset payloads without calling the API.
#>
param(
    [string]$Repository = "",
    [switch]$DryRun
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Resolve-Repository {
    param([string]$Value)

    if (-not [string]::IsNullOrWhiteSpace($Value)) {
        return $Value.Trim()
    }

    $url = (& gh repo view --json nameWithOwner --jq .nameWithOwner 2>$null)
    if ($LASTEXITCODE -ne 0 -or [string]::IsNullOrWhiteSpace($url)) {
        throw "Could not resolve repository. Pass -Repository owner/name or run inside a gh-authenticated clone."
    }
    return $url.Trim()
}

function Get-RulesetIdByName {
    param(
        [string]$Repo,
        [string]$Name
    )

    $id = & gh api "repos/$Repo/rulesets" --jq ".[] | select(.name == `"$Name`") | .id"
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to list rulesets for $Repo."
    }
    if ([string]::IsNullOrWhiteSpace($id)) {
        return $null
    }
    return [int]$id
}

function Write-Ruleset {
    param(
        [string]$Repo,
        [string]$Name,
        [hashtable]$Payload
    )

    $json = $Payload | ConvertTo-Json -Depth 8 -Compress
    $existingId = Get-RulesetIdByName -Repo $Repo -Name $Name

    if ($DryRun) {
        if ($existingId) {
            Write-Host "DRY-RUN PUT repos/$Repo/rulesets/$existingId ($Name)"
        } else {
            Write-Host "DRY-RUN POST repos/$Repo/rulesets ($Name)"
        }
        Write-Host $json
        return
    }

    $temp = [System.IO.Path]::GetTempFileName()
    try {
        [System.IO.File]::WriteAllText($temp, $json, [System.Text.UTF8Encoding]::new($false))
        if ($existingId) {
            & gh api --method PUT "repos/$Repo/rulesets/$existingId" --input $temp
        } else {
            & gh api --method POST "repos/$Repo/rulesets" --input $temp
        }
        if ($LASTEXITCODE -ne 0) {
            throw "Failed to apply ruleset '$Name' on $Repo."
        }
        Write-Host "Applied ruleset '$Name' on $Repo."
    } finally {
        Remove-Item -LiteralPath $temp -Force -ErrorAction SilentlyContinue
    }
}

$repo = Resolve-Repository -Value $Repository

# RepositoryRole actor_id 5 = admin. Lets maintainers/automation recover without
# disabling the ruleset. Prefer normal merge paths over bypass.
$adminBypass = @(
    [ordered]@{
        actor_id = 5
        actor_type = 'RepositoryRole'
        bypass_mode = 'always'
    }
)

$developPayload = [ordered]@{
    name = 'develop-protection'
    target = 'branch'
    enforcement = 'active'
    bypass_actors = $adminBypass
    conditions = [ordered]@{
        ref_name = [ordered]@{
            include = @('refs/heads/develop')
            exclude = @()
        }
    }
    rules = @(
        [ordered]@{ type = 'deletion' }
        [ordered]@{ type = 'non_fast_forward' }
        [ordered]@{
            type = 'pull_request'
            parameters = [ordered]@{
                required_approving_review_count = 0
                dismiss_stale_reviews_on_push = $false
                require_code_owner_review = $false
                require_last_push_approval = $false
                required_review_thread_resolution = $false
                # Both methods: feature PRs squash; release back-sync must merge.
                allowed_merge_methods = @('squash', 'merge')
            }
        }
    )
}

$mainPayload = [ordered]@{
    name = 'main-protection'
    target = 'branch'
    enforcement = 'active'
    bypass_actors = $adminBypass
    conditions = [ordered]@{
        ref_name = [ordered]@{
            include = @('refs/heads/main')
            exclude = @()
        }
    }
    rules = @(
        [ordered]@{ type = 'deletion' }
        [ordered]@{ type = 'non_fast_forward' }
        [ordered]@{
            type = 'pull_request'
            parameters = [ordered]@{
                required_approving_review_count = 0
                dismiss_stale_reviews_on_push = $false
                require_code_owner_review = $false
                require_last_push_approval = $false
                required_review_thread_resolution = $false
                # Merge only: promote / release-please / automation all use --merge.
                allowed_merge_methods = @('merge')
            }
        }
        [ordered]@{
            type = 'required_status_checks'
            parameters = [ordered]@{
                strict_required_status_checks_policy = $false
                do_not_enforce_on_create = $false
                required_status_checks = @(
                    [ordered]@{ context = 'Frontend quality' }
                    [ordered]@{ context = 'Windows cross-build' }
                )
            }
        }
    )
}

Write-Ruleset -Repo $repo -Name 'develop-protection' -Payload $developPayload
Write-Ruleset -Repo $repo -Name 'main-protection' -Payload $mainPayload

if (-not $DryRun) {
    Write-Host "Ensuring repository-level merge methods stay compatible with rulesets..."
    & gh api --method PATCH "repos/$repo" -f allow_merge_commit=true -f allow_squash_merge=true -f allow_rebase_merge=false | Out-Null
    if ($LASTEXITCODE -ne 0) {
        throw "Failed to update repository merge method flags on $repo."
    }
    Write-Host "Repository merge flags: merge=true, squash=true, rebase=false."
}

Write-Host "Merge policy rulesets are up to date."
