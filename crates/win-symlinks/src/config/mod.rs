use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConfig {
    pub additional_source_blacklist: Vec<PathBuf>,
    pub allow_direct_create_attempt: bool,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            additional_source_blacklist: Vec::new(),
            allow_direct_create_attempt: true,
        }
    }
}

pub fn default_config_path() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("win-symlinks")
        .join("config.json")
}

pub fn load_config() -> crate::Result<AppConfig> {
    let path = default_config_path();
    match fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw).map_err(|err| {
            crate::WinSymlinksError::new(
                crate::ErrorCode::ServiceUnavailable,
                format!("failed to parse configuration at {}: {err}", path.display()),
            )
        }),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(err) => Err(crate::WinSymlinksError::new(
            crate::ErrorCode::ServiceUnavailable,
            format!("failed to read configuration at {}: {err}", path.display()),
        )),
    }
}
