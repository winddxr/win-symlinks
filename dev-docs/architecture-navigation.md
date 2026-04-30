# win-symlinks 架构与开发导航 (Architecture Navigation)

## 摘要 (Summary)

`win-symlinks` aims to provide a Linux-like symbolic link experience on Windows 11 while keeping the underlying implementation strictly based on real Windows symbolic links.

The project does not use junctions, hardlinks, file copies, or `.lnk` shortcuts as semantic fallbacks. If a real symbolic link cannot be created, the command must fail with a clear diagnostic.

The user-facing command is `ln.exe`, aligned as closely as practical with Linux `ln`. Administrative and diagnostic operations are exposed through `win-symlinks.exe`. The privileged link creation work is performed by a separate Windows Service named `WinSymlinksBroker`.

## 目标与非目标 (Goals And Non-Goals)

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

## 高层级架构架构 (Architecture / Process Model)

The normal execution path is:

```text
User shell
  -> ln.exe
  -> win_symlinks::client
  -> direct CreateSymbolicLinkW or Named Pipe request
  -> WinSymlinksBroker when broker privileges are needed
  -> CreateSymbolicLinkW
  -> response to ln.exe
```

External Rust projects should use the same shared client API:

```text
External Rust project
  -> win_symlinks::client
  -> direct CreateSymbolicLinkW or Named Pipe request
  -> WinSymlinksBroker when broker privileges are needed
  -> CreateSymbolicLinkW
```

`ln.exe` is a command-line frontend for Linux-compatible argument behavior. It should call `win_symlinks::client` rather than owning the reusable direct-create and broker-fallback orchestration. External projects should integrate through the client API or documented IPC contract, not by copying `src/bin/ln.rs`.

The client API may first attempt direct symbolic link creation if the current process has the required privilege or Windows allows unprivileged creation. If direct creation fails because of privilege, it must call the broker. Direct creation and broker creation must both use the same true symbolic link semantics.

The broker is the stable path for systems where Developer Mode is disabled and the user account does not have `SeCreateSymbolicLinkPrivilege`.

## Rust 实现规划 (Rust Implementation Plan)

### Crates

Use these Rust dependencies:

```toml
[dependencies]
windows-service = "0.8"
windows = { version = "0.62", features = [
  "Win32_Foundation",
  "Win32_Storage_FileSystem",
  "Win32_System_Services",
  "Win32_System_Pipes",
  "Win32_System_Threading",
  "Win32_Security",
  "Win32_Security_Authorization",
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
src/client
src/path_policy
src/symlink
src/service
src/config
```

### Responsibilities

`ln.exe`:
- Parse Linux-compatible arguments.
- Decide CLI-specific link path, target path, force mode, and target kind.
- Call `win_symlinks::client` for direct true symlink creation and broker fallback.
- Print Linux-like errors.

`win_symlinks::client`:
- Provide the stable Rust integration API for creating real Windows symbolic links.
- Preserve `ln -s TARGET LINK_NAME` target/link ordering in its public options.
- Resolve relative `link_path` against the caller current directory.
- Attempt direct true symlink creation when appropriate.
- Call the broker on privilege failure or broker-only operations.
- Never fall back to junctions, hardlinks, copies, or `.lnk` shortcuts.

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

---

## 开发上下文导航 (Design Drill-Down Documents)

为了降低认知负担并匹配实际开发边界，详细的设计规范已拆分为以下独立的子文档：

1. **[核心机制与安全策略](./design-core-policies.md)**
   - 威胁模型与安全限制
   - 源路径黑名单策略 (Source Blacklist Security Policy)
   - 路径解析、真实符号链接语义与安全替换逻辑
   - 审计日志

2. **[Broker 服务端设计](./design-broker-service.md)**
   - `WinSymlinksBroker` Windows 服务配置
   - 命名管道 (Named Pipe) IPC 设计与安全 DACL
   - Broker 验证与鉴权流程 (Broker Validation Flow)
   - 并发模型设计

3. **[客户端 CLI 与管理工具](./design-client-interfaces.md)**
   - `win_symlinks::client` Rust API, `ln.exe` (Linux 兼容命令) 与 `win-symlinks.exe` (管理命令) 接口定义
   - 无安装器的服务注册机制
   - 错误处理规范
   - 诊断 (Diagnostics) 流程

4. **[测试计划与验收标准](./design-testing-qa.md)**
   - 单元测试与集成测试案例规划
   - 手动验收场景 (Administrator vs Non-Admin)
   - 项目整体验收标准 (Acceptance Criteria)
