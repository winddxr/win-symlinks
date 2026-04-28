use serde::{Deserialize, Serialize};
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
pub enum DirectCreateOutcome {
    Created,
    NeedsBroker,
}

pub fn try_direct_create(_options: &CreateSymlinkOptions) -> crate::Result<DirectCreateOutcome> {
    Err(crate::WinSymlinksError::new(
        crate::ErrorCode::CreateSymlinkFailed,
        "direct symbolic link creation is not implemented yet",
    ))
}
