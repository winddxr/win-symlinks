# Broker Service Design (win-symlinks)

This document covers the design of the `WinSymlinksBroker` service, including IPC mechanisms, validation flows, and concurrency.

## Windows Service

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

## IPC

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

## Broker Validation Flow

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

## Concurrency Model

V1 should use a single broker worker that processes one filesystem mutation at a time.

Rationale:

- Symlink creation is fast.
- Serial execution avoids races between concurrent requests for the same `link_path`.
- The simpler model is easier to audit for a privileged service.

The Named Pipe server may accept multiple client connections, but mutation requests must be serialized before validation and filesystem changes. If a second request targets a `link_path` currently being processed, it waits behind the broker queue and then observes the final filesystem state.
