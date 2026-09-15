#Requires -RunAsAdministrator
<#
.SYNOPSIS
    Install the broker-only WinSymlinksBroker runtime.

.DESCRIPTION
    Copies win-symlinks-broker.exe to a stable directory, registers the
    WinSymlinksBroker Windows service as LocalSystem with delayed automatic
    startup, and starts the service unless -NoStart is supplied.

.PARAMETER InstallDir
    Target installation directory. Defaults to
    "C:\Program Files\win-symlinks-broker".

.PARAMETER NoStart
    Register or update the service without starting it.

.EXAMPLE
    .\install-broker.ps1
    .\install-broker.ps1 -InstallDir "C:\Program Files\AgentLinker\Broker"
#>
[CmdletBinding()]
param(
    [string]$InstallDir = "$env:ProgramFiles\win-symlinks-broker",
    [switch]$NoStart
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ServiceName = "WinSymlinksBroker"
$DisplayName = "Win Symlinks Broker"
$Description = "Privileged local broker for creating real Windows symbolic links."
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$SourceBroker = Join-Path $ScriptDir "win-symlinks-broker.exe"

function Write-Step { param([string]$Message) Write-Host "`n==> $Message" -ForegroundColor Cyan }
function Write-Ok { param([string]$Message) Write-Host "    [OK] $Message" -ForegroundColor Green }
function Write-Fail { param([string]$Message) Write-Host "    [FAIL] $Message" -ForegroundColor Red }

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

function Stop-BrokerServiceIfNeeded {
    $service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
    if ($null -eq $service) {
        return
    }

    if ($service.Status -ne "Stopped") {
        Write-Step "Stopping existing $ServiceName service"
        Stop-Service -Name $ServiceName -Force
        $service.WaitForStatus("Stopped", [TimeSpan]::FromSeconds(30))
        Write-Ok "Service stopped"
    }
}

Write-Step "Checking broker runtime package"
if (-not (Test-Path -LiteralPath $SourceBroker -PathType Leaf)) {
    Write-Fail "win-symlinks-broker.exe was not found next to install-broker.ps1"
    exit 1
}
Write-Ok "Found win-symlinks-broker.exe"

Write-Step "Installing broker binary"
New-Item -ItemType Directory -Force $InstallDir | Out-Null
$DestinationBroker = Join-Path $InstallDir "win-symlinks-broker.exe"
Stop-BrokerServiceIfNeeded
Copy-Item -LiteralPath $SourceBroker -Destination $DestinationBroker -Force
Write-Ok "Copied broker to $DestinationBroker"

Write-Step "Registering $ServiceName"
$quotedBrokerPath = "`"$DestinationBroker`""
$service = Get-Service -Name $ServiceName -ErrorAction SilentlyContinue
if ($null -eq $service) {
    Invoke-Sc -Action "create service" -Arguments @(
        "create",
        $ServiceName,
        "binPath= $quotedBrokerPath",
        "start= delayed-auto",
        "obj= LocalSystem",
        "DisplayName= $DisplayName"
    )
    Write-Ok "Service created"
} else {
    Invoke-Sc -Action "configure service" -Arguments @(
        "config",
        $ServiceName,
        "binPath= $quotedBrokerPath",
        "start= delayed-auto",
        "obj= LocalSystem",
        "DisplayName= $DisplayName"
    )
    Write-Ok "Service updated"
}

Invoke-Sc -Action "set service description" -Arguments @(
    "description",
    $ServiceName,
    $Description
)

if (-not $NoStart) {
    Write-Step "Starting $ServiceName"
    Start-Service -Name $ServiceName
    (Get-Service -Name $ServiceName).WaitForStatus("Running", [TimeSpan]::FromSeconds(30))
    Write-Ok "Service running"
}

Write-Host ""
Write-Host "WinSymlinksBroker runtime installation complete." -ForegroundColor Green
Write-Host "Install path : $InstallDir" -ForegroundColor Cyan
Write-Host "Service name : $ServiceName" -ForegroundColor Cyan
