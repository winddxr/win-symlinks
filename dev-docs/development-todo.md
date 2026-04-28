# Development TODO

This TODO breaks implementation into bounded stages to keep agent context small and verification clear. Do not mark a stage complete until its listed verification passes.

## Stage 1 - CLI And Data Model

Goal: stabilize command parsing, shared errors, and IPC payload contracts before Windows-specific implementation begins.

- [x] Add focused tests for `ln -s TARGET LINK_NAME`, `ln -sf TARGET LINK_NAME`, `ln -sT TARGET LINK_NAME`, and `--win-kind=file|dir`.
- [x] Reject unsupported hardlink-style `ln TARGET LINK_NAME` with `UNSUPPORTED_MODE`.
- [x] Keep `ln.exe --help` and `ln.exe --version` service-free.
- [x] Finalize JSON request/response schema in `src/ipc`.
- [x] Add round-trip serialization tests for broker request/response payloads.
- [x] Ensure `ErrorCode` display strings match the documented script-friendly names.

Verification:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

## Stage 2 - Path Policy And Symlink Semantics

Goal: complete pure logic before any privileged broker path exists.

- [x] Complete path normalization for drive paths, supported `\\?\` forms, suspicious UNC forms, short-name handling where practical, and malformed native paths.
- [x] Reject ADS-style `link_path` inputs after accounting for drive-letter colons.
- [x] Reject or canonicalize trailing-space and trailing-dot path spellings.
- [x] Implement default source blacklist generation from actual system locations where possible.
- [x] Add user blacklist merge logic without allowing user config to replace built-ins.
- [x] Implement component-aware blacklist matching, including volume roots and UNC administrative shares.
- [x] Decide target kind from filesystem state, with `--win-kind` as a hint for missing targets.
- [x] Model safe replacement rules for `ln -sf`: only existing symbolic links may be replaced.

Verification:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

## Stage 3 - Direct True Symlink Creation

Goal: implement the unprivileged/direct path while preserving the same true symlink semantics as the broker path.

- [x] Wrap `CreateSymbolicLinkW` in `src/symlink`.
- [x] Use `SYMBOLIC_LINK_FLAG_DIRECTORY` for directory symlinks.
- [x] Use `SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE` only for direct client attempts.
- [x] Distinguish privilege failure from other create failures.
- [x] Return `TARGET_KIND_REQUIRED`, `LINK_ALREADY_EXISTS`, `TARGET_KIND_CONFLICT`, and `CREATE_SYMLINK_FAILED` as appropriate.
- [x] Confirm direct creation never falls back to junctions, hardlinks, copies, or `.lnk` files.

Verification:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

Manual Windows verification should confirm created filesystem objects are true symbolic links.

## Stage 4 - Management Command And Service Registration

Goal: make `win-symlinks.exe` manage `WinSymlinksBroker` without putting management behavior in `ln.exe`.

- [x] Implement `service install` with `CreateServiceW`.
- [x] Configure service name `WinSymlinksBroker`, display name `Win Symlinks Broker`, `LocalSystem`, and delayed automatic start.
- [x] Implement `service uninstall`, stopping first when needed.
- [x] Implement `service start`, `service stop`, and `service status`.
- [x] Keep administrator-required failures clear.
- [x] Implement `config show` with effective built-in and user blacklist entries.

Verification:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

Manual administrator-shell verification is required for install/start/status/stop/uninstall.

## Stage 5 - Broker IPC

Goal: connect `ln.exe` to the broker over local Named Pipe IPC with stable JSON payloads and timeouts.

- [x] Implement Named Pipe server at `\\.\pipe\win-symlinks-broker`.
- [x] Create the pipe from the broker service only.
- [x] Use `PIPE_REJECT_REMOTE_CLIENTS`.
- [x] Apply an explicit DACL granting only the documented local principals and minimum client read/write access.
- [x] Implement client connection timeout of 3 seconds and request timeout of 30 seconds.
- [x] Verify connected server process identity before sending privileged requests.
- [x] Return `SERVICE_IDENTITY_MISMATCH` if the pipe server is not the installed broker service process.
- [x] Serialize filesystem mutation processing in the broker.

Verification:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

Manual or integration verification must include unavailable service and wrong-server pipe scenarios.

## Stage 6 - Broker Security Validation

Goal: enforce the broker security contract before privileged symlink creation.

- [x] Validate request schema and protocol version.
- [x] Impersonate the Named Pipe client and identify a concrete local user SID.
- [x] Reject remote, anonymous, and network logon clients.
- [x] Normalize `link_path` and `target_path` before policy checks.
- [x] Enforce the source blacklist against normalized `link_path`.
- [x] While impersonating the caller, verify write/create permission on the `link_path` parent directory.
- [x] Re-check safe replacement conditions immediately before deletion.
- [x] Return to the service token before calling privileged `CreateSymbolicLinkW`.
- [x] Write audit log entries for successful privileged operations.

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
