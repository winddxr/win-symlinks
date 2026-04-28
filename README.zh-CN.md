# win-symlinks

> [English](README.md) | **简体中文**

`win-symlinks` 为 Windows 11 提供接近 Linux `ln -s TARGET LINK_NAME` 的使用体验，并且只创建真实的 Symbolic Link (符号链接)。

## 项目目标 (Project Goals)

- 不需要每次创建符号链接时都以管理员权限运行 `ln.exe` 或 `win-symlinks.exe` 的同时确保安全性。
- 只使用真实的 Windows Symbolic Link (符号链接)
- 不使用 Junction (目录联接)、Hard Link (硬链接)、文件复制、目录复制或 `.lnk` 快捷方式做伪替代
- 如果当前环境无法创建真实符号链接，就直接失败并输出清晰诊断

## 概述 (Overview)

`win-symlinks` 由三个可执行文件组成：

- `ln.exe`：面向终端用户的 Linux 风格链接命令
- `win-symlinks.exe`：管理与诊断命令
- `win-symlinks-broker.exe`：以 Windows Service (Windows 服务) 方式运行的特权代理

正常工作流如下：

```text
User Shell (用户终端) -> ln.exe -> Named Pipe (命名管道) -> WinSymlinksBroker -> CreateSymbolicLinkW
```

`ln.exe` 会优先尝试直接创建真实符号链接；如果因为权限原因失败，则自动回退到 Broker Service (代理服务) 路径。两条路径都必须保持一致的真实符号链接语义。

## 功能特性 (Features)

- 提供 Linux 风格的 `ln -s`、`ln -sf`、`ln -sT`
- 支持 `--win-kind=file|dir` 处理目标尚不存在的 Windows 场景
- 通过本地 Named Pipe (命名管道) 与 `WinSymlinksBroker` 通信
- 非管理员用户在服务已安装时也可创建真实符号链接
- 提供 `doctor` 诊断命令，检查服务状态、PATH 冲突和当前 `ln.exe` 解析结果
- 提供可配置的 Source Blacklist (源路径黑名单) 保护策略

## 系统要求 (Requirements)

- Windows 11
- Rust `1.82+`
- 安装/启动/卸载服务时需要管理员 PowerShell 或管理员命令提示符
- 运行目录建议位于 NTFS 或 ReFS 文件系统

## 构建 (Build)

在项目根目录执行：

```powershell
cargo build --release
```

生成的二进制位于：

```text
target\release\ln.exe
target\release\win-symlinks.exe
target\release\win-symlinks-broker.exe
```

## 安装 (Install)

当前项目没有 MSI 安装器，推荐使用"构建后复制可执行文件"的方式安装。

### 1. 选择安装目录

推荐使用一个固定目录，例如：

```text
C:\Program Files\win-symlinks
```

或当前用户目录：

```text
%LOCALAPPDATA%\Programs\win-symlinks
```

### 2. 复制三个可执行文件

把以下三个文件放到同一个目录中：

```text
ln.exe
win-symlinks.exe
win-symlinks-broker.exe
```

注意：`win-symlinks.exe service install` 会在自己的同级目录中查找 `win-symlinks-broker.exe`。因此这两个文件必须放在同一个目录里，最简单的做法是三个文件全部同目录部署。

## 安装服务 (Install Service)

以管理员 PowerShell 打开安装目录，然后执行：

```powershell
.\win-symlinks.exe service install
.\win-symlinks.exe service start
.\win-symlinks.exe service status
```

如果已经把安装目录加入 `PATH`，也可以直接执行：

```powershell
win-symlinks service install
win-symlinks service start
win-symlinks service status
```

成功后将会注册并启动以下服务：

- Service Name (服务名称): `WinSymlinksBroker`
- Display Name (显示名称): `Win Symlinks Broker`
- Account (运行账户): `LocalSystem`

卸载服务：

```powershell
win-symlinks service stop
win-symlinks service uninstall
```

## 在 Windows 上使用 `ln` (Make `ln` Available On Windows)

这里需要区分两类命令：

- `cd` 是 Shell Built-in (Shell 内建命令)
- `npm` 是通过 `PATH` 找到的可执行文件命令

`ln.exe` 在 Windows 上的工作方式更接近 `npm`，不是 `cd` 这种内建命令。因此要让你在任意目录下直接输入 `ln`，核心就是把 `ln.exe` 所在目录加入 `PATH`。

### PowerShell 配置用户级 PATH

假设安装目录为：

```text
$env:LOCALAPPDATA\Programs\win-symlinks
```

可以执行：

```powershell
$InstallDir = "$env:LOCALAPPDATA\Programs\win-symlinks"
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ([string]::IsNullOrWhiteSpace($UserPath)) {
    [Environment]::SetEnvironmentVariable("Path", $InstallDir, "User")
} elseif ($UserPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable("Path", ($UserPath.TrimEnd(';') + ";" + $InstallDir), "User")
}
```

然后：

1. 关闭当前终端
2. 重新打开 PowerShell 或 Windows Terminal
3. 执行 `where.exe ln`

如果输出指向你的安装目录，例如：

```text
C:\Users\<你>\AppData\Local\Programs\win-symlinks\ln.exe
```

说明 `ln` 已经可以像 `npm` 一样直接调用。

### 推荐的 PATH 排序

Windows 上常见的 `ln.exe` 冲突来源包括：

- Git for Windows
- MSYS2
- Cygwin
- BusyBox
- coreutils

如果你已经安装这些工具，请确保 `win-symlinks` 的目录排在它们前面，否则终端里输入 `ln` 时，可能会先命中别的实现。

建议检查：

```powershell
where.exe ln
win-symlinks doctor
```

## 用法 (Usage)

常见用法：

```powershell
ln -s target.txt link.txt
ln -sf target.txt link.txt
ln -sT target.txt link.txt
ln -s --win-kind=file future-target.txt future-link.txt
ln -s --win-kind=dir future-target-dir future-link-dir
```

帮助与版本信息：

```powershell
ln --help
ln --version
win-symlinks --help
```

管理与诊断：

```powershell
win-symlinks service status
win-symlinks doctor
win-symlinks config show
```

## 配置 (Configuration)

默认配置文件路径：

```text
C:\ProgramData\win-symlinks\config.json
```

当前支持的配置项包括：

- `additional_source_blacklist` — 附加源路径黑名单
- `allow_direct_create_attempt` — 是否允许直接创建尝试

查看生效配置：

```powershell
win-symlinks config show
```

## 验证 (Verification)

开发验证命令：

```powershell
cargo fmt -- --check
cargo test
cargo check
```

安装后的建议检查命令：

```powershell
win-symlinks service status
win-symlinks doctor
where.exe ln
```

## 故障排除 (Troubleshooting)

### `win-symlinks service install` 提示找不到 Broker

请确认 `win-symlinks.exe` 与 `win-symlinks-broker.exe` 在同一目录。

### `ln` 调到了别的程序

执行：

```powershell
where.exe ln
```

如果第一条结果不是你安装的 `ln.exe`，请把 `win-symlinks` 安装目录移动到 `PATH` 更靠前的位置。

### 非管理员 `ln -s` 失败

先检查：

```powershell
win-symlinks service status
win-symlinks doctor
```

如果服务未安装或未启动，请使用管理员终端执行安装与启动命令。

## 当前状态 (Status)

当前仓库已经实现以下能力：

- `ln.exe` 命令解析与真实符号链接创建
- `WinSymlinksBroker` 服务注册、启动、停止、卸载
- 本地 Named Pipe (命名管道) IPC
- 路径黑名单与基础安全校验
- `doctor` 与 `config show` 诊断能力

项目当前更偏向"可构建、可验证、可手动部署"的开发者安装方式，而不是面向普通用户的一键安装包。

## 许可证 (License)

双许可证 (Dual License)：

- MIT
- Apache-2.0
