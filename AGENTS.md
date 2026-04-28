# AGENTS.md

## Project
`win-symlinks` provides a Linux-like `ln -s TARGET LINK_NAME` experience on Windows 11 using real Windows symbolic links.

The project must not emulate symlinks with junctions, hardlinks, file copies, or `.lnk` shortcuts. If a real symbolic link cannot be created, fail with a clear diagnostic.

## Stack
- Language: Rust.
- Target platform: Windows 11.
- Main user command: `ln.exe`.
- Management/diagnostic command: `win-symlinks.exe`.
- Privileged broker service: `WinSymlinksBroker`.

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

## Boundaries
- Keep service management out of `ln.exe`.
- Do not make Developer Mode or Group Policy the primary mechanism.
- Use the source path blacklist as the primary path safety policy.
- The broker must validate caller identity, local-only IPC, request schema, caller write/create permission on the link parent, and blacklist policy before creating links.
- Do not invent new architecture beyond the design docs without updating the relevant `dev-docs/` document.

## Sandbox Notes
Use this section to record commands that should be run with escalation immediately in this workspace, without first attempting a non-escalated run.
- `git add` — direct escalation required; sandboxed execution consistently fails with Git index lock or permission errors.
- `git commit` — direct escalation required; sandboxed execution consistently fails with Git index lock or permission errors.

## Verification
There is no Cargo project or project-level build command yet. After implementation begins, verify relevant changes with Rust formatting, tests, and build checks before calling work complete.

## Read More Only When Needed
- Broad architecture or subsystem placement changes: read `dev-docs/architecture-navigation.md`.
- Symlink semantics, path policy, blacklist, replacement safety, or audit logging: read `dev-docs/design-core-policies.md`.
- Broker service, Named Pipe IPC, DACLs, validation, auth, or concurrency: read `dev-docs/design-broker-service.md`.
- `ln.exe`, `win-symlinks.exe`, service registration, diagnostics, or CLI errors: read `dev-docs/design-client-interfaces.md`.
- Test design, manual acceptance, admin/non-admin scenarios, or completion criteria: read `dev-docs/design-testing-qa.md`.
- Editing this file or other agent harness instructions: read `dev-docs/howto-write-harness-agents-md.md`.
