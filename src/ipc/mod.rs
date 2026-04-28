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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorCode;

    fn request_id() -> Uuid {
        Uuid::parse_str("018f5b2a-7f3a-7b7a-9c21-000000000001").unwrap()
    }

    #[test]
    fn create_symlink_request_round_trips_with_documented_schema() {
        let request = CreateSymlinkRequest {
            version: PROTOCOL_VERSION,
            request_id: request_id(),
            operation: Operation::CreateSymlink,
            link_path: PathBuf::from(r"F:\work\project\node_modules\pkg"),
            target_path: PathBuf::from(r"..\shared\pkg"),
            target_kind: Some(TargetKind::Dir),
            replace_existing_symlink: false,
        };

        let json = serde_json::to_value(&request).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "version": 1,
                "request_id": "018f5b2a-7f3a-7b7a-9c21-000000000001",
                "operation": "create_symlink",
                "link_path": r"F:\work\project\node_modules\pkg",
                "target_path": r"..\shared\pkg",
                "target_kind": "directory",
                "replace_existing_symlink": false
            })
        );
        assert_eq!(
            serde_json::from_value::<CreateSymlinkRequest>(json).unwrap(),
            request
        );
    }

    #[test]
    fn broker_success_response_round_trips() {
        let response = BrokerResponse::ok(request_id());
        let json = serde_json::to_value(&response).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "request_id": "018f5b2a-7f3a-7b7a-9c21-000000000001",
                "ok": true,
                "error_code": null,
                "message": null
            })
        );
        assert_eq!(
            serde_json::from_value::<BrokerResponse>(json).unwrap(),
            response
        );
    }

    #[test]
    fn broker_error_response_round_trips() {
        let response = BrokerResponse::error(
            request_id(),
            ErrorCode::SourceBlacklisted,
            r"link path is blocked by source blacklist: C:\Windows",
        );
        let json = serde_json::to_value(&response).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "request_id": "018f5b2a-7f3a-7b7a-9c21-000000000001",
                "ok": false,
                "error_code": "SOURCE_BLACKLISTED",
                "message": r"link path is blocked by source blacklist: C:\Windows"
            })
        );
        assert_eq!(
            serde_json::from_value::<BrokerResponse>(json).unwrap(),
            response
        );
    }

    #[test]
    fn request_schema_rejects_unknown_fields() {
        let json = serde_json::json!({
            "version": 1,
            "request_id": "018f5b2a-7f3a-7b7a-9c21-000000000001",
            "operation": "create_symlink",
            "link_path": "link",
            "target_path": "target",
            "target_kind": "file",
            "replace_existing_symlink": false,
            "unexpected": true
        });

        assert!(serde_json::from_value::<CreateSymlinkRequest>(json).is_err());
    }
}
