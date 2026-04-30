# win-symlinks Integration Guide

This guide is for projects, tools, and AI coding agents that want to create real
Windows symbolic links by reusing `win-symlinks`.

The recommended integration surface is the Rust client API. Non-Rust projects
may use the documented broker Named Pipe protocol, but should preserve the same
security and true-symlink semantics.

## Recommended Rust API

Add `win-symlinks` as a dependency from the local workspace or repository, then
call `win_symlinks::client::create_symlink`.

```rust
use win_symlinks::client::{create_symlink, CreateSymlinkOptions};
use win_symlinks::TargetKind;

fn main() -> win_symlinks::Result<()> {
    create_symlink(
        CreateSymlinkOptions::new("..\\shared\\pkg", "node_modules\\pkg")
            .target_kind(TargetKind::Dir),
    )
}
```

`CreateSymlinkOptions::new(target_path, link_path)` follows Linux
`ln -s TARGET LINK_NAME` order:

- `target_path` is the path stored in the symbolic link.
- `link_path` is where the symbolic link is created.
- `target_kind` is optional when the target exists, but required for missing
  targets because Windows needs to know whether to create a file or directory
  symlink.
- `replace_existing_symlink` allows replacing an existing symbolic link only.

`create_symlink` first tries direct true symbolic link creation. If direct
creation needs broker privileges, it sends the same request to
`WinSymlinksBroker`.

## Broker-Only Rust API

Use `create_symlink_via_broker` when the caller intentionally wants to skip the
direct attempt and always use the installed service.

```rust
use win_symlinks::client::{create_symlink_via_broker, CreateSymlinkOptions};
use win_symlinks::TargetKind;

fn main() -> win_symlinks::Result<()> {
    create_symlink_via_broker(
        CreateSymlinkOptions::new("future-target.txt", "future-link.txt")
            .target_kind(TargetKind::File),
    )
}
```

This requires the `WinSymlinksBroker` service to be installed and reachable.

## Raw Named Pipe Protocol

Non-Rust clients may connect to the local Named Pipe:

```text
\\.\pipe\win-symlinks-broker
```

Protocol constants:

- Protocol version: `1`
- Pipe connection timeout: 3 seconds
- Request timeout: 30 seconds

Request payloads are JSON:

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

Successful response:

```json
{
  "request_id": "018f5b2a-7f3a-7b7a-9c21-000000000001",
  "ok": true,
  "error_code": null,
  "message": null
}
```

Failure response:

```json
{
  "request_id": "018f5b2a-7f3a-7b7a-9c21-000000000001",
  "ok": false,
  "error_code": "SOURCE_BLACKLISTED",
  "message": "link path is blocked by source blacklist: C:\\Windows"
}
```

`target_kind` values are:

- `"file"`
- `"directory"`
- `null` when the target exists and can be inspected

Clients should verify that the connected pipe server is the installed
`WinSymlinksBroker` service process before sending privileged requests. The Rust
client API already performs this verification.

## Error Codes

Errors use stable script-friendly codes:

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

## Security And Semantics

`win-symlinks` creates only real Windows symbolic links through
`CreateSymbolicLinkW`.

It must not fall back to:

- junctions
- hardlinks
- file or directory copies
- `.lnk` shortcuts

The broker validates local-only IPC, caller identity, request schema, caller
permission to create in the link parent directory, and source blacklist policy
before creating a link.

## Guidance For AI Development

AI agents should integrate through `win_symlinks::client` or the raw protocol
documented here.

Do not copy and modify `src/bin/ln.rs` as the primary integration pattern.
`ln.exe` is a command-line frontend with Linux-compatible argument handling; it
is intentionally separate from the stable client API used by other projects.
