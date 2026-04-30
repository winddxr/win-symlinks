# Handoff: Testing And Code Review For Client Crate Split

## Summary

The next implementation splits the reusable Rust client API into
`win-symlinks-client` and turns the repository into a Cargo workspace.

This handoff covers verification and code review expectations for that work.

The current local machine used for planning does not appear to have a Rust
toolchain installed or available on `PATH`; `cargo fmt`, `cargo test`,
`cargo check`, and `cargo clippy` could not be run locally here.

## Local Environment Note

Observed locally:

- `cargo` is not recognized by PowerShell.
- `where.exe cargo` did not find Cargo.
- `%USERPROFILE%\.cargo\bin\cargo.exe` was not present.
- `git` is not on `PATH`, but `C:\Program Files\Git\cmd\git.exe` exists.

Before final verification, use a machine/session with Rust 1.82+ and Cargo
available.

## Required Commands

Run from the workspace root after the split:

```powershell
cargo fmt -- --check
cargo test
cargo check
cargo clippy -- -D warnings
cargo build --release
```

`cargo clippy` and `cargo build --release` are included because CI/release
workflows run them even though the project `AGENTS.md` minimum verification
lists fmt, test, and check.

## Client Crate Test Focus

Verify `crates/win-symlinks-client` covers:

- `CreateSymlinkOptions::new(target, link)` preserves Linux
  `ln -s TARGET LINK_NAME` ordering.
- Builder methods set `target_kind` and `replace_existing_symlink`.
- Relative `link_path` resolves against caller current directory.
- `target_path` remains preserved exactly as supplied.
- Broker request/response JSON remains byte-compatible with protocol v1.
- Broker error responses map to `WinSymlinksError`.
- Direct `CreateSymbolicLinkW` maps privilege failures to
  `PRIVILEGE_REQUIRED`.
- Client crate does not depend on `clap`, `windows-service`,
  `tracing-subscriber`, config, doctor, or broker server-only code.

## Main App Test Focus

Verify `crates/win-symlinks` covers:

- Existing `ln.rs` parsing tests still pass.
- `--win-kind=file|dir` still parses correctly after `TargetKind` moves out of
  the app crate.
- `ln.exe` imports creation behavior from `win_symlinks_client`.
- Broker server deserializes client-crate `CreateSymlinkRequest`.
- Doctor checks still compile and use the moved direct/pipe client helpers.
- Service management remains only in the app crate.
- Binary names remain:
  - `ln.exe`
  - `win-symlinks.exe`
  - `win-symlinks-broker.exe`

## Code Review Checklist

Review for boundary correctness:

- `win-symlinks-client` contains only SDK/client responsibilities.
- Broker server hosting, impersonation, audit logging, config loading, doctor,
  path blacklist enforcement, and service management stay in app crate.
- No fake fallback is introduced: no junctions, hardlinks, copies, or `.lnk`
  shortcuts.
- Protocol version remains `1`.
- JSON field names and enum spellings remain unchanged.
- `TargetKind` serialization remains `"file"` / `"directory"`.
- Direct creation uses `SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE` only for
  direct client attempts.
- Replacement of existing symlinks remains broker-mediated and revalidated.
- The client crate verifies the connected pipe server identity before sending a
  privileged request.
- Public docs point external Rust users to `win-symlinks-client`, not the app
  crate internals.

## Manual Windows Verification

Still required on Windows 11:

- Install and start `WinSymlinksBroker` from an administrator shell.
- From a non-admin shell, create file and directory symlinks through `ln.exe`.
- Confirm direct creation works where Windows permits it.
- Create missing-target links with `--win-kind=file` and `--win-kind=dir`.
- Replace an existing symbolic link with `ln -sf`.
- Confirm `ln -sf` refuses real files, real directories, and non-symlink
  reparse points.
- Confirm created objects are true Windows symbolic links.

## Known Verification Gap

This planning environment cannot run Rust verification because Cargo is absent.
Do not mark the client-crate split complete until the required Cargo commands
and manual Windows checks have passed in a Rust-enabled environment.
