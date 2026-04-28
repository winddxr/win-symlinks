use crate::symlink::TargetKind;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

pub const PROTOCOL_VERSION: u32 = 1;
pub const PIPE_NAME: &str = r"\\.\pipe\win-symlinks-broker";
pub const PIPE_CONNECT_TIMEOUT_MS: u64 = 3_000;
pub const REQUEST_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    CreateSymlink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSymlinkRequest {
    pub version: u32,
    pub request_id: Uuid,
    pub operation: Operation,
    pub link_path: PathBuf,
    pub target_path: PathBuf,
    pub target_kind: Option<TargetKind>,
    pub replace_existing_symlink: bool,
}

impl CreateSymlinkRequest {
    pub fn new(
        link_path: PathBuf,
        target_path: PathBuf,
        target_kind: Option<TargetKind>,
        replace_existing_symlink: bool,
    ) -> Self {
        Self {
            version: PROTOCOL_VERSION,
            request_id: Uuid::now_v7(),
            operation: Operation::CreateSymlink,
            link_path,
            target_path,
            target_kind,
            replace_existing_symlink,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokerResponse {
    pub request_id: Uuid,
    pub ok: bool,
    pub error_code: Option<crate::ErrorCode>,
    pub message: Option<String>,
}

impl BrokerResponse {
    pub fn ok(request_id: Uuid) -> Self {
        Self {
            request_id,
            ok: true,
            error_code: None,
            message: None,
        }
    }

    pub fn error(
        request_id: Uuid,
        error_code: crate::ErrorCode,
        message: impl Into<String>,
    ) -> Self {
        Self {
            request_id,
            ok: false,
            error_code: Some(error_code),
            message: Some(message.into()),
        }
    }
}
