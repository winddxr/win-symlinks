# Client Interfaces & Management (win-symlinks)

This document details the public user interfaces for `win-symlinks`, focusing on the CLI behavior, management tools, service installation without MSI, diagnostics, and error handling.

## Public Interfaces

### Project Name

The project name is:

```text
win-symlinks
```

### `win-symlinks-client` Rust API

The primary integration surface for Rust projects is the lightweight
`win-symlinks-client` crate:

```rust
win_symlinks_client::create_symlink(options)
win_symlinks_client::create_symlink_via_broker(options)
```

The public options preserve Linux `ln -s TARGET LINK_NAME` ordering:

```rust
pub struct CreateSymlinkOptions {
    pub target_path: PathBuf,
    pub link_path: PathBuf,
    pub target_kind: Option<TargetKind>,
    pub replace_existing_symlink: bool,
}
```

Behavior:

- `create_symlink` attempts direct true symbolic link creation first, then uses
  `WinSymlinksBroker` when broker privileges are needed.
- `create_symlink_via_broker` submits the request directly to the broker.
- Relative `link_path` values are resolved against the caller current directory.
- `target_path` is preserved as the value stored in the symbolic link.
- The API must never fall back to junctions, hardlinks, copies, or `.lnk`
  shortcuts.

`ln.exe` is a user-facing command-line frontend and should be treated as a
client API consumer. External projects and AI agents should use the library API
or documented IPC schema rather than copying and modifying
`crates/win-symlinks/src/bin/ln.rs`.

### Linux-Compatible Command

The primary user command is:

```text
ln.exe
```

Minimum supported forms:

```text
ln -s TARGET LINK_NAME
ln -sf TARGET LINK_NAME
ln -sT TARGET LINK_NAME
ln -s --win-kind=file TARGET LINK_NAME
ln -s --win-kind=dir TARGET LINK_NAME
ln --help
ln --version
```

Behavior:

- `ln -s TARGET LINK_NAME` creates a symbolic link at `LINK_NAME` pointing to `TARGET`.
- `-s` is required for v1. Hardlink mode is intentionally unsupported because this project is true-symlink-only.
- `-f` may replace an existing symbolic link at `LINK_NAME`.
- `-f` must not delete a real file or real directory unless the behavior is explicitly designed and documented later.
- `-T` treats `LINK_NAME` as a normal link path rather than treating an existing destination directory as a container.
- `--win-kind=file|dir` is a Windows-only extension used when `TARGET` does not exist and Windows requires the link type at creation time.
- `--help` and `--version` do not contact the service.
- Actual symlink creation is delegated to `win_symlinks_client`.

If a requested Linux `ln` mode would require hardlinks or another unsupported behavior, `ln.exe` must fail explicitly instead of emulating it with a different Windows object type.

### Management Command

The management executable is:

```text
win-symlinks.exe
```

Required commands:

```text
win-symlinks.exe service install
win-symlinks.exe service uninstall
win-symlinks.exe service start
win-symlinks.exe service stop
win-symlinks.exe service status
win-symlinks.exe doctor
win-symlinks.exe config show
```

Behavior:

- `service install` registers `WinSymlinksBroker` with the Windows Service Control Manager.
- `service uninstall` stops and removes the service registration.
- `service start` starts the installed broker service.
- `service stop` stops the installed broker service.
- `service status` reports whether the service is installed and running.
- `doctor` checks platform support, service status, PATH conflicts, current `ln.exe` resolution, and blacklist configuration.
- `config show` prints the effective configuration, including source blacklist entries.

## Service Installation Without An Installer Package

Windows Service installation does not require an MSI or graphical installer. It does require administrator rights because registering a service modifies the Service Control Manager database.

Supported no-installer flows:

### Built-In Install Command

Preferred:

```text
win-symlinks.exe service install
```

Implementation:

- Resolve the path to the broker executable.
- Call `OpenSCManagerW`.
- Call `CreateServiceW` with service name `WinSymlinksBroker`.
- Configure the service to run as `LocalSystem`.
- Configure default start type as `Automatic (Delayed Start)`.
- Set a clear display name, such as `Win Symlinks Broker`.
- Optionally start the service after registration.

### Built-In Uninstall Command

Preferred:

```text
win-symlinks.exe service uninstall
```

Implementation:

- Open the Service Control Manager.
- Open `WinSymlinksBroker`.
- Stop the service if running.
- Call `DeleteService`.

### Built-In Start And Stop Commands

Preferred:

```text
win-symlinks.exe service start
win-symlinks.exe service stop
```

Implementation:

- `service start` calls `StartServiceW` for `WinSymlinksBroker`.
- `service stop` sends a service stop control and waits for the stopped state.
- Both commands require administrator rights unless the service ACL is explicitly relaxed later.

### Manual sc.exe Registration

Document as an alternative:

```text
sc.exe create WinSymlinksBroker binPath= "C:\Path\To\win-symlinks-broker.exe" start= delayed-auto obj= LocalSystem DisplayName= "Win Symlinks Broker"
sc.exe start WinSymlinksBroker
```

### Manual PowerShell Registration

Document as an alternative:

```powershell
New-Service -Name "WinSymlinksBroker" -BinaryPathName "C:\Path\To\win-symlinks-broker.exe" -DisplayName "Win Symlinks Broker" -StartupType Automatic
Start-Service WinSymlinksBroker
```

When using PowerShell, configure delayed automatic start separately if required by the target Windows version and PowerShell module support.

These alternatives are for operators and debugging. The product path should be the built-in `win-symlinks.exe service install` command.

## Diagnostics

`win-symlinks.exe doctor` should check:

- Windows version.
- Filesystem type for current test directory.
- Whether Developer Mode appears enabled.
- Whether direct symlink creation works.
- Whether `WinSymlinksBroker` is installed.
- Whether `WinSymlinksBroker` is running.
- Whether the Named Pipe is reachable.
- Whether the Named Pipe rejects remote clients.
- Whether the Named Pipe DACL matches the expected local-only policy.
- Whether the connected pipe server process matches the installed `WinSymlinksBroker` service.
- Which `ln.exe` is resolved first on PATH.
- Whether that `ln.exe` belongs to `win-symlinks`.
- Potential conflicts from Git for Windows, MSYS2, Cygwin, BusyBox, or coreutils.
- Effective source blacklist.
- Whether default blacklist entries map to actual system directories.

If PATH resolves `ln.exe` to another tool, print the full path and remediation instructions.

## Error Handling

Errors should be stable and script-friendly.

Recommended categories:

```text
UNSUPPORTED_MODE
SERVICE_NOT_INSTALLED
SERVICE_UNAVAILABLE
PRIVILEGE_REQUIRED
SOURCE_BLACKLISTED
TARGET_KIND_REQUIRED
LINK_ALREADY_EXISTS
LINK_PATH_IS_NOT_SYMLINK
UNSAFE_REPARSE_POINT
CREATE_SYMLINK_FAILED
PATH_NORMALIZATION_FAILED
SERVICE_IDENTITY_MISMATCH
CALLER_PARENT_WRITE_DENIED
TARGET_KIND_CONFLICT
REMOTE_CLIENT_REJECTED
REPLACEMENT_PARTIALLY_COMPLETED
```

`ln.exe` should print concise stderr messages and exit non-zero. `win-symlinks.exe doctor` may print richer diagnostics.
