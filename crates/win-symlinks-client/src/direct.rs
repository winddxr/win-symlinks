use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};

#[cfg(windows)]
use windows::{
    core::PCWSTR,
    Win32::{
        Foundation::{
            GetLastError, ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, ERROR_PRIVILEGE_NOT_HELD,
            WIN32_ERROR,
        },
        Storage::FileSystem::{
            CreateSymbolicLinkW, SYMBOLIC_LINK_FLAGS, SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE,
            SYMBOLIC_LINK_FLAG_DIRECTORY,
        },
    },
};

#[cfg(windows)]
const FILE_ATTRIBUTE_REPARSE_POINT_BITS: u32 = 0x400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    File,
    #[serde(rename = "directory")]
    Dir,
}

impl TargetKind {
    pub fn as_protocol_value(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Dir => "directory",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectCreateOptions {
    pub link_path: PathBuf,
    pub target_path: PathBuf,
    pub target_kind: Option<TargetKind>,
    pub replace_existing_symlink: bool,
    pub allow_unprivileged_direct_create: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkPathState {
    Missing,
    SymbolicLink,
    File,
    Directory,
    OtherReparsePoint,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplacementPlan {
    Create,
    ReplaceExistingSymlink,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectCreateOutcome {
    Created,
    NeedsBroker,
}

pub fn decide_target_kind(
    target_path: impl AsRef<Path>,
    hint: Option<TargetKind>,
) -> crate::Result<TargetKind> {
    match fs::metadata(target_path.as_ref()) {
        Ok(metadata) => {
            let actual = if metadata.is_dir() {
                TargetKind::Dir
            } else {
                TargetKind::File
            };

            if hint.is_some_and(|hint| hint != actual) {
                return Err(crate::WinSymlinksError::new(
                    crate::ErrorCode::TargetKindConflict,
                    "target kind hint conflicts with the existing target",
                ));
            }

            Ok(actual)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => hint.ok_or_else(|| {
            crate::WinSymlinksError::new(
                crate::ErrorCode::TargetKindRequired,
                "target does not exist; pass --win-kind=file or --win-kind=dir",
            )
        }),
        Err(err) => Err(crate::WinSymlinksError::new(
            crate::ErrorCode::CreateSymlinkFailed,
            format!("failed to inspect target path: {err}"),
        )),
    }
}

pub fn plan_replacement(
    link_state: LinkPathState,
    replace_existing_symlink: bool,
) -> crate::Result<ReplacementPlan> {
    match (link_state, replace_existing_symlink) {
        (LinkPathState::Missing, _) => Ok(ReplacementPlan::Create),
        (LinkPathState::SymbolicLink, true) => Ok(ReplacementPlan::ReplaceExistingSymlink),
        (LinkPathState::SymbolicLink, false)
        | (LinkPathState::File, false)
        | (LinkPathState::Directory, false)
        | (LinkPathState::OtherReparsePoint, false)
        | (LinkPathState::Other, false) => Err(crate::WinSymlinksError::new(
            crate::ErrorCode::LinkAlreadyExists,
            "link path already exists",
        )),
        (LinkPathState::File, true) | (LinkPathState::Directory, true) => {
            Err(crate::WinSymlinksError::new(
                crate::ErrorCode::LinkPathIsNotSymlink,
                "refusing to replace an existing non-symlink filesystem object",
            ))
        }
        (LinkPathState::OtherReparsePoint, true) => Err(crate::WinSymlinksError::new(
            crate::ErrorCode::UnsafeReparsePoint,
            "refusing to replace an existing non-symlink reparse point",
        )),
        (LinkPathState::Other, true) => Err(crate::WinSymlinksError::new(
            crate::ErrorCode::LinkPathIsNotSymlink,
            "refusing to replace an existing non-symlink filesystem object",
        )),
    }
}

pub fn inspect_link_path_state(path: impl AsRef<Path>) -> crate::Result<LinkPathState> {
    let metadata = match fs::symlink_metadata(path.as_ref()) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(LinkPathState::Missing),
        Err(err) => {
            return Err(crate::WinSymlinksError::new(
                crate::ErrorCode::CreateSymlinkFailed,
                format!("failed to inspect link path: {err}"),
            ));
        }
    };

    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Ok(LinkPathState::SymbolicLink);
    }

    #[cfg(windows)]
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT_BITS != 0 {
        return Ok(LinkPathState::OtherReparsePoint);
    }

    if metadata.is_dir() {
        Ok(LinkPathState::Directory)
    } else if metadata.is_file() {
        Ok(LinkPathState::File)
    } else {
        Ok(LinkPathState::Other)
    }
}

pub fn try_direct_create(options: &DirectCreateOptions) -> crate::Result<DirectCreateOutcome> {
    let link_state = inspect_link_path_state(&options.link_path)?;
    let replacement_plan = plan_replacement(link_state, options.replace_existing_symlink)?;

    if replacement_plan == ReplacementPlan::ReplaceExistingSymlink {
        return Ok(DirectCreateOutcome::NeedsBroker);
    }

    let target_kind = decide_target_kind(&options.target_path, options.target_kind)?;

    match create_symbolic_link(
        &options.link_path,
        &options.target_path,
        target_kind,
        options.allow_unprivileged_direct_create,
    ) {
        Ok(()) => Ok(DirectCreateOutcome::Created),
        Err(err) if err.code() == crate::ErrorCode::PrivilegeRequired => {
            Ok(DirectCreateOutcome::NeedsBroker)
        }
        Err(err) => Err(err),
    }
}

pub fn create_symbolic_link(
    link_path: &Path,
    target_path: &Path,
    target_kind: TargetKind,
    allow_unprivileged_direct_create: bool,
) -> crate::Result<()> {
    create_symbolic_link_platform(
        link_path,
        target_path,
        target_kind,
        allow_unprivileged_direct_create,
    )
}

#[cfg(not(windows))]
fn create_symbolic_link_platform(
    _link_path: &Path,
    _target_path: &Path,
    _target_kind: TargetKind,
    _allow_unprivileged_direct_create: bool,
) -> crate::Result<()> {
    Err(crate::WinSymlinksError::new(
        crate::ErrorCode::CreateSymlinkFailed,
        "Windows symbolic link creation is only available on Windows",
    ))
}

#[cfg(windows)]
fn create_symbolic_link_platform(
    link_path: &Path,
    target_path: &Path,
    target_kind: TargetKind,
    allow_unprivileged_direct_create: bool,
) -> crate::Result<()> {
    let link_path = path_to_wide_null(link_path);
    let target_path = path_to_wide_null(target_path);
    let flags = symbolic_link_flags(target_kind, allow_unprivileged_direct_create);

    let created = unsafe {
        CreateSymbolicLinkW(
            PCWSTR(link_path.as_ptr()),
            PCWSTR(target_path.as_ptr()),
            flags,
        )
    };

    if created {
        Ok(())
    } else {
        let error = unsafe { GetLastError() };
        Err(map_create_symbolic_link_error(error))
    }
}

#[cfg(windows)]
fn symbolic_link_flags(
    target_kind: TargetKind,
    allow_unprivileged_direct_create: bool,
) -> SYMBOLIC_LINK_FLAGS {
    let mut flags = SYMBOLIC_LINK_FLAGS(0);
    if target_kind == TargetKind::Dir {
        flags |= SYMBOLIC_LINK_FLAG_DIRECTORY;
    }
    if allow_unprivileged_direct_create {
        flags |= SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE;
    }
    flags
}

#[cfg(windows)]
fn path_to_wide_null(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn map_create_symbolic_link_error(error: WIN32_ERROR) -> crate::WinSymlinksError {
    let (code, message) = if error == ERROR_PRIVILEGE_NOT_HELD {
        (
            crate::ErrorCode::PrivilegeRequired,
            "symbolic link privilege is not held by this process".to_string(),
        )
    } else if error == ERROR_ALREADY_EXISTS || error == ERROR_FILE_EXISTS {
        (
            crate::ErrorCode::LinkAlreadyExists,
            "link path already exists".to_string(),
        )
    } else {
        (
            crate::ErrorCode::CreateSymlinkFailed,
            format!("CreateSymbolicLinkW failed with Windows error {}", error.0),
        )
    };

    crate::WinSymlinksError::new(code, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorCode;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("win-symlinks-{name}-{unique}"))
    }

    #[test]
    fn decides_existing_file_target_kind_from_filesystem() {
        let file = temp_path("target-file");
        fs::write(&file, b"target").unwrap();

        let kind = decide_target_kind(&file, None).unwrap();

        assert_eq!(kind, TargetKind::File);
        fs::remove_file(file).unwrap();
    }

    #[test]
    fn decides_existing_directory_target_kind_from_filesystem() {
        let dir = temp_path("target-dir");
        fs::create_dir(&dir).unwrap();

        let kind = decide_target_kind(&dir, None).unwrap();

        assert_eq!(kind, TargetKind::Dir);
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn uses_hint_for_missing_target() {
        let missing = temp_path("missing-target");

        assert_eq!(
            decide_target_kind(&missing, Some(TargetKind::Dir)).unwrap(),
            TargetKind::Dir
        );
    }

    #[test]
    fn requires_hint_for_missing_target() {
        let missing = temp_path("missing-target");
        let err = decide_target_kind(&missing, None).unwrap_err();

        assert_eq!(err.code(), ErrorCode::TargetKindRequired);
    }

    #[test]
    fn rejects_conflicting_target_kind_hint() {
        let file = temp_path("target-file-conflict");
        fs::write(&file, b"target").unwrap();

        let err = decide_target_kind(&file, Some(TargetKind::Dir)).unwrap_err();

        assert_eq!(err.code(), ErrorCode::TargetKindConflict);
        fs::remove_file(file).unwrap();
    }

    #[test]
    fn replacement_plan_allows_missing_link_path() {
        assert_eq!(
            plan_replacement(LinkPathState::Missing, false).unwrap(),
            ReplacementPlan::Create
        );
    }

    #[test]
    fn replacement_plan_allows_existing_symlink_only_with_force() {
        assert_eq!(
            plan_replacement(LinkPathState::SymbolicLink, true).unwrap(),
            ReplacementPlan::ReplaceExistingSymlink
        );

        let err = plan_replacement(LinkPathState::SymbolicLink, false).unwrap_err();
        assert_eq!(err.code(), ErrorCode::LinkAlreadyExists);
    }

    #[test]
    fn replacement_plan_rejects_real_files_and_directories_even_with_force() {
        let file_err = plan_replacement(LinkPathState::File, true).unwrap_err();
        let dir_err = plan_replacement(LinkPathState::Directory, true).unwrap_err();

        assert_eq!(file_err.code(), ErrorCode::LinkPathIsNotSymlink);
        assert_eq!(dir_err.code(), ErrorCode::LinkPathIsNotSymlink);
    }

    #[test]
    fn replacement_plan_rejects_non_symlink_reparse_points() {
        let err = plan_replacement(LinkPathState::OtherReparsePoint, true).unwrap_err();

        assert_eq!(err.code(), ErrorCode::UnsafeReparsePoint);
    }

    #[test]
    fn direct_create_reports_existing_link_path_before_missing_target_kind() {
        let link = temp_path("existing-link-path");
        let missing_target = temp_path("missing-target-for-existing-link");
        fs::write(&link, b"real file").unwrap();
        let options = DirectCreateOptions {
            link_path: link.clone(),
            target_path: missing_target,
            target_kind: None,
            replace_existing_symlink: false,
            allow_unprivileged_direct_create: true,
        };

        let err = try_direct_create(&options).unwrap_err();

        assert_eq!(err.code(), ErrorCode::LinkAlreadyExists);
        fs::remove_file(link).unwrap();
    }

    #[test]
    fn inspects_missing_file_and_directory_link_states() {
        let missing = temp_path("missing-link-state");
        let file = temp_path("file-link-state");
        let dir = temp_path("dir-link-state");
        fs::write(&file, b"link").unwrap();
        fs::create_dir(&dir).unwrap();

        assert_eq!(
            inspect_link_path_state(&missing).unwrap(),
            LinkPathState::Missing
        );
        assert_eq!(inspect_link_path_state(&file).unwrap(), LinkPathState::File);
        assert_eq!(
            inspect_link_path_state(&dir).unwrap(),
            LinkPathState::Directory
        );

        fs::remove_file(file).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn symbolic_link_flags_include_only_requested_bits() {
        let file_direct = symbolic_link_flags(TargetKind::File, true);
        let dir_privileged = symbolic_link_flags(TargetKind::Dir, false);

        assert!(file_direct.contains(SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE));
        assert!(!file_direct.contains(SYMBOLIC_LINK_FLAG_DIRECTORY));
        assert!(dir_privileged.contains(SYMBOLIC_LINK_FLAG_DIRECTORY));
        assert!(!dir_privileged.contains(SYMBOLIC_LINK_FLAG_ALLOW_UNPRIVILEGED_CREATE));
    }

    #[cfg(windows)]
    #[test]
    fn maps_privilege_failure_separately_from_create_failures() {
        let privilege_error = map_create_symbolic_link_error(ERROR_PRIVILEGE_NOT_HELD);
        let exists_error = map_create_symbolic_link_error(ERROR_ALREADY_EXISTS);
        let other_error = map_create_symbolic_link_error(WIN32_ERROR(3));

        assert_eq!(privilege_error.code(), ErrorCode::PrivilegeRequired);
        assert_eq!(exists_error.code(), ErrorCode::LinkAlreadyExists);
        assert_eq!(other_error.code(), ErrorCode::CreateSymlinkFailed);
    }
}
