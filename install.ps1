#Requires -RunAsAdministrator
<#
.SYNOPSIS
    win-symlinks installer script.

.DESCRIPTION
    Copies ln.exe, win-symlinks.exe, and win-symlinks-broker.exe to the install
    directory, adds that directory to Machine PATH, registers and starts the
    WinSymlinksBroker service, then runs a quick smoke test to verify everything
    works.

.PARAMETER InstallDir
    Target installation directory. Defaults to "C:\Program Files\win-symlinks".

.EXAMPLE
    .\install.ps1
    .\install.ps1 -InstallDir "D:\Tools\win-symlinks"
#>
param(
    [string]$InstallDir = "C:\Program Files\win-symlinks"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Definition
$Binaries  = @("ln.exe", "win-symlinks.exe", "win-symlinks-broker.exe")

# ── Helpers ──────────────────────────────────────────────────────────────────

function Write-Step  { param([string]$Msg) Write-Host "`n==> $Msg" -ForegroundColor Cyan }
function Write-Ok    { param([string]$Msg) Write-Host "    [OK] $Msg" -ForegroundColor Green }
function Write-Warn  { param([string]$Msg) Write-Host "    [WARN] $Msg" -ForegroundColor Yellow }
function Write-Fail  { param([string]$Msg) Write-Host "    [FAIL] $Msg" -ForegroundColor Red }

# ── 1. Pre-flight checks ────────────────────────────────────────────────────

Write-Step "Checking prerequisites"

foreach ($bin in $Binaries) {
    $src = Join-Path $ScriptDir $bin
    if (-not (Test-Path $src)) {
        Write-Fail "$bin not found next to install.ps1 ($ScriptDir)"
        exit 1
    }
}
Write-Ok "All binaries found in $ScriptDir"

# ── 2. Copy binaries ────────────────────────────────────────────────────────

Write-Step "Installing to $InstallDir"

if (-not (Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Force $InstallDir | Out-Null
    Write-Ok "Created directory $InstallDir"
}

foreach ($bin in $Binaries) {
    Copy-Item (Join-Path $ScriptDir $bin) $InstallDir -Force
    Write-Ok "Copied $bin"
}

# ── 3. Add to PATH ──────────────────────────────────────────────────────────

Write-Step "Configuring Machine PATH"

$machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
$entries = $machinePath -split ";" | Where-Object { $_ }

if ($entries -contains $InstallDir) {
    Write-Ok "PATH already contains $InstallDir"
} else {
    [Environment]::SetEnvironmentVariable("Path", "$machinePath;$InstallDir", "Machine")
    # Also update the current session so the rest of this script can find the binaries.
    $env:Path = "$env:Path;$InstallDir"
    Write-Ok "Added $InstallDir to Machine PATH"
    Write-Warn "Other terminals must be reopened to pick up the new PATH"
}

# ── 4. Install and start the broker service ──────────────────────────────────

Write-Step "Registering WinSymlinksBroker service"

$wsCli = Join-Path $InstallDir "win-symlinks.exe"

& $wsCli service install
if ($LASTEXITCODE -ne 0) {
    Write-Fail "'win-symlinks service install' failed (exit $LASTEXITCODE)"
    exit 1
}
Write-Ok "Service registered"

& $wsCli service start
if ($LASTEXITCODE -ne 0) {
    Write-Fail "'win-symlinks service start' failed (exit $LASTEXITCODE)"
    exit 1
}
Write-Ok "Service started"

& $wsCli service status
Write-Host ""

# ── 5. Smoke tests ──────────────────────────────────────────────────────────

Write-Step "Running verification tests"

$lnExe = Join-Path $InstallDir "ln.exe"
$testDir = Join-Path $env:TEMP "win-symlinks-install-test"
$testPass = 0
$testFail = 0

# Create a clean temp directory for tests.
if (Test-Path $testDir) { Remove-Item $testDir -Recurse -Force }
New-Item -ItemType Directory -Force $testDir | Out-Null

# 5a. doctor
Write-Host ""
Write-Host "    --- doctor ---" -ForegroundColor DarkGray
& $wsCli doctor
Write-Host ""

# 5b. File symlink
$targetFile = Join-Path $testDir "_ws_test_target.txt"
$linkFile   = Join-Path $testDir "_ws_test_link.txt"

"win-symlinks test content" | Set-Content $targetFile
& $lnExe -s $targetFile $linkFile 2>&1 | Out-Null

if ((Test-Path $linkFile) -and ((Get-Item $linkFile).LinkType -eq "SymbolicLink")) {
    Write-Ok "File symlink created and verified"
    $testPass++
} else {
    Write-Fail "File symlink test failed"
    $testFail++
}

# 5c. Directory symlink
$targetDir2 = Join-Path $testDir "_ws_test_target_dir"
$linkDir2   = Join-Path $testDir "_ws_test_link_dir"

New-Item -ItemType Directory -Force $targetDir2 | Out-Null
& $lnExe -s $targetDir2 $linkDir2 2>&1 | Out-Null

if ((Test-Path $linkDir2) -and ((Get-Item $linkDir2).LinkType -eq "SymbolicLink")) {
    Write-Ok "Directory symlink created and verified"
    $testPass++
} else {
    Write-Fail "Directory symlink test failed"
    $testFail++
}

# ── 6. Cleanup test artifacts ───────────────────────────────────────────────

Write-Step "Cleaning up test files"

# Remove links first (they are symlinks, not real content).
foreach ($p in @($linkFile, $linkDir2)) {
    if (Test-Path $p) {
        # Remove-Item on symlinks to directories needs special handling.
        $item = Get-Item $p -Force
        if ($item.PSIsContainer) {
            $item.Delete()
        } else {
            Remove-Item $p -Force
        }
    }
}
# Remove the whole test directory.
if (Test-Path $testDir) { Remove-Item $testDir -Recurse -Force }
Write-Ok "Test artifacts removed"

# ── Summary ──────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "================================================" -ForegroundColor Cyan
if ($testFail -eq 0) {
    Write-Host "  Installation complete.  Tests: $testPass passed, 0 failed." -ForegroundColor Green
} else {
    Write-Host "  Installation complete.  Tests: $testPass passed, $testFail failed." -ForegroundColor Yellow
}
Write-Host "  Install path : $InstallDir" -ForegroundColor Cyan
Write-Host "  Open a NEW terminal to use ln / win-symlinks." -ForegroundColor Cyan
Write-Host "================================================" -ForegroundColor Cyan
Write-Host ""
