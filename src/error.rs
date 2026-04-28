use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    UnsupportedMode,
    ServiceNotInstalled,
    ServiceUnavailable,
    PrivilegeRequired,
    SourceBlacklisted,
    TargetKindRequired,
    LinkAlreadyExists,
    LinkPathIsNotSymlink,
    UnsafeReparsePoint,
    CreateSymlinkFailed,
    PathNormalizationFailed,
    ServiceIdentityMismatch,
    CallerParentWriteDenied,
    TargetKindConflict,
    RemoteClientRejected,
    ReplacementPartiallyCompleted,
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let json = serde_json::to_string(self).map_err(|_| fmt::Error)?;
        f.write_str(json.trim_matches('"'))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WinSymlinksError {
    code: ErrorCode,
    message: String,
}

impl WinSymlinksError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for WinSymlinksError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for WinSymlinksError {}

pub type Result<T> = std::result::Result<T, WinSymlinksError>;
