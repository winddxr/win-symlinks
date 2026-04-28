use crate::{ErrorCode, Result, WinSymlinksError};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlacklistSource {
    BuiltIn,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlacklistEntry {
    pub path: PathBuf,
    pub source: BlacklistSource,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveBlacklist {
    entries: Vec<BlacklistEntry>,
}

impl EffectiveBlacklist {
    pub fn new(entries: Vec<BlacklistEntry>) -> Self {
        Self { entries }
    }

    pub fn entries(&self) -> &[BlacklistEntry] {
        &self.entries
    }

    pub fn is_blocked(&self, path: &Path) -> Result<Option<&BlacklistEntry>> {
        let candidate = normalize_for_policy(path)?;

        if is_volume_root(&candidate) || is_unc_admin_share(&candidate) {
            return Ok(self.entries.first());
        }

        Ok(self
            .entries
            .iter()
            .find(|entry| path_has_component_prefix(&candidate, &entry.path)))
    }
}

pub fn built_in_source_blacklist() -> EffectiveBlacklist {
    let mut paths = vec![
        PathBuf::from(r"C:\Windows"),
        PathBuf::from(r"C:\Program Files"),
        PathBuf::from(r"C:\Program Files (x86)"),
        PathBuf::from(r"C:\ProgramData"),
        PathBuf::from(r"C:\System Volume Information"),
        PathBuf::from(r"C:\$Recycle.Bin"),
    ];

    for key in [
        "SystemRoot",
        "WINDIR",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
    ] {
        if let Some(value) = std::env::var_os(key) {
            paths.push(PathBuf::from(value));
        }
    }

    paths.sort();
    paths.dedup();

    EffectiveBlacklist::new(
        paths
            .into_iter()
            .map(|path| BlacklistEntry {
                path,
                source: BlacklistSource::BuiltIn,
            })
            .collect(),
    )
}

pub fn normalize_for_policy(path: &Path) -> Result<PathBuf> {
    let raw = path.to_string_lossy().replace('/', r"\");
    if raw.trim().is_empty() {
        return Err(policy_error("path is empty"));
    }
    if has_alternate_data_stream_syntax(&raw) {
        return Err(policy_error(
            "link path contains NTFS alternate data stream syntax",
        ));
    }

    let absolute = make_absolute(&raw)?;
    let collapsed = collapse_dot_segments(&absolute)?;
    Ok(PathBuf::from(collapsed))
}

pub fn path_has_component_prefix(path: &Path, prefix: &Path) -> bool {
    let Ok(path) = normalize_for_policy(path) else {
        return false;
    };
    let Ok(prefix) = normalize_for_policy(prefix) else {
        return false;
    };

    let path = trim_trailing_separators(&path.to_string_lossy()).to_ascii_lowercase();
    let prefix = trim_trailing_separators(&prefix.to_string_lossy()).to_ascii_lowercase();

    path == prefix || path.starts_with(&format!(r"{prefix}\"))
}

fn make_absolute(raw: &str) -> Result<String> {
    if is_drive_absolute(raw) || raw.starts_with(r"\\") {
        return Ok(raw.to_string());
    }
    if is_drive_relative(raw) {
        return Err(policy_error("drive-relative paths are not accepted"));
    }

    let current_dir = std::env::current_dir()
        .map_err(|err| policy_error(format!("failed to read current directory: {err}")))?;
    Ok(format!(
        r"{}\{}",
        trim_trailing_separators(&current_dir.to_string_lossy()),
        raw
    ))
}

fn collapse_dot_segments(raw: &str) -> Result<String> {
    let mut prefix = String::new();
    let mut segments: Vec<&str> = Vec::new();

    if is_drive_absolute(raw) {
        prefix.push_str(&raw[..3]);
        segments.extend(raw[3..].split('\\'));
    } else if let Some(stripped) = raw.strip_prefix(r"\\") {
        let parts: Vec<&str> = stripped.split('\\').collect();
        let server = parts.first().copied().unwrap_or_default();
        let share = parts.get(1).copied().unwrap_or_default();
        if server.is_empty() || share.is_empty() {
            return Err(policy_error("UNC path must include server and share"));
        }
        prefix = format!(r"\\{server}\{share}\");
        segments.extend(parts.into_iter().skip(2));
    } else {
        segments.extend(raw.split('\\'));
    }

    let mut stack: Vec<&str> = Vec::new();
    for part in segments {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            stack.pop();
        } else {
            if component_has_unsafe_trailing_chars(part) {
                return Err(policy_error(
                    "path components with trailing spaces or dots are not accepted",
                ));
            }
            stack.push(part);
        }
    }

    let suffix = stack.join(r"\");
    if suffix.is_empty() {
        Ok(prefix)
    } else {
        Ok(format!("{prefix}{suffix}"))
    }
}

fn has_alternate_data_stream_syntax(raw: &str) -> bool {
    for (index, _) in raw.match_indices(':') {
        let is_drive_colon = index == 1 && raw.as_bytes()[0].is_ascii_alphabetic();
        if !is_drive_colon {
            return true;
        }
    }
    false
}

fn is_drive_absolute(raw: &str) -> bool {
    raw.len() >= 3
        && raw.as_bytes()[0].is_ascii_alphabetic()
        && raw.as_bytes()[1] == b':'
        && raw.as_bytes()[2] == b'\\'
}

fn is_drive_relative(raw: &str) -> bool {
    raw.len() >= 2
        && raw.as_bytes()[0].is_ascii_alphabetic()
        && raw.as_bytes()[1] == b':'
        && !is_drive_absolute(raw)
}

fn is_volume_root(path: &Path) -> bool {
    let path_text = path.to_string_lossy();
    let path = trim_trailing_separators(&path_text);
    path.len() == 2 && path.as_bytes()[0].is_ascii_alphabetic() && path.as_bytes()[1] == b':'
}

fn is_unc_admin_share(path: &Path) -> bool {
    let raw = path.to_string_lossy();
    let Some(stripped) = raw.strip_prefix(r"\\") else {
        return false;
    };
    let mut parts = stripped.split('\\');
    let _server = parts.next();
    let Some(share) = parts.next() else {
        return false;
    };
    share.ends_with('$')
}

fn component_has_unsafe_trailing_chars(component: &str) -> bool {
    component.ends_with(' ') || component.ends_with('.')
}

fn trim_trailing_separators(path: &str) -> &str {
    path.trim_end_matches(['\\', '/'])
}

fn policy_error(message: impl Into<String>) -> WinSymlinksError {
    WinSymlinksError::new(ErrorCode::PathNormalizationFailed, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_blacklisted_prefix_by_component() {
        assert!(path_has_component_prefix(
            Path::new(r"C:\Windows\System32"),
            Path::new(r"C:\Windows")
        ));
    }

    #[test]
    fn does_not_match_sibling_prefix() {
        assert!(!path_has_component_prefix(
            Path::new(r"C:\WindowsTools"),
            Path::new(r"C:\Windows")
        ));
    }

    #[test]
    fn rejects_ads_syntax_after_drive_colon() {
        let err = normalize_for_policy(Path::new(r"C:\work\file.txt:stream")).unwrap_err();
        assert_eq!(err.code(), ErrorCode::PathNormalizationFailed);
    }

    #[test]
    fn rejects_drive_relative_paths() {
        let err = normalize_for_policy(Path::new(r"C:work\link")).unwrap_err();
        assert_eq!(err.code(), ErrorCode::PathNormalizationFailed);
    }

    #[test]
    fn collapses_dot_segments() {
        assert_eq!(
            normalize_for_policy(Path::new(r"C:\work\.\a\..\b")).unwrap(),
            PathBuf::from(r"C:\work\b")
        );
    }
}
