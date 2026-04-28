# Core Concepts & Security Policies (win-symlinks)

This document contains the core mechanisms, threat models, and safety policies for the `win-symlinks` project.

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
