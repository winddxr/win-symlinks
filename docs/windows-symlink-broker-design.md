# win-symlinks Windows 11 True Symlink Broker Design

## Summary

`win-symlinks` aims to provide a Linux-like symbolic link experience on Windows 11 while keeping the underlying implementation strictly based on real Windows symbolic links.

The project does not use junctions, hardlinks, file copies, or `.lnk` shortcuts as semantic fallbacks. If a real symbolic link cannot be created, the command must fail with a clear diagnostic.

The user-facing command is `ln.exe`, aligned as closely as practical with Linux `ln`. Administrative and diagnostic operations are exposed through `win-symlinks.exe`. The privileged link creation work is performed by a separate Windows Service named `WinSymlinksBroker`.

```text
ln.exe
  -> parses Linux-compatible ln arguments
  -> calls WinSymlinksBroker through Named Pipe IPC when elevation is needed
  -> reports Linux-like success and error behavior

win-symlinks.exe
  -> manages service installation, service status, diagnostics, and config

WinSymlinksBroker
  -> runs as an elevated Windows Service
  -> validates requests
  -> calls CreateSymbolicLinkW
```

## Goals And Non-Goals

Goals:

- Provide `ln -s TARGET LINK_NAME` behavior on Windows 11 using real symbolic links.
- Keep `ln.exe` focused on Linux-compatible link behavior.
- Use `win-symlinks.exe` for management commands.
- Implement the broker and CLI tools in Rust.
- Support service registration without an MSI or graphical installer.
- Use a source path blacklist as the primary path safety policy, as required by the project direction.

Non-goals:

- Do not depend on Windows Developer Mode as the main mechanism.
- Do not require Group Policy changes as the main mechanism.
- Do not silently fall back to junctions, hardlinks, copies, or `.lnk` files.
- Do not put service management commands inside `ln.exe`.
- Do not require the service name to match either executable name.

## Threat Model And Limitations

`WinSymlinksBroker` is a privileged local service. Its security contract is narrower than "create any symlink requested by any caller":

- Only local clients may connect to the broker IPC endpoint.
- The broker must identify the calling Windows user for every request.
- The broker must verify that the calling user already has filesystem permission to create an entry in the `link_path` parent directory.
- The broker must enforce the source blacklist after path normalization.
- The broker must create only true symbolic links.

The source blacklist is an explicit project choice, but it is weaker than an allowlist. The design accepts this tradeoff only with additional defenses: strict path normalization, caller write-permission checks, local-only IPC, audit logs, and `doctor` warnings.

Known limitations:

- The blacklist primarily protects where a privileged service may create the link. It does not restrict `target_path` by default.
- A symlink targeting a sensitive directory may still be dangerous when consumed by a vulnerable high-privilege program. This is outside the v1 authorization boundary and must be documented to users.
- Windows path parsing has edge cases such as native `\\?\` paths, UNC paths, short 8.3 names, reparse points, trailing spaces, trailing dots, and NTFS alternate data stream syntax.
- The implementation must reject ADS-style `link_path` inputs such as `file.txt:stream` and must canonicalize or reject long-path forms before blacklist matching.
- A time-of-check-to-time-of-use window exists between validation and creation. V1 reduces this with parent-directory permission checks and symlink-only replacement rules, but it does not claim full transactional filesystem semantics.
- `ln -sf` replacement is not guaranteed to be atomic in v1.

## Public Interfaces

### Project Name

The project name is:

```text
win-symlinks
```

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

### Windows Service

The Windows Service name is:

```text
WinSymlinksBroker
```

The service name is independent from both `ln.exe` and `win-symlinks.exe`. These names do not need to match:

```text
ln.exe                  user-facing Linux-compatible command
win-symlinks.exe        management command
WinSymlinksBroker       Windows Service name
```

V1 service identity:

- Run the service as `LocalSystem`.
- Use `LocalSystem` because the broker must reliably hold the privilege needed to call `CreateSymbolicLinkW` when Developer Mode and user privileges are unavailable.
- Do not use `LocalService` or `NetworkService` for v1 because their privilege sets are less suitable for guaranteed symbolic link creation.
- Treat `LocalSystem` as high risk: every request must be authenticated, locally constrained, authorized against the caller's own filesystem permissions, and audited.

V1 startup policy:

- Default to `Automatic (Delayed Start)` after `win-symlinks.exe service install`.
- Allow a future config option for `Manual`, but the default should optimize for Linux-like availability after boot.
- `ln.exe` may attempt to start the service if it is installed but stopped. If auto-start fails, it must return a clear `SERVICE_UNAVAILABLE` error.

## Architecture

### Process Model

The normal execution path is:

```text
User shell
  -> ln.exe
  -> Named Pipe request
  -> WinSymlinksBroker
  -> CreateSymbolicLinkW
  -> response to ln.exe
```

`ln.exe` may first attempt direct symbolic link creation if the current process has the required privilege or Windows allows unprivileged creation. If direct creation fails because of privilege, it must call the broker. Direct creation and broker creation must both use the same true symbolic link semantics.

The broker is the stable path for systems where Developer Mode is disabled and the user account does not have `SeCreateSymbolicLinkPrivilege`.

### IPC

Use a Windows Named Pipe for local IPC. The pipe is a privileged boundary and must be created with an explicit security descriptor.

Recommended pipe name:

```text
\\.\pipe\win-symlinks-broker
```

Required pipe security:

- Create the pipe from `WinSymlinksBroker`, not from `ln.exe`.
- Use `PIPE_REJECT_REMOTE_CLIENTS` so SMB remote named-pipe clients are rejected by the kernel.
- Use an explicit DACL instead of inheriting the service process default security descriptor.
- Grant `LocalSystem` and `Administrators` full control.
- Grant local interactive authenticated users only the minimum read/write permissions needed to submit requests.
- Do not grant access to anonymous users, `Everyone`, or network logon users.
- Reject a request if the impersonated client token represents a remote or network logon.

Client-side pipe safety:

- `ln.exe` should verify that the connected pipe server belongs to the installed `WinSymlinksBroker` service before sending a privileged request.
- Use `GetNamedPipeServerProcessId` where available, then verify that the process ID matches the running service process.
- If the pipe exists but is not owned by the service, fail with `SERVICE_IDENTITY_MISMATCH`.
- This mitigates a malicious process pre-creating `\\.\pipe\win-symlinks-broker` before the service starts.

Request payloads should be JSON for debuggability and versioning:

```json
{
  "version": 1,
  "request_id": "018f5b2a-7f3a-7b7a-9c21-000000000001",
  "operation": "create_symlink",
  "link_path": "F:\\work\\project\\node_modules\\pkg",
  "target_path": "..\\shared\\pkg",
  "target_kind": "directory",
  "replace_existing_symlink": false
}
```

Response payload:

```json
{
  "request_id": "018f5b2a-7f3a-7b7a-9c21-000000000001",
  "ok": true,
  "error_code": null,
  "message": null
}
```

On failure:

```json
{
  "request_id": "018f5b2a-7f3a-7b7a-9c21-000000000001",
  "ok": false,
  "error_code": "SOURCE_BLACKLISTED",
  "message": "link path is blocked by source blacklist: C:\\Windows"
}
```

The implementation may later move to a binary protocol, but v1 should prefer JSON unless performance proves inadequate.

Client timeouts:

- Pipe connection timeout: 3 seconds.
- Request timeout: 30 seconds.
- No automatic retry for `create_symlink` after a request is accepted by the broker.
- Retrying is allowed only if no request was sent or the broker returned a retryable transport error before processing began.

### Broker Validation Flow

For `create_symlink`:

1. Parse and validate the request schema.
2. Identify the calling user from the Named Pipe client with `ImpersonateNamedPipeClient` and token inspection.
3. Reject the request if the client is remote, anonymous, or cannot be mapped to a concrete Windows user SID.
4. Normalize `link_path` and `target_path`.
5. Apply the source blacklist to the normalized `link_path`.
6. While impersonating the caller, verify that the caller has permission to create an entry in the parent directory of `link_path`.
7. Check that `link_path` does not already exist, unless `replace_existing_symlink` is true.
8. If replacing, verify the existing object is a symbolic link and delete only the link itself.
9. Determine the target kind as file or directory on the broker side. Treat the CLI-provided value as a hint, not as authority.
10. Return to the service token before calling `CreateSymbolicLinkW` if impersonation would remove the privilege required to create symbolic links.
11. Call `CreateSymbolicLinkW`.
12. Write an audit log and return a structured response.

Caller write authorization is mandatory. Blacklist checks alone are not sufficient, because a privileged broker must not let a normal user create links in directories where that user could not normally create filesystem entries.

Acceptable authorization implementation:

- While impersonating the client, attempt to open the `link_path` parent directory with the permissions required to create a child entry.
- Alternatively, use `AccessCheck` against the parent directory security descriptor for equivalent rights.
- If the check fails, return `CALLER_PARENT_WRITE_DENIED`.

### Concurrency Model

V1 should use a single broker worker that processes one filesystem mutation at a time.

Rationale:

- Symlink creation is fast.
- Serial execution avoids races between concurrent requests for the same `link_path`.
- The simpler model is easier to audit for a privileged service.

The Named Pipe server may accept multiple client connections, but mutation requests must be serialized before validation and filesystem changes. If a second request targets a `link_path` currently being processed, it waits behind the broker queue and then observes the final filesystem state.

## True Symlink Semantics

The only supported filesystem object created by this project is a Windows symbolic link.

Use Win32 API:

```text
CreateSymbolicLinkW
```

For file symlinks:

```text
CreateSymbolicLinkW(link, target, 0)
```

For directory symlinks:

```text
CreateSymbolicLinkW(link, target, SYMBOLIC_LINK_FLAG_DIRECTORY)
```

The implementation may include `SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE` only when attempting direct creation from `ln.exe`. The service path must not depend on Developer Mode.

Unsupported fallback objects:

- Junctions
- Hardlinks
- Directory copies
- File copies
- `.lnk` shortcuts

If true symlink creation fails, return the Windows error and a short explanation.

## Path And Target Rules

### Source And Target Terminology

In this design:

- `link_path` is the path where the symbolic link will be created. This corresponds to `LINK_NAME` in `ln -s TARGET LINK_NAME`.
- `target_path` is the path stored inside the symbolic link. This corresponds to `TARGET`.
- The source blacklist applies to `link_path`, because that is the filesystem location the privileged service creates or replaces.

### Relative Targets

Prefer preserving the user's target spelling when it is a valid relative target:

```text
ln -s ..\shared\pkg node_modules\pkg
```

The resulting symlink should store:

```text
..\shared\pkg
```

This keeps workspace movement behavior close to Linux.

When a relative target cannot be represented safely, fail with a clear error instead of rewriting the request silently.

### Missing Targets

Windows requires the symlink type at creation time. If `target_path` does not exist, the caller must supply or infer whether the link is for a file or directory.

Rules:

- Existing target directory: create directory symlink.
- Existing target file: create file symlink.
- Missing target with `--win-kind=dir`: create directory symlink.
- Missing target with `--win-kind=file`: create file symlink.
- Missing target without a known kind: fail with an explicit error.

`ln.exe` sends `target_kind` as a hint. The broker performs the final target-kind decision immediately before creation, because the target may change between CLI parsing and broker execution. If the actual filesystem state conflicts with the hint, the broker returns `TARGET_KIND_CONFLICT`.

## Source Blacklist Security Policy

The project intentionally uses a source blacklist for `link_path` safety.

This is less robust than an allowlist because Windows paths have many equivalent spellings and redirection mechanisms. The implementation must therefore normalize paths before applying blacklist rules and must include `doctor` warnings explaining the residual risk.

### Required Blacklist Categories

The default blacklist must block creating links under:

```text
C:\Windows
C:\Program Files
C:\Program Files (x86)
C:\ProgramData
C:\System Volume Information
C:\$Recycle.Bin
```

It must also block:

- Volume roots, such as `C:\`, `D:\`, and mounted volume roots.
- Other users' profile directories under `C:\Users`.
- UNC administrative shares, such as `\\server\C$`.
- Known service-owned data roots when detected.
- Any path resolved through a reparse point into a blacklisted location.

The blacklist must be generated from actual system directories where possible instead of hardcoding only `C:\`. For example, use Windows APIs or environment-derived known folders for the Windows directory, Program Files directories, ProgramData, and user profile roots.

### User Configuration

Users may extend the blacklist through configuration.

Recommended config location:

```text
%ProgramData%\win-symlinks\config.json
```

Recommended shape:

```json
{
  "additional_source_blacklist": [
    "D:\\SensitiveServiceData"
  ],
  "allow_direct_create_attempt": true
}
```

User entries are appended to the built-in default blacklist. They must not replace the default blacklist, because replacement could accidentally remove system protections.

`win-symlinks.exe config show` must display the merged effective configuration and clearly separate built-in entries from user-added entries.

### Blacklist Matching Requirements

Before matching:

- Convert to an absolute path.
- Normalize path separators.
- Remove redundant `.` and `..` segments.
- Resolve short 8.3 names where possible.
- Resolve case-insensitive comparisons.
- Detect and resolve intermediate reparse points.
- Canonicalize supported `\\?\` long-path forms or reject them before matching.
- Reject malformed native paths and suspicious UNC forms.
- Reject `link_path` values containing NTFS alternate data stream syntax, such as `file.txt:stream`, after accounting for the drive-letter colon in paths like `C:\path`.
- Reject trailing-space and trailing-dot spellings unless they can be canonicalized to the same path that will be used for creation.

Matching must be prefix-aware and path-component-aware. For example, blacklisting `C:\Windows` must block:

```text
C:\Windows
C:\Windows\System32
```

but must not accidentally block:

```text
C:\WindowsTools
```

## Safe Replacement And Deletion

`ln -sf` may replace an existing symbolic link only if the existing object at `LINK_NAME` is itself a symbolic link.

Required behavior:

- If `LINK_NAME` does not exist, create normally.
- If `LINK_NAME` is an existing symbolic link and `-f` is set, delete the link itself and create the new link.
- If `LINK_NAME` is a real file or real directory, fail.
- If `LINK_NAME` is a junction, mount point, or unknown reparse point, fail.

Deletion must never follow the symlink target.

Replacement atomicity:

- V1 `ln -sf` replacement is not guaranteed to be atomic.
- The broker should verify the existing object immediately before deletion.
- If deletion succeeds and creation fails, the old symlink may be gone. The error response must state that replacement partially completed.
- A future implementation may add a more atomic strategy if Windows filesystem behavior can support it reliably for both file and directory symlinks.

The management tool may later expose:

```text
win-symlinks.exe unlink LINK_NAME
```

but v1 deletion behavior needed by `ln -sf` must already be implemented safely inside the broker.

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

## Rust Implementation Plan

### Crates

Use these Rust dependencies:

```toml
[dependencies]
windows-service = "0.7"
windows = { version = "0.58", features = [
  "Win32_Foundation",
  "Win32_Storage_FileSystem",
  "Win32_System_Services",
  "Win32_System_Pipes",
  "Win32_System_Threading",
  "Win32_Security",
  "Win32_System_IO",
] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
clap = { version = "4", features = ["derive"] }
tracing = "0.1"
tracing-subscriber = "0.3"
uuid = { version = "1", features = ["v7", "serde"] }
```

Actual versions should be checked when implementation begins.

`tokio` is not required for v1. The v1 broker should prefer a simple serial mutation model. Async Named Pipe I/O can be revisited if throughput becomes a real constraint.

### Binary Layout

Recommended Cargo package:

```text
win-symlinks
```

Recommended binaries:

```text
src/bin/ln.rs
src/bin/win-symlinks.rs
src/bin/win-symlinks-broker.rs
```

Shared library modules:

```text
src/ipc
src/path_policy
src/symlink
src/service
src/config
```

### Responsibilities

`ln.exe`:

- Parse Linux-compatible arguments.
- Decide link path, target path, force mode, and target kind.
- Attempt direct true symlink creation if enabled.
- Call broker on privilege failure.
- Print Linux-like errors.

`win-symlinks.exe`:

- Install, uninstall, and query service status.
- Run diagnostics.
- Show effective configuration.
- Avoid implementing link creation user flows unless needed for testing.

`win-symlinks-broker.exe`:

- Run as `WinSymlinksBroker`.
- Run under the `LocalSystem` account.
- Host a local-only Named Pipe server with explicit DACL.
- Validate caller identity, server identity, and request schema.
- Reject remote pipe clients.
- Verify the caller has write/create permission on the `link_path` parent directory.
- Enforce source blacklist.
- Create true symbolic links.
- Perform safe symlink replacement.
- Log operations.

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

## Manifest And Logs

The broker should write an audit log for privileged operations.

Recommended log directory:

```text
%ProgramData%\win-symlinks\logs
```

Recommended manifest path:

```text
%ProgramData%\win-symlinks\manifest.jsonl
```

Each successful create operation should record:

```json
{
  "timestamp": "2026-04-27T00:00:00Z",
  "operation": "create_symlink",
  "caller_sid": "S-1-5-21-...",
  "link_path": "F:\\work\\project\\link",
  "target_path": "..\\target",
  "target_kind": "directory"
}
```

The manifest is for diagnostics and repair workflows. It must not be the source of truth for whether a filesystem object currently exists.

## Test Plan

### Unit Tests

- Parse `ln -s TARGET LINK_NAME`.
- Parse `ln -sf TARGET LINK_NAME`.
- Parse `ln -sT TARGET LINK_NAME`.
- Reject unsupported hardlink-style `ln TARGET LINK_NAME`.
- Normalize paths before blacklist matching.
- Match blacklisted prefixes by path component.
- Do not match sibling prefixes such as `C:\WindowsTools` for `C:\Windows`.
- Reject malformed native paths and suspicious UNC administrative shares.
- Reject ADS-style `link_path` inputs.
- Canonicalize or reject `\\?\` long-path inputs before blacklist matching.
- Require target kind when target is missing and cannot be inferred.
- Detect `TARGET_KIND_CONFLICT` when the broker's final target-kind decision conflicts with the CLI hint.

### Integration Tests

- Create a true file symlink through the broker.
- Create a true directory symlink through the broker.
- Confirm created objects are symbolic links, not junctions or hardlinks.
- Confirm `ln -sf` replaces an existing symbolic link.
- Confirm `ln -sf` refuses to replace a real file.
- Confirm `ln -sf` refuses to replace a real directory.
- Confirm deletion removes only the symbolic link and leaves the target intact.
- Confirm source blacklist blocks protected paths.
- Confirm source blacklist blocks ADS and long-path bypass attempts.
- Confirm the broker rejects remote Named Pipe clients.
- Confirm the Named Pipe DACL does not allow anonymous, `Everyone`, or network logon access.
- Confirm the client rejects a pipe server that does not match the installed `WinSymlinksBroker` service.
- Confirm a caller cannot create a symlink in a parent directory where that caller lacks create/write permission.
- Confirm concurrent requests for the same `link_path` are serialized and the second request observes the final state.
- Confirm service unavailable errors are clear when broker is not installed.
- Confirm `doctor` reports the resolved `ln.exe` path.

### Manual Acceptance Scenarios

First run from an administrator shell on Windows 11:

```text
win-symlinks.exe service install
win-symlinks.exe service status
```

Then run from a non-admin shell with Developer Mode disabled:

```text
ln -s target.txt link.txt
ln -s target-dir link-dir
ln -s --win-kind=file future-target.txt future-link.txt
ln -sf target2.txt link.txt
win-symlinks.exe doctor
```

Finally run from an administrator shell:

```text
win-symlinks.exe service uninstall
```

Expected result:

- Service installation requires administrator approval.
- Once installed, non-admin `ln -s` can create real symbolic links.
- The implementation never creates junctions, hardlinks, copies, or `.lnk` files.

## Acceptance Criteria

- The design document exists at `docs/windows-symlink-broker-design.md`.
- The project is consistently named `win-symlinks`.
- `ln.exe` is the Linux-compatible command name.
- `win-symlinks.exe` is the management command name.
- `WinSymlinksBroker` is the Windows Service name.
- The document clearly states that command names and service names do not need to match.
- The document clearly states that service installation can be done without an installer package but still requires administrator rights.
- The document clearly states that source uses a blacklist policy.
- The document clearly states that only real symbolic links are supported.
- The document defines the service account as `LocalSystem`.
- The document requires local-only Named Pipe IPC and explicit pipe DACL.
- The document requires caller identity validation and caller parent-directory write authorization.
- The document documents blacklist limitations, ADS handling, long-path handling, and TOCTOU limits.
- The document includes Rust crate recommendations.
- The document includes test cases for true symlink creation and refusal of fallback object types.
