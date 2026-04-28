use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum TargetKind {
    File,
    #[serde(rename = "directory")]
    #[value(name = "dir")]
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
pub struct CreateSymlinkOptions {
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
    target_path: impl AsRef<std::path::Path>,
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

pub fn try_direct_create(_options: &CreateSymlinkOptions) -> crate::Result<DirectCreateOutcome> {
    Err(crate::WinSymlinksError::new(
        crate::ErrorCode::CreateSymlinkFailed,
        "direct symbolic link creation is not implemented yet",
    ))
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
}
