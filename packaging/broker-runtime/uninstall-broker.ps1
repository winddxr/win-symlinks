#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Uninstall the broker-only WinSymlinksBroker runtime.

.DESCRIPTION
    Stops and deletes the WinSymlinksBroker Windows service. By default it also
    removes win-symlinks-broker.exe from the installation directory, then removes
    the directory only if it is empty.

.PARAMETER InstallDir
    Broker installation directory. Defaults to
    "C:\Program Files\win-symlinks-broker".

.PARAMETER KeepFiles
    Remove only the Windows service registration and leave installed files in
    place.

.EXAMPLE
    .\uninstall-broker.ps1
    .\uninstall-broker.ps1 -KeepFiles
#>
[CmdletBinding()]
param(
    [string]$InstallDir = "$env:ProgramFiles\win-symlinks-broker",
    [switch]$KeepFiles
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ServiceName = "WinSymlinksBroker"

function Write-Step { param([string]$Message) Write-Host "`n==> $Message" -ForegroundColor Cyan }
function Write-Ok { param([string]$Message) Write-Host "    [OK] $Message" -ForegroundColor Green }
function Write-Warn { param([string]$Message) Write-Host "    [WARN] $Message" -ForegroundColor Yellow }

function Invoke-Sc {
    param(
        [Parameter(Mandatory = $true)]
        [string[]]$Arguments,
        [Parameter(Mandatory = $true)]
        [string]$Action
    )

    $output = & sc.exe @Arguments 2>&1
    if ($LASTEXITCODE -ne 0) {
        throw "$Action failed with exit code $LASTEXITCODE`n$output"
    }
}

Write-Step "Removing $ServiceName service"
$service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
if ($null -eq $service) {
    Write-Warn "$ServiceName is not installed"
} else {
    if ($service.Status -ne "Stopped") {
        Stop-Service -Name $ServiceName -Force
        $service.WaitForStatus("Stopped", [TimeSpan]::FromSeconds(30))
        Write-Ok "Service stopped"
    }

    Invoke-Sc -Action "delete service" -Arguments @("delete", $ServiceName)
    Write-Ok "Service deleted"
}

if ($KeepFiles) {
    Write-Warn "Keeping installed broker files"
    exit 0
}

Write-Step "Removing broker binary"
$brokerPath = Join-Path $InstallDir "win-symlinks-broker.exe"
if (Test-Path -LiteralPath $brokerPath -PathType Leaf) {
    Remove-Item -LiteralPath $brokerPath -Force
    Write-Ok "Removed $brokerPath"
} else {
    Write-Warn "No broker binary found at $brokerPath"
}

if (Test-Path -LiteralPath $InstallDir -PathType Container) {
    $remainingItems = Get-ChildItem -LiteralPath $InstallDir -Force
    if ($remainingItems.Count -eq 0) {
        Remove-Item -LiteralPath $InstallDir -Force
        Write-Ok "Removed empty directory $InstallDir"
    } else {
        Write-Warn "Left non-empty directory in place: $InstallDir"
    }
}

Write-Host ""
Write-Host "WinSymlinksBroker runtime uninstall complete." -ForegroundColor Green
