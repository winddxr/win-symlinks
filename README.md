# win-symlinks

> **English** | [简体中文](README.zh-CN.md)

`win-symlinks` 为 Windows 11 提供类似 Linux 的 `ln -s TARGET LINK_NAME` 体验，使用真实的 Windows 符号链接。

The project does not emulate symlinks with junctions, hardlinks, file copies, or
`.lnk` shortcuts. If a real symbolic link cannot be created, the command fails
with a clear diagnostic.

## Overview

The package builds three executables:

| Executable | Purpose |
| --- | --- |
| `ln.exe` | User-facing Linux-compatible command for creating symbolic links. |
| `win-symlinks.exe` | Management and diagnostic command. |
| `win-symlinks-broker.exe` | Privileged Windows service host for `WinSymlinksBroker`. |

Normal operation:

```text
User shell -> ln.exe -> Named Pipe -> WinSymlinksBroker -> CreateSymbolicLinkW
```

`ln.exe` may try direct true symlink creation first. If direct creation fails
because of privilege, it uses the broker service. Both paths preserve true
Windows symbolic link semantics.

## Requirements

- Windows 11.
- Rust 1.82 or newer.
- Cargo.
- Administrator rights for service installation, service start/stop, and service
  removal.
- A filesystem and Windows policy configuration that support symbolic links.

The broker service is intended for machines where Developer Mode is disabled or
the current user does not have `SeCreateSymbolicLinkPrivilege`.

## Build

Build all binaries:

```powershell
cargo build --release
```

The release executables will be created under:

```text
target\release\
```

Useful development checks:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

Run the CLIs from the build directory:

```powershell
cargo run --bin ln -- --help
cargo run --bin win-symlinks -- --help
```

## Install And Configure

The recommended no-installer setup is to copy the three release executables to a
stable installation directory, add that directory to `PATH`, then register and
start the broker service from that same directory.

### 1. Build Release Binaries

```powershell
cargo build --release
```

### 2. Copy Binaries To A Stable Directory

Run PowerShell as Administrator:

```powershell
$install = "C:\Program Files\win-symlinks"
New-Item -ItemType Directory -Force $install
Copy-Item target\release\ln.exe $install -Force
Copy-Item target\release\win-symlinks.exe $install -Force
Copy-Item target\release\win-symlinks-broker.exe $install -Force
```

Do not use `target\release` as the long-term installation directory. The Windows
service registration stores the broker executable path, so it should point at a
stable location.

### 3. Add The Install Directory To PATH

Machine-wide PATH, from an Administrator PowerShell:

```powershell
$install = "C:\Program Files\win-symlinks"
$machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
if (($machinePath -split ";") -notcontains $install) {
    [Environment]::SetEnvironmentVariable("Path", "$machinePath;$install", "Machine")
}
```

Open a new terminal after changing `PATH`.

`ln.exe` should appear before Git for Windows, MSYS2, Cygwin, BusyBox, or other
coreutils `ln.exe` entries on `PATH`.

### 4. Install And Start The Broker Service

Run from an Administrator PowerShell:

```powershell
& "C:\Program Files\win-symlinks\win-symlinks.exe" service install
& "C:\Program Files\win-symlinks\win-symlinks.exe" service start
& "C:\Program Files\win-symlinks\win-symlinks.exe" service status
```

`service install` registers the `WinSymlinksBroker` Windows service and resolves
`win-symlinks-broker.exe` from the same directory as `win-symlinks.exe`.

### 5. Verify The Installation

Open a new non-admin terminal:

```powershell
where.exe ln
win-symlinks.exe doctor
```

Create a file symlink:

```powershell
"hello" | Set-Content target.txt
ln -s target.txt link.txt
Get-Item link.txt | Format-List FullName,LinkType,Target
```

Create a directory symlink:

```powershell
New-Item -ItemType Directory target-dir
ln -s target-dir link-dir
Get-Item link-dir | Format-List FullName,LinkType,Target
```

When the target does not exist yet, provide the Windows link kind explicitly:

```powershell
ln -s --win-kind=file future-target.txt future-link.txt
ln -s --win-kind=dir future-target-dir future-link-dir
```

## PowerShell Alias Note

PowerShell environments may define `ln` as an alias. If `ln` does not resolve to
this project's executable, use `ln.exe` explicitly or remove the alias in your
PowerShell profile:

```powershell
Remove-Item Alias:ln -Force -ErrorAction SilentlyContinue
```

To inspect command resolution:

```powershell
Get-Command ln -All
where.exe ln
```

## Usage

Supported forms include:

```powershell
ln -s TARGET LINK_NAME
ln -sf TARGET LINK_NAME
ln -sT TARGET LINK_NAME
ln -s --win-kind=file TARGET LINK_NAME
ln -s --win-kind=dir TARGET LINK_NAME
```

Notes:

- `-s` is required. Hardlink mode is intentionally unsupported.
- `-f` may replace an existing symbolic link at `LINK_NAME`.
- `-f` must not replace a real file or real directory.
- `-T` treats `LINK_NAME` as the link path instead of placing the link inside an
  existing destination directory.
- `--win-kind=file|dir` is needed when the target does not exist and Windows
  cannot infer the symbolic link type.

## Management Commands

```powershell
win-symlinks.exe service install
win-symlinks.exe service uninstall
win-symlinks.exe service start
win-symlinks.exe service stop
win-symlinks.exe service status
win-symlinks.exe doctor
win-symlinks.exe config show
```

## Uninstall

Run PowerShell as Administrator:

```powershell
win-symlinks.exe service uninstall
```

Then remove the installation directory from `PATH` and delete the installed
files:

```powershell
$install = "C:\Program Files\win-symlinks"
$machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
$newPath = (($machinePath -split ";") | Where-Object { $_ -and $_ -ne $install }) -join ";"
[Environment]::SetEnvironmentVariable("Path", $newPath, "Machine")
Remove-Item $install -Recurse -Force
```

Open a new terminal after removing the PATH entry.

## Security Model

`WinSymlinksBroker` runs as `LocalSystem`, so it treats every request as
security-sensitive. The broker validates local-only IPC, caller identity, request
schema, caller permission to create in the link parent directory, and source
blacklist policy before creating a link.

The broker creates only real Windows symbolic links.
