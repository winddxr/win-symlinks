# Handoff: Split Client API Into A Lightweight Crate

## Summary

Split the reusable symlink creation client API out of the main `win-symlinks`
crate into a lightweight Rust crate named `win-symlinks-client`.

The goal is to let external Rust projects depend on a small SDK instead of the
full application crate that contains CLI tools, service management, diagnostics,
broker server code, and internal development tooling.

The main project should continue to provide:

- `ln.exe`
- `win-symlinks.exe`
- `win-symlinks-broker.exe`
- broker service implementation
- diagnostics and service management

The new client crate should provide:

- `CreateSymlinkOptions`
- `create_symlink`
- `create_symlink_via_broker`
- `TargetKind`
- `ErrorCode`
- `WinSymlinksError`
- broker request/response protocol types
- direct true symlink creation
- Named Pipe client transport
- broker service identity verification

It must not provide CLI parsing, service installation, broker server hosting,
doctor diagnostics, or config display behavior.

## Target Workspace Layout

Convert the repository to a Cargo workspace:

```text
win-symlinks/
  Cargo.toml
  crates/
    win-symlinks-client/
      Cargo.toml
      src/
        lib.rs
        error.rs
        protocol.rs
        direct.rs
        pipe.rs
        service_identity.rs
    win-symlinks/
      Cargo.toml
      src/
        bin/
          ln.rs
          win-symlinks.rs
          win-symlinks-broker.rs
        config/
        doctor/
        ipc/
        path_policy/
        service/
        symlink/
```

The root `Cargo.toml` should become a workspace manifest:

```toml
[workspace]
members = [
  "crates/win-symlinks-client",
  "crates/win-symlinks",
]
resolver = "2"
```

Move the existing package metadata and binary definitions into
`crates/win-symlinks/Cargo.toml`.

## Client Crate Public API

The client crate name should be:

```toml
name = "win-symlinks-client"
```

The Rust library path should expose:

```rust
pub use error::{ErrorCode, Result, WinSymlinksError};
pub use protocol::{BrokerResponse, CreateSymlinkRequest, Operation, PROTOCOL_VERSION};

pub struct CreateSymlinkOptions {
    pub target_path: PathBuf,
    pub link_path: PathBuf,
    pub target_kind: Option<TargetKind>,
    pub replace_existing_symlink: bool,
}

pub enum TargetKind {
    File,
    Dir,
}

pub fn create_symlink(options: CreateSymlinkOptions) -> Result<()>;
pub fn create_symlink_via_broker(options: CreateSymlinkOptions) -> Result<()>;
```

`CreateSymlinkOptions::new(target_path, link_path)` must preserve Linux
`ln -s TARGET LINK_NAME` ordering.

Behavior requirements:

- `create_symlink` first tries direct `CreateSymbolicLinkW` with
  `SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE`.
- It falls back to the broker only for privilege-required cases or replacement
  cases that must be handled by broker-side validation.
- `create_symlink_via_broker` skips direct creation and submits directly to the
  broker.
- Relative `link_path` resolves against the caller current directory.
- `target_path` is preserved as supplied so relative symlink targets keep their
  intended spelling.
- The client crate must never create junctions, hardlinks, copies, or `.lnk`
  shortcuts.

## Code Movement

Move or split these responsibilities into `win-symlinks-client`:

- From current `src/client/mod.rs`:
  - public options and builder methods
  - `create_symlink`
  - `create_symlink_via_broker`
  - link-path absolutization
- From current `src/error.rs`:
  - `ErrorCode`
  - `WinSymlinksError`
  - `Result`
- From current `src/ipc/mod.rs`:
  - protocol constants
  - `Operation`
  - `CreateSymlinkRequest`
  - `BrokerResponse`
  - client-side Named Pipe connection and message read/write
  - pipe server identity verification
- From current `src/symlink/mod.rs`:
  - `TargetKind`
  - direct `CreateSymbolicLinkW` wrapper
  - target-kind decision
  - link-path state inspection
  - safe replacement planning needed before direct create

Keep these responsibilities in the main app crate:

- broker Named Pipe server
- broker validation and impersonation
- source blacklist enforcement
- config loading
- audit logging
- service install/start/stop/status
- doctor diagnostics
- CLI parsing for `ln.exe`

The main app crate should depend on the client crate:

```toml
[dependencies]
win-symlinks-client = { path = "../win-symlinks-client" }
```

`ln.rs` should import from `win_symlinks_client` for creation behavior.

The broker server can also use protocol and error types from
`win-symlinks-client` to avoid wire schema duplication.

## Dependency Boundary

`win-symlinks-client` should keep dependencies minimal:

```toml
[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
uuid = { version = "1", features = ["v7", "serde"] }

[target.'cfg(windows)'.dependencies]
windows = { version = "0.62", features = [
  "Win32_Foundation",
  "Win32_Storage_FileSystem",
  "Win32_System_Pipes",
  "Win32_System_Services",
  "Win32_System_Threading",
] }
```

Do not include these in the client crate unless a concrete client-side need is
introduced:

- `clap`
- `tracing-subscriber`
- `windows-service`
- broker impersonation/security server features
- config parsing for `%ProgramData%\win-symlinks\config.json`
- doctor-only dependencies

If service identity verification needs additional Windows feature flags, add
only the minimum required flags and document why.

## Documentation Updates

Update `docs/win-symlinks-integration.md` to recommend:

```toml
[dependencies]
win-symlinks-client = { git = "https://github.com/winddxr/win-symlinks", package = "win-symlinks-client" }
```

Local development example:

```toml
[dependencies]
win-symlinks-client = { path = "../win-symlinks/crates/win-symlinks-client" }
```

Update examples to import:

```rust
use win_symlinks_client::{create_symlink, CreateSymlinkOptions, TargetKind};
```

Update `dev-docs/architecture-navigation.md`:

- show `external Rust project -> win-symlinks-client -> direct create or broker`
- list `crates/win-symlinks-client` as the stable integration boundary
- keep `crates/win-symlinks` as the application/service crate

Update `dev-docs/design-client-interfaces.md`:

- rename the library API section to mention `win-symlinks-client`
- explicitly state that `ln.exe` is a consumer of the client crate

Update both READMEs to link to `docs/win-symlinks-integration.md` and mention the lightweight
client crate.

## Test Plan

Run from the workspace root:

```powershell
cargo fmt -- --check
cargo test
cargo check
```

Client crate tests:

- `CreateSymlinkOptions::new(target, link)` preserves target/link order.
- builder methods set `target_kind` and `replace_existing_symlink`.
- relative `link_path` resolves against caller current directory.
- protocol request/response JSON schema stays byte-compatible with current v1
  schema.
- broker error responses map to `WinSymlinksError`.
- direct symlink code maps privilege failures to `PRIVILEGE_REQUIRED`.

Main app crate tests:

- existing `ln.rs` parsing tests still pass.
- broker server can deserialize client-crate protocol requests.
- service and doctor code still compile against the moved shared types.

Manual Windows verification remains required:

- non-admin caller creates file and directory symlinks through installed broker.
- direct creation still works where Windows permits it.
- `ln -sf` replaces only existing symbolic links.
- created objects are true symbolic links, not junctions, hardlinks, copies, or
  `.lnk` shortcuts.

## Compatibility And Rollout Notes

Keep broker protocol version at `1`. This refactor must not change the JSON wire
schema.

Keep the existing binary names:

- `ln.exe`
- `win-symlinks.exe`
- `win-symlinks-broker.exe`

Keep the existing service name:

```text
WinSymlinksBroker
```

If publishing crates later, publish `win-symlinks-client` as the SDK crate. The
application crate may remain unpublished or be published separately, but external
projects should not need to depend on it for client API usage.
