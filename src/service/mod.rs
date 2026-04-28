pub const SERVICE_NAME: &str = "WinSymlinksBroker";
pub const SERVICE_DISPLAY_NAME: &str = "Win Symlinks Broker";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    NotInstalled,
    Stopped,
    Running,
    Unknown,
}

pub fn query_service_state() -> crate::Result<ServiceState> {
    Err(crate::WinSymlinksError::new(
        crate::ErrorCode::ServiceNotInstalled,
        "service management is not implemented yet",
    ))
}

pub fn run_broker_service() -> crate::Result<()> {
    Err(crate::WinSymlinksError::new(
        crate::ErrorCode::ServiceUnavailable,
        "broker service host is not implemented yet",
    ))
}
