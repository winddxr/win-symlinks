use crate::{config, ipc, path_policy, service, symlink, ErrorCode, Result, WinSymlinksError};
use serde::Serialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DoctorStatus {
    Pass,
    Warn,
    Fail,
    Info,
}

impl DoctorStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Fail => "FAIL",
            Self::Info => "INFO",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorCheck {
    pub name: String,
    pub status: DoctorStatus,
    pub detail: String,
}

impl DoctorCheck {
    fn new(
        name: impl Into<String>,
        status: DoctorStatus,
        detail: impl Into<String>,
    ) -> DoctorCheck {
        DoctorCheck {
            name: name.into(),
            status,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    pub fn has_failures(&self) -> bool {
        self.checks
            .iter()
            .any(|check| check.status == DoctorStatus::Fail)
    }
}

pub fn run_doctor() -> Result<()> {
    let report = collect_doctor_report();
    print_doctor_report(&report);

    if report.has_failures() {
        Err(WinSymlinksError::new(
            ErrorCode::ServiceUnavailable,
            "doctor found one or more failed checks",
        ))
    } else {
        Ok(())
    }
}

pub fn collect_doctor_report() -> DoctorReport {
    let mut checks = vec![
        platform_check(),
        filesystem_check(),
        developer_mode_check(),
        direct_symlink_check(),
    ];
    checks.extend(service_checks());
    checks.push(named_pipe_check());
    checks.push(DoctorCheck::new(
        "named_pipe_remote_clients",
        DoctorStatus::Info,
        "broker pipe is created with PIPE_REJECT_REMOTE_CLIENTS; remote rejection still needs manual verification",
    ));
    checks.push(DoctorCheck::new(
        "named_pipe_dacl",
        DoctorStatus::Info,
        "broker pipe uses the built-in SDDL policy; external DACL inspection still needs manual verification",
    ));
    checks.extend(path_checks());
    checks.extend(config_checks());

    DoctorReport { checks }
}

fn print_doctor_report(report: &DoctorReport) {
    println!("win-symlinks doctor");
    for check in &report.checks {
        println!(
            "[{}] {:<28} {}",
            check.status.label(),
            check.name,
            check.detail
        );
    }
}

fn platform_check() -> DoctorCheck {
    let version = windows_version_detail().unwrap_or_else(|| std::env::consts::OS.to_string());
    if !cfg!(windows) {
        return DoctorCheck::new(
            "platform",
            DoctorStatus::Fail,
            format!("{version}; win-symlinks requires Windows 11"),
        );
    }

    match parse_windows_build_number(&version) {
        Some(build) if build >= 22_000 => DoctorCheck::new("platform", DoctorStatus::Pass, version),
        Some(build) => DoctorCheck::new(
            "platform",
            DoctorStatus::Fail,
            format!("{version}; build {build} is older than Windows 11"),
        ),
        None => DoctorCheck::new(
            "platform",
            DoctorStatus::Warn,
            format!("{version}; could not verify Windows 11 build number"),
        ),
    }
}

fn filesystem_check() -> DoctorCheck {
    let current_dir = match std::env::current_dir() {
        Ok(path) => path,
        Err(err) => {
            return DoctorCheck::new(
                "filesystem",
                DoctorStatus::Fail,
                format!("failed to read current directory: {err}"),
            );
        }
    };

    match filesystem_name_for_path(&current_dir) {
        Ok(Some(name))
            if name.eq_ignore_ascii_case("NTFS") || name.eq_ignore_ascii_case("ReFS") =>
        {
            DoctorCheck::new(
                "filesystem",
                DoctorStatus::Pass,
                format!("{} is on {name}", current_dir.display()),
            )
        }
        Ok(Some(name)) => DoctorCheck::new(
            "filesystem",
            DoctorStatus::Warn,
            format!(
                "{} is on {name}; verify symbolic link support manually",
                current_dir.display()
            ),
        ),
        Ok(None) => DoctorCheck::new(
            "filesystem",
            DoctorStatus::Info,
            format!(
                "filesystem type for {} is unavailable on this platform",
                current_dir.display()
            ),
        ),
        Err(err) => DoctorCheck::new(
            "filesystem",
            DoctorStatus::Warn,
            format!("failed to inspect filesystem type: {err}"),
        ),
    }
}

fn developer_mode_check() -> DoctorCheck {
    match developer_mode_enabled() {
        Some(true) => DoctorCheck::new(
            "developer_mode",
            DoctorStatus::Pass,
            "Developer Mode appears enabled",
        ),
        Some(false) => DoctorCheck::new(
            "developer_mode",
            DoctorStatus::Info,
            "Developer Mode appears disabled; broker fallback is expected for non-admin users",
        ),
        None => DoctorCheck::new(
            "developer_mode",
            DoctorStatus::Info,
            "Developer Mode status could not be determined",
        ),
    }
}

fn direct_symlink_check() -> DoctorCheck {
    match try_direct_temp_symlink() {
        Ok(()) => DoctorCheck::new(
            "direct_symlink_create",
            DoctorStatus::Pass,
            "direct unprivileged CreateSymbolicLinkW succeeded in a temp directory",
        ),
        Err(err) if err.code() == ErrorCode::PrivilegeRequired => DoctorCheck::new(
            "direct_symlink_create",
            DoctorStatus::Info,
            "direct symlink creation needs privilege; broker flow should handle non-admin creation",
        ),
        Err(err) => DoctorCheck::new(
            "direct_symlink_create",
            DoctorStatus::Warn,
            format!("direct symlink probe failed: {err}"),
        ),
    }
}

fn service_checks() -> Vec<DoctorCheck> {
    match service::query_service_state() {
        Ok(service::ServiceState::NotInstalled) => vec![
            DoctorCheck::new(
                "service_installed",
                DoctorStatus::Fail,
                format!("{} is not installed", service::SERVICE_NAME),
            ),
            DoctorCheck::new(
                "service_running",
                DoctorStatus::Fail,
                format!("{} is not running", service::SERVICE_NAME),
            ),
        ],
        Ok(service::ServiceState::Running) => vec![
            DoctorCheck::new(
                "service_installed",
                DoctorStatus::Pass,
                format!("{} is installed", service::SERVICE_NAME),
            ),
            DoctorCheck::new(
                "service_running",
                DoctorStatus::Pass,
                format!("{} is running", service::SERVICE_NAME),
            ),
        ],
        Ok(state) => vec![
            DoctorCheck::new(
                "service_installed",
                DoctorStatus::Pass,
                format!("{} is installed", service::SERVICE_NAME),
            ),
            DoctorCheck::new(
                "service_running",
                DoctorStatus::Fail,
                format!("{} state is {state}", service::SERVICE_NAME),
            ),
        ],
        Err(err) => vec![
            DoctorCheck::new(
                "service_installed",
                DoctorStatus::Fail,
                format!("failed to query service: {err}"),
            ),
            DoctorCheck::new(
                "service_running",
                DoctorStatus::Fail,
                format!("failed to query service: {err}"),
            ),
        ],
    }
}

fn named_pipe_check() -> DoctorCheck {
    match ipc::check_broker_pipe() {
        Ok(()) => DoctorCheck::new(
            "named_pipe_identity",
            DoctorStatus::Pass,
            format!(
                "{} is reachable and served by the installed broker process",
                ipc::PIPE_NAME
            ),
        ),
        Err(err) => DoctorCheck::new(
            "named_pipe_identity",
            DoctorStatus::Fail,
            format!("failed to verify broker pipe: {err}"),
        ),
    }
}

fn path_checks() -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    let ln_paths = resolve_path_candidates("ln.exe");
    let current_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));

    match ln_paths.first() {
        Some(first) => {
            let expected = current_dir.as_ref().map(|dir| dir.join("ln.exe"));
            let belongs_to_this_build = expected
                .as_ref()
                .is_some_and(|expected| same_path(first, expected));
            checks.push(DoctorCheck::new(
                "path_ln_resolution",
                if belongs_to_this_build {
                    DoctorStatus::Pass
                } else {
                    DoctorStatus::Warn
                },
                format!("first ln.exe on PATH is {}", first.display()),
            ));

            if belongs_to_this_build {
                checks.push(DoctorCheck::new(
                    "path_ln_owner",
                    DoctorStatus::Pass,
                    "PATH resolves ln.exe from the same directory as win-symlinks.exe",
                ));
            } else if let Some(expected) = expected {
                checks.push(DoctorCheck::new(
                    "path_ln_owner",
                    DoctorStatus::Warn,
                    format!(
                        "expected {} to appear before other ln.exe entries",
                        expected.display()
                    ),
                ));
            }
        }
        None => {
            checks.push(DoctorCheck::new(
                "path_ln_resolution",
                DoctorStatus::Warn,
                "ln.exe was not found on PATH",
            ));
            checks.push(DoctorCheck::new(
                "path_ln_owner",
                DoctorStatus::Warn,
                "add the directory containing win-symlinks ln.exe to PATH",
            ));
        }
    }

    let conflicts: Vec<PathBuf> = ln_paths
        .into_iter()
        .filter(|path| path_looks_like_known_ln_conflict(path))
        .collect();
    if conflicts.is_empty() {
        checks.push(DoctorCheck::new(
            "path_ln_conflicts",
            DoctorStatus::Pass,
            "no Git/MSYS2/Cygwin/BusyBox/coreutils ln.exe candidates were detected on PATH",
        ));
    } else {
        checks.push(DoctorCheck::new(
            "path_ln_conflicts",
            DoctorStatus::Warn,
            format!(
                "possible conflicting ln.exe entries: {}",
                join_paths_for_display(&conflicts)
            ),
        ));
    }

    checks
}

fn config_checks() -> Vec<DoctorCheck> {
    match config::load_config() {
        Ok(config) => {
            let effective =
                path_policy::merge_source_blacklist(&config.additional_source_blacklist);
            let existing_defaults = path_policy::built_in_source_blacklist()
                .entries()
                .iter()
                .filter(|entry| entry.path.exists())
                .count();
            vec![
                DoctorCheck::new(
                    "config_load",
                    DoctorStatus::Pass,
                    format!("loaded effective config from {}", config::default_config_path().display()),
                ),
                DoctorCheck::new(
                    "source_blacklist",
                    DoctorStatus::Pass,
                    format!(
                        "{} effective source blacklist entries",
                        effective.entries().len()
                    ),
                ),
                DoctorCheck::new(
                    "default_blacklist_paths",
                    if existing_defaults == 0 {
                        DoctorStatus::Warn
                    } else {
                        DoctorStatus::Pass
                    },
                    format!("{existing_defaults} built-in source blacklist entries exist on this machine"),
                ),
            ]
        }
        Err(err) => vec![DoctorCheck::new(
            "config_load",
            DoctorStatus::Fail,
            format!("failed to load config: {err}"),
        )],
    }
}

fn resolve_path_candidates(executable_name: &str) -> Vec<PathBuf> {
    let Some(path_var) = std::env::var_os("PATH") else {
        return Vec::new();
    };
    std::env::split_paths(&path_var)
        .map(|dir| dir.join(executable_name))
        .filter(|candidate| candidate.is_file())
        .collect()
}

fn same_path(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => left
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy()),
    }
}

fn path_looks_like_known_ln_conflict(path: &Path) -> bool {
    let raw = path.to_string_lossy().to_ascii_lowercase();
    raw.contains(r"\git\usr\bin\")
        || raw.contains(r"\msys")
        || raw.contains(r"\cygwin")
        || raw.contains(r"\busybox")
        || raw.contains(r"\coreutils")
}

fn join_paths_for_display(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

fn windows_version_detail() -> Option<String> {
    if !cfg!(windows) {
        return None;
    }

    let output = std::process::Command::new("cmd")
        .args(["/C", "ver"])
        .output()
        .ok()?;
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        None
    } else {
        Some(raw)
    }
}

fn parse_windows_build_number(version: &str) -> Option<u32> {
    let version_start = version.find("10.0.")? + "10.0.".len();
    let rest = &version[version_start..];
    let digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse().ok()
}

fn developer_mode_enabled() -> Option<bool> {
    if !cfg!(windows) {
        return None;
    }

    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\AppModelUnlock",
            "/v",
            "AllowDevelopmentWithoutDevLicense",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return Some(false);
    }

    let text = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    Some(text.contains("0x1") || text.contains("0x00000001"))
}

fn try_direct_temp_symlink() -> Result<()> {
    if !cfg!(windows) {
        return Err(WinSymlinksError::new(
            ErrorCode::CreateSymlinkFailed,
            "direct symlink probe is only supported on Windows",
        ));
    }

    let unique = uuid::Uuid::now_v7();
    let link = std::env::temp_dir().join(format!("win-symlinks-doctor-{unique}.link"));
    let target = std::env::temp_dir().join(format!("win-symlinks-doctor-{unique}.target"));
    let result = symlink::create_symbolic_link(&link, &target, symlink::TargetKind::File, true);
    if std::fs::symlink_metadata(&link).is_ok() {
        let _ = std::fs::remove_file(&link);
    }
    result
}

#[cfg(windows)]
fn filesystem_name_for_path(path: &Path) -> Result<Option<String>> {
    platform::filesystem_name_for_path(path)
}

#[cfg(not(windows))]
fn filesystem_name_for_path(_path: &Path) -> Result<Option<String>> {
    Ok(None)
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::{
        core::PCWSTR,
        Win32::Storage::FileSystem::{GetVolumeInformationW, GetVolumePathNameW},
    };

    pub(super) fn filesystem_name_for_path(path: &Path) -> Result<Option<String>> {
        let path_wide = wide_null(path.as_os_str());
        let mut volume_path = vec![0u16; 261];
        unsafe { GetVolumePathNameW(PCWSTR(path_wide.as_ptr()), &mut volume_path) }.map_err(
            |err| {
                WinSymlinksError::new(
                    ErrorCode::ServiceUnavailable,
                    format!("failed to resolve volume path: {err}"),
                )
            },
        )?;

        let mut fs_name = vec![0u16; 64];
        unsafe {
            GetVolumeInformationW(
                PCWSTR(volume_path.as_ptr()),
                None,
                None,
                None,
                None,
                Some(&mut fs_name),
            )
        }
        .map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!("failed to read volume information: {err}"),
            )
        })?;

        Ok(Some(wide_to_string(&fs_name)))
    }

    fn wide_null(value: &std::ffi::OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
    }

    fn wide_to_string(value: &[u16]) -> String {
        let end = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
        OsString::from_wide(&value[..end])
            .to_string_lossy()
            .into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_path_conflict_detection_matches_common_toolchains() {
        assert!(path_looks_like_known_ln_conflict(Path::new(
            r"C:\Program Files\Git\usr\bin\ln.exe"
        )));
        assert!(path_looks_like_known_ln_conflict(Path::new(
            r"C:\msys64\usr\bin\ln.exe"
        )));
        assert!(!path_looks_like_known_ln_conflict(Path::new(
            r"C:\tools\win-symlinks\ln.exe"
        )));
    }

    #[test]
    fn doctor_report_failure_detection_uses_fail_status_only() {
        let report = DoctorReport {
            checks: vec![DoctorCheck::new("warning", DoctorStatus::Warn, "detail")],
        };
        assert!(!report.has_failures());

        let report = DoctorReport {
            checks: vec![DoctorCheck::new("failure", DoctorStatus::Fail, "detail")],
        };
        assert!(report.has_failures());
    }

    #[test]
    fn parses_windows_build_number_from_cmd_version_output() {
        assert_eq!(
            parse_windows_build_number("Microsoft Windows [Version 10.0.26200.8246]"),
            Some(26200)
        );
        assert_eq!(parse_windows_build_number("linux"), None);
    }
}
