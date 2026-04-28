# Development TODO

This TODO breaks implementation into bounded stages to keep agent context small and verification clear. Do not mark a stage complete until its listed verification passes.

## Stage 1 - CLI And Data Model

Goal: stabilize command parsing, shared errors, and IPC payload contracts before Windows-specific implementation begins.

- [ ] Add focused tests for `ln -s TARGET LINK_NAME`, `ln -sf TARGET LINK_NAME`, `ln -sT TARGET LINK_NAME`, and `--win-kind=file|dir`.
- [ ] Reject unsupported hardlink-style `ln TARGET LINK_NAME` with `UNSUPPORTED_MODE`.
- [ ] Keep `ln.exe --help` and `ln.exe --version` service-free.
- [ ] Finalize JSON request/response schema in `src/ipc`.
- [ ] Add round-trip serialization tests for broker request/response payloads.
- [ ] Ensure `ErrorCode` display strings match the documented script-friendly names.

Verification:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

## Stage 2 - Path Policy And Symlink Semantics

Goal: complete pure logic before any privileged broker path exists.

- [ ] Complete path normalization for drive paths, supported `\\?\` forms, suspicious UNC forms, short-name handling where practical, and malformed native paths.
- [ ] Reject ADS-style `link_path` inputs after accounting for drive-letter colons.
- [ ] Reject or canonicalize trailing-space and trailing-dot path spellings.
- [ ] Implement default source blacklist generation from actual system locations where possible.
- [ ] Add user blacklist merge logic without allowing user config to replace built-ins.
- [ ] Implement component-aware blacklist matching, including volume roots and UNC administrative shares.
- [ ] Decide target kind from filesystem state, with `--win-kind` as a hint for missing targets.
- [ ] Model safe replacement rules for `ln -sf`: only existing symbolic links may be replaced.

Verification:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

## Stage 3 - Direct True Symlink Creation

Goal: implement the unprivileged/direct path while preserving the same true symlink semantics as the broker path.

- [ ] Wrap `CreateSymbolicLinkW` in `src/symlink`.
- [ ] Use `SYMBOLIC_LINK_FLAG_DIRECTORY` for directory symlinks.
- [ ] Use `SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE` only for direct client attempts.
- [ ] Distinguish privilege failure from other create failures.
- [ ] Return `TARGET_KIND_REQUIRED`, `LINK_ALREADY_EXISTS`, `TARGET_KIND_CONFLICT`, and `CREATE_SYMLINK_FAILED` as appropriate.
- [ ] Confirm direct creation never falls back to junctions, hardlinks, copies, or `.lnk` files.

Verification:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

Manual Windows verification should confirm created filesystem objects are true symbolic links.

## Stage 4 - Management Command And Service Registration

Goal: make `win-symlinks.exe` manage `WinSymlinksBroker` without putting management behavior in `ln.exe`.

- [ ] Implement `service install` with `CreateServiceW`.
- [ ] Configure service name `WinSymlinksBroker`, display name `Win Symlinks Broker`, `LocalSystem`, and delayed automatic start.
- [ ] Implement `service uninstall`, stopping first when needed.
- [ ] Implement `service start`, `service stop`, and `service status`.
- [ ] Keep administrator-required failures clear.
- [ ] Implement `config show` with effective built-in and user blacklist entries.

Verification:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

Manual administrator-shell verification is required for install/start/status/stop/uninstall.

## Stage 5 - Broker IPC

Goal: connect `ln.exe` to the broker over local Named Pipe IPC with stable JSON payloads and timeouts.

- [ ] Implement Named Pipe server at `\\.\pipe\win-symlinks-broker`.
- [ ] Create the pipe from the broker service only.
- [ ] Use `PIPE_REJECT_REMOTE_CLIENTS`.
- [ ] Apply an explicit DACL granting only the documented local principals and minimum client read/write access.
- [ ] Implement client connection timeout of 3 seconds and request timeout of 30 seconds.
- [ ] Verify connected server process identity before sending privileged requests.
- [ ] Return `SERVICE_IDENTITY_MISMATCH` if the pipe server is not the installed broker service process.
- [ ] Serialize filesystem mutation processing in the broker.

Verification:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

Manual or integration verification must include unavailable service and wrong-server pipe scenarios.

## Stage 6 - Broker Security Validation

Goal: enforce the broker security contract before privileged symlink creation.

- [ ] Validate request schema and protocol version.
- [ ] Impersonate the Named Pipe client and identify a concrete local user SID.
- [ ] Reject remote, anonymous, and network logon clients.
- [ ] Normalize `link_path` and `target_path` before policy checks.
- [ ] Enforce the source blacklist against normalized `link_path`.
- [ ] While impersonating the caller, verify write/create permission on the `link_path` parent directory.
- [ ] Re-check safe replacement conditions immediately before deletion.
- [ ] Return to the service token before calling privileged `CreateSymbolicLinkW`.
- [ ] Write audit log entries for successful privileged operations.

Verification:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

Manual security verification must cover denied parent permissions, blacklisted paths, and replacement refusal for real files/directories/junctions.

## Stage 7 - End-To-End Acceptance

Goal: prove the user workflow works under the intended Windows 11 conditions.

- [ ] From an administrator shell: install and start the service.
- [ ] From a non-admin shell with Developer Mode disabled: create file and directory symlinks through `ln.exe`.
- [ ] Create missing-target links with `--win-kind=file` and `--win-kind=dir`.
- [ ] Replace an existing symlink with `ln -sf`.
- [ ] Confirm `ln -sf` refuses to replace real files and real directories.
- [ ] Confirm created objects are symbolic links, not junctions, hardlinks, file copies, directory copies, or `.lnk` shortcuts.
- [ ] Run `win-symlinks.exe doctor`.
- [ ] Uninstall the service from an administrator shell.

Verification:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

Record any manual-only acceptance gaps in the final handoff for the stage.
