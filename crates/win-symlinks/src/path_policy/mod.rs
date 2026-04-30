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

    pub fn is_blocked(&self, path: &Path) -> Result<Option<BlacklistEntry>> {
        let candidate = normalize_for_policy(path)?;

        if is_volume_root(&candidate) || is_unc_admin_share(&candidate) {
            return Ok(Some(BlacklistEntry {
                path: candidate,
                source: BlacklistSource::BuiltIn,
            }));
        }

        if is_other_users_profile_path(&candidate)? {
            return Ok(Some(BlacklistEntry {
                path: candidate,
                source: BlacklistSource::BuiltIn,
            }));
        }

        Ok(self
            .entries
            .iter()
            .find(|entry| path_has_component_prefix(&candidate, &entry.path))
            .cloned())
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

    EffectiveBlacklist::new(entries_from_paths(paths, BlacklistSource::BuiltIn))
}

pub fn merge_source_blacklist(user_entries: &[PathBuf]) -> EffectiveBlacklist {
    let mut entries = built_in_source_blacklist().entries().to_vec();
    entries.extend(entries_from_paths(
        user_entries.iter().cloned(),
        BlacklistSource::User,
    ));
    entries.sort_by_key(|entry| entry.path.to_string_lossy().to_ascii_lowercase());
    entries.dedup_by(|left, right| {
        left.source == right.source
            && left
                .path
                .to_string_lossy()
                .eq_ignore_ascii_case(&right.path.to_string_lossy())
    });
    EffectiveBlacklist::new(entries)
}

pub fn normalize_for_policy(path: &Path) -> Result<PathBuf> {
    let raw = path.to_string_lossy().replace('/', r"\");
    if raw.trim().is_empty() {
        return Err(policy_error("path is empty"));
    }
    let raw = canonicalize_supported_windows_prefix(&raw)?;

    if has_alternate_data_stream_syntax(&raw) {
        return Err(policy_error(
            "link path contains NTFS alternate data stream syntax",
        ));
    }

    let absolute = make_absolute(&raw)?;
    let collapsed = collapse_dot_segments(&absolute)?;
    let path = PathBuf::from(collapsed);

    if should_canonicalize_full_path(&path) {
        if let Ok(canonical) = std::fs::canonicalize(&path) {
            let canonical = canonical.to_string_lossy().replace('/', r"\");
            let canonical = canonicalize_supported_windows_prefix(&canonical)?;
            return Ok(PathBuf::from(collapse_dot_segments(&canonical)?));
        }
    }

    Ok(path)
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
    if raw.starts_with('\\') {
        return Err(policy_error(
            "root-relative paths are not accepted because they depend on the current drive",
        ));
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

fn canonicalize_supported_windows_prefix(raw: &str) -> Result<String> {
    if raw.starts_with(r"\\.\") || raw.starts_with(r"\??\") || raw.starts_with(r"\Device\") {
        return Err(policy_error("native device paths are not accepted"));
    }

    let Some(stripped) = raw.strip_prefix(r"\\?\") else {
        return Ok(raw.to_string());
    };

    if stripped
        .get(..11)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"GLOBALROOT\"))
    {
        return Err(policy_error("global native paths are not accepted"));
    }

    if is_drive_absolute(stripped) {
        return Ok(stripped.to_string());
    }

    if stripped
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(r"UNC\"))
    {
        let unc_body = &stripped[4..];
        if unc_body.starts_with('\\') {
            return Err(policy_error("extended UNC path is malformed"));
        }
        return Ok(format!(r"\\{unc_body}"));
    }

    Err(policy_error("unsupported extended path prefix"))
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

fn is_other_users_profile_path(path: &Path) -> Result<bool> {
    let Some(user_profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
        return Ok(false);
    };
    let Some(users_root) = user_profile.parent() else {
        return Ok(false);
    };

    let path = normalize_for_policy(path)?;
    let users_root = normalize_for_policy(users_root)?;
    let user_profile = normalize_for_policy(&user_profile)?;

    Ok(path_has_component_prefix(&path, &users_root)
        && !path_has_component_prefix(&path, &user_profile)
        && path != users_root)
}

fn component_has_unsafe_trailing_chars(component: &str) -> bool {
    component.ends_with(' ') || component.ends_with('.')
}

fn trim_trailing_separators(path: &str) -> &str {
    path.trim_end_matches(['\\', '/'])
}

fn should_canonicalize_full_path(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => !metadata.file_type().is_symlink(),
        Err(_) => false,
    }
}

fn policy_error(message: impl Into<String>) -> WinSymlinksError {
    WinSymlinksError::new(ErrorCode::PathNormalizationFailed, message)
}

fn entries_from_paths(
    paths: impl IntoIterator<Item = PathBuf>,
    source: BlacklistSource,
) -> Vec<BlacklistEntry> {
    let mut paths: Vec<PathBuf> = paths.into_iter().collect();
    paths.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
    paths.dedup_by(|left, right| {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    });
    paths
        .into_iter()
        .map(|path| BlacklistEntry {
            path,
            source: source.clone(),
        })
        .collect()
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

    #[test]
    fn canonicalizes_supported_extended_drive_paths() {
        assert_eq!(
            normalize_for_policy(Path::new(r"\\?\C:\work\.\link")).unwrap(),
            PathBuf::from(r"C:\work\link")
        );
    }

    #[test]
    fn canonicalizes_supported_extended_unc_paths() {
        assert_eq!(
            normalize_for_policy(Path::new(r"\\?\UNC\server\share\dir\..\link")).unwrap(),
            PathBuf::from(r"\\server\share\link")
        );
    }

    #[test]
    fn rejects_suspicious_native_paths() {
        for path in [
            r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy1\Windows",
            r"\\.\C:\Windows",
            r"\??\C:\Windows",
            r"\Device\HarddiskVolume1\Windows",
        ] {
            let err = normalize_for_policy(Path::new(path)).unwrap_err();
            assert_eq!(err.code(), ErrorCode::PathNormalizationFailed);
        }
    }

    #[test]
    fn rejects_root_relative_paths() {
        let err = normalize_for_policy(Path::new(r"\Windows\System32")).unwrap_err();

        assert_eq!(err.code(), ErrorCode::PathNormalizationFailed);
    }

    #[test]
    fn rejects_trailing_space_and_dot_components() {
        for path in [r"C:\work\link. ", r"C:\work\link."] {
            let err = normalize_for_policy(Path::new(path)).unwrap_err();
            assert_eq!(err.code(), ErrorCode::PathNormalizationFailed);
        }
    }

    #[test]
    fn does_not_canonicalize_missing_paths() {
        assert!(!should_canonicalize_full_path(Path::new(
            r"C:\path\that\should\not\exist\link"
        )));
    }

    #[test]
    fn blocks_volume_roots() {
        let blacklist = built_in_source_blacklist();
        let blocked = blacklist.is_blocked(Path::new(r"C:\")).unwrap().unwrap();

        assert_eq!(blocked.source, BlacklistSource::BuiltIn);
        assert_eq!(blocked.path, PathBuf::from(r"C:\"));
    }

    #[test]
    fn blocks_unc_admin_shares_and_children() {
        let blacklist = built_in_source_blacklist();

        assert!(blacklist
            .is_blocked(Path::new(r"\\server\C$\Windows"))
            .unwrap()
            .is_some());
    }

    #[test]
    fn merges_user_blacklist_without_replacing_built_ins() {
        let blacklist = merge_source_blacklist(&[PathBuf::from(r"D:\SensitiveServiceData")]);

        assert!(blacklist
            .entries()
            .iter()
            .any(|entry| entry.source == BlacklistSource::BuiltIn
                && path_has_component_prefix(Path::new(r"C:\Windows"), &entry.path)));
        assert!(blacklist
            .entries()
            .iter()
            .any(|entry| entry.source == BlacklistSource::User
                && entry.path == std::path::Path::new(r"D:\SensitiveServiceData")));
        assert!(blacklist
            .is_blocked(Path::new(r"D:\SensitiveServiceData\child"))
            .unwrap()
            .is_some());
    }
}
