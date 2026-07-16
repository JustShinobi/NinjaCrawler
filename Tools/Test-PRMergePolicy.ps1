Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$policyScript = Join-Path $PSScriptRoot "Get-PRMergePolicy.ps1"

function Invoke-Policy {
    param(
        [string]$Base,
        [string]$Head
    )

    $json = & $policyScript -Base $Base -Head $Head
    if ($null -eq $json -or [string]::IsNullOrWhiteSpace([string]$json)) {
        throw "Get-PRMergePolicy.ps1 returned no policy for $Head → $Base."
    }
    return $json | ConvertFrom-Json
}

function Assert-Policy {
    param(
        [string]$Base,
        [string]$Head,
        [string]$Method,
        [string]$Label,
        [bool]$Critical
    )

    $policy = Invoke-Policy -Base $Base -Head $Head
    if ($policy.method -ne $Method) {
        throw "Expected method '$Method' for $Head → $Base, got '$($policy.method)'."
    }
    if ($policy.label -ne $Label) {
        throw "Expected label '$Label' for $Head → $Base, got '$($policy.label)'."
    }
    if ([bool]$policy.critical -ne $Critical) {
        throw "Expected critical=$Critical for $Head → $Base, got '$($policy.critical)'."
    }
    if ([string]::IsNullOrWhiteSpace($policy.reason) -or [string]::IsNullOrWhiteSpace($policy.how)) {
        throw "Policy for $Head → $Base must include reason and how guidance."
    }
}

$cases = @(
    @{ Base = 'main'; Head = 'develop'; Method = 'merge-commit'; Label = 'merge:merge-commit'; Critical = $true }
    @{ Base = 'develop'; Head = 'sync/release-v0.21.0'; Method = 'merge-commit'; Label = 'merge:merge-commit'; Critical = $true }
    @{ Base = 'develop'; Head = 'sync/release-companion-v0.16.0'; Method = 'merge-commit'; Label = 'merge:merge-commit'; Critical = $true }
    @{ Base = 'main'; Head = 'release-please--branches--main--components--ninjacrawler'; Method = 'merge-commit'; Label = 'merge:merge-commit'; Critical = $true }
    @{ Base = 'main'; Head = 'release-please--branches--main--components--companion'; Method = 'merge-commit'; Label = 'merge:merge-commit'; Critical = $true }
    @{ Base = 'main'; Head = 'automation/readme-release-v0.21.0'; Method = 'merge-commit'; Label = 'merge:merge-commit'; Critical = $false }
    @{ Base = 'develop'; Head = 'feat/merge-policy-bot'; Method = 'squash'; Label = 'merge:squash'; Critical = $false }
    @{ Base = 'develop'; Head = 'dependabot/github_actions/develop/actions/setup-node-7.0.0'; Method = 'squash'; Label = 'merge:squash'; Critical = $false }
    @{ Base = 'main'; Head = 'ci/register-release-workflows'; Method = 'merge-commit'; Label = 'merge:merge-commit'; Critical = $false }
    @{ Base = 'refs/heads/main'; Head = 'refs/heads/develop'; Method = 'merge-commit'; Label = 'merge:merge-commit'; Critical = $true }
)

foreach ($case in $cases) {
    Assert-Policy @case
}

$workflowPath = Join-Path $repoRoot ".github\workflows\merge-policy.yml"
if (-not (Test-Path -LiteralPath $workflowPath)) {
    throw "Missing merge-policy workflow at $workflowPath."
}
$workflow = Get-Content -LiteralPath $workflowPath -Raw
foreach ($fragment in @(
    'Get-PRMergePolicy.ps1',
    'merge:squash',
    'merge:merge-commit',
    '<!-- merge-policy-bot -->',
    'pull_request:'
)) {
    if (-not $workflow.Contains($fragment)) {
        throw "merge-policy workflow is missing required fragment: $fragment"
    }
}

$applyScript = Join-Path $PSScriptRoot "Apply-MergePolicyRulesets.ps1"
if (-not (Test-Path -LiteralPath $applyScript)) {
    throw "Missing Apply-MergePolicyRulesets.ps1."
}
$apply = Get-Content -LiteralPath $applyScript -Raw
foreach ($fragment in @(
    'develop-protection',
    'main-protection',
    'allowed_merge_methods',
    'squash',
    'merge'
)) {
    if (-not $apply.Contains($fragment)) {
        throw "Apply-MergePolicyRulesets.ps1 is missing required fragment: $fragment"
    }
}

Write-Host "PR merge policy tests passed."
