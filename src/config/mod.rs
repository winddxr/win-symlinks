use serde::{Deserialize, Serialize};
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
