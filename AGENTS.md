# AGENTS.md

## Project
`win-symlinks` provides a Linux-like `ln -s TARGET LINK_NAME` experience on Windows 11 using real Windows symbolic links.

The project must not emulate symlinks with junctions, hardlinks, file copies, or `.lnk` shortcuts. If a real symbolic link cannot be created, fail with a clear diagnostic.

## Stack
- Language: Rust.
- Target platform: Windows 11.
- Cargo package: `win-symlinks`.
- Main user command: `ln.exe`.
- Management/diagnostic command: `win-symlinks.exe`.
- Privileged broker service: `WinSymlinksBroker`.
- Current Windows dependency baseline: `windows = "0.62"`, `windows-service = "0.8"`.

## Commands
- Format check: `cargo fmt -- --check`.
- Test: `cargo test`.
- Build/check: `cargo check`.
- Run CLI locally: `cargo run --bin ln -- --help`.
- Run management CLI locally: `cargo run --bin win-symlinks -- --help`.

## Architecture
Normal flow:

```text
User shell -> ln.exe -> Named Pipe -> WinSymlinksBroker -> CreateSymbolicLinkW
```

`ln.exe` may try direct true symlink creation first. If direct creation fails because of privilege, it should call the broker. Direct and broker paths must preserve the same true symlink semantics.

Planned Rust binary layout:
- `src/bin/ln.rs` - Linux-compatible link command.
- `src/bin/win-symlinks.rs` - service management and diagnostics.
- `src/bin/win-symlinks-broker.rs` - Windows service broker.

Planned shared modules:
- `src/ipc`
- `src/path_policy`
- `src/symlink`
- `src/service`
- `src/config`

## Development Roadmap
Use `dev-docs/development-todo.md` as the stage checklist. Prefer completing one stage at a time; each stage must end with formatting, tests, and build checks.

## Boundaries
- Keep service management out of `ln.exe`.
- Do not make Developer Mode or Group Policy the primary mechanism.
- Use the source path blacklist as the primary path safety policy.
- The broker must validate caller identity, local-only IPC, request schema, caller write/create permission on the link parent, and blacklist policy before creating links.
- Do not invent new architecture beyond the design docs without updating the relevant `dev-docs/` document.
- Keep unimplemented privileged behavior explicit; do not add fake fallbacks or placeholder behavior that appears to create links.

## Sandbox Notes
Use this section to record commands that should be run with escalation immediately in this workspace, without first attempting a non-escalated run.
- `git add` — direct escalation required; sandboxed execution consistently fails with Git index lock or permission errors.
- `git commit` — direct escalation required; sandboxed execution consistently fails with Git index lock or permission errors.

## Build & Release
- Build release binaries: `cargo build --release`.
- Tag a release: `git tag v<VERSION> && git push origin v<VERSION>`.
- Pushing a `v*` tag triggers `.github/workflows/release.yml`, which builds, packages, and publishes a GitHub Release automatically.
- CI (fmt, clippy, test, build) runs on every push to `main` and every PR via `.github/workflows/ci.yml`.

## Verification
Before calling development work complete, run the relevant subset of:
- `cargo fmt -- --check`
- `cargo test`
- `cargo check`

Manual Windows verification is required for service registration, Named Pipe security, privilege boundaries, and true symlink creation.

## Read More Only When Needed
- Broad architecture or subsystem placement changes: read `dev-docs/architecture-navigation.md`.
- Symlink semantics, path policy, blacklist, replacement safety, or audit logging: read `dev-docs/design-core-policies.md`.
- Broker service, Named Pipe IPC, DACLs, validation, auth, or concurrency: read `dev-docs/design-broker-service.md`.
- `ln.exe`, `win-symlinks.exe`, service registration, diagnostics, or CLI errors: read `dev-docs/design-client-interfaces.md`.
- Test design, manual acceptance, admin/non-admin scenarios, or completion criteria: read `dev-docs/design-testing-qa.md`.
- Stage-by-stage implementation work: read `dev-docs/development-todo.md`.
- Editing this file or other agent harness instructions: read `dev-docs/howto-write-harness-agents-md.md`.
