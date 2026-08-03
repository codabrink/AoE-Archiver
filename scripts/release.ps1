<#
.SYNOPSIS
    Creates a git tag and pushes it to trigger the release workflow.

.PARAMETER Version
    The version tag to create, e.g. v1.15.0.

.EXAMPLE
    ./scripts/release.ps1 v1.15.0
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [string]$Version
)

$ErrorActionPreference = "Stop"

git tag $Version
if ($LASTEXITCODE -ne 0) { throw "git tag failed with exit code $LASTEXITCODE" }

git push origin $Version
if ($LASTEXITCODE -ne 0) { throw "git push failed with exit code $LASTEXITCODE" }

Write-Host "Pushed tag $Version - release workflow will run."
