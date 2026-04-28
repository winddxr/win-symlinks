pub const SERVICE_NAME: &str = "WinSymlinksBroker";
pub const SERVICE_DISPLAY_NAME: &str = "Win Symlinks Broker";

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    NotInstalled,
    Stopped,
    StartPending,
    StopPending,
    Running,
    ContinuePending,
    PausePending,
    Paused,
    Unknown,
}

impl fmt::Display for ServiceState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = match self {
            ServiceState::NotInstalled => "not-installed",
            ServiceState::Stopped => "stopped",
            ServiceState::StartPending => "start-pending",
            ServiceState::StopPending => "stop-pending",
            ServiceState::Running => "running",
            ServiceState::ContinuePending => "continue-pending",
            ServiceState::PausePending => "pause-pending",
            ServiceState::Paused => "paused",
            ServiceState::Unknown => "unknown",
        };
        f.write_str(state)
    }
}

pub fn install_service() -> crate::Result<()> {
    platform::install_service()
}

pub fn uninstall_service() -> crate::Result<()> {
    platform::uninstall_service()
}

pub fn start_service() -> crate::Result<()> {
    platform::start_service()
}

pub fn stop_service() -> crate::Result<()> {
    platform::stop_service()
}

pub fn query_service_state() -> crate::Result<ServiceState> {
    platform::query_service_state()
}

pub fn run_broker_service() -> crate::Result<()> {
    platform::run_broker_service()
}

#[cfg(windows)]
mod platform {
    use super::{ServiceState, SERVICE_DISPLAY_NAME, SERVICE_NAME};
    use crate::{ErrorCode, Result, WinSymlinksError};
    use std::ffi::{OsStr, OsString};
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use windows_service::define_windows_service;
    use windows_service::service::{
        ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
        ServiceInfo, ServiceStartType, ServiceStatus, ServiceType,
    };
    use windows_service::service_control_handler::{
        self, ServiceControlHandlerResult, ServiceStatusHandle,
    };
    use windows_service::service_dispatcher;
    use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
    use windows_service::Error as WindowsServiceError;

    const ERROR_ACCESS_DENIED: i32 = 5;
    const ERROR_SERVICE_DOES_NOT_EXIST: i32 = 1060;
    const ERROR_SERVICE_ALREADY_RUNNING: i32 = 1056;
    const ERROR_SERVICE_NOT_ACTIVE: i32 = 1062;
    const ERROR_SERVICE_EXISTS: i32 = 1073;
    const SERVICE_WAIT_TIMEOUT: Duration = Duration::from_secs(30);
    const SERVICE_POLL_INTERVAL: Duration = Duration::from_millis(250);

    define_windows_service!(ffi_service_main, broker_service_main);

    pub fn install_service() -> Result<()> {
        let broker_exe = broker_executable_path()?;
        if !broker_exe.is_file() {
            return Err(WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!(
                    "broker executable was not found at {}; build win-symlinks-broker.exe first",
                    broker_exe.display()
                ),
            ));
        }

        let manager =
            service_manager(ServiceManagerAccess::CREATE_SERVICE, "open service manager")?;
        let service_info = ServiceInfo {
            name: OsString::from(SERVICE_NAME),
            display_name: OsString::from(SERVICE_DISPLAY_NAME),
            service_type: ServiceType::OWN_PROCESS,
            start_type: ServiceStartType::AutoStart,
            error_control: ServiceErrorControl::Normal,
            executable_path: broker_exe,
            launch_arguments: Vec::new(),
            dependencies: Vec::new(),
            account_name: None,
            account_password: None,
        };
        let service = manager
            .create_service(
                &service_info,
                ServiceAccess::CHANGE_CONFIG | ServiceAccess::QUERY_STATUS,
            )
            .map_err(|err| map_service_error("install service", err))?;
        service
            .set_delayed_auto_start(true)
            .map_err(|err| map_service_error("configure delayed automatic start", err))?;

        Ok(())
    }

    pub fn uninstall_service() -> Result<()> {
        let manager = service_manager(ServiceManagerAccess::CONNECT, "open service manager")?;
        let service = open_service(
            &manager,
            ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS | ServiceAccess::STOP,
            "open service for uninstall",
        )?;

        stop_service_handle(&service)?;
        service
            .delete()
            .map_err(|err| map_service_error("uninstall service", err))?;
        Ok(())
    }

    pub fn start_service() -> Result<()> {
        let manager = service_manager(ServiceManagerAccess::CONNECT, "open service manager")?;
        let service = open_service(
            &manager,
            ServiceAccess::START | ServiceAccess::QUERY_STATUS,
            "open service for start",
        )?;

        match query_windows_state(&service)? {
            windows_service::service::ServiceState::Running => return Ok(()),
            windows_service::service::ServiceState::StartPending => {
                wait_for_service_state(
                    &service,
                    windows_service::service::ServiceState::Running,
                    "start service",
                )?;
                return Ok(());
            }
            _ => {}
        }

        service
            .start::<&OsStr>(&[])
            .map_err(|err| map_service_error("start service", err))?;
        wait_for_service_state(
            &service,
            windows_service::service::ServiceState::Running,
            "start service",
        )
    }

    pub fn stop_service() -> Result<()> {
        let manager = service_manager(ServiceManagerAccess::CONNECT, "open service manager")?;
        let service = open_service(
            &manager,
            ServiceAccess::STOP | ServiceAccess::QUERY_STATUS,
            "open service for stop",
        )?;

        stop_service_handle(&service)
    }

    pub fn query_service_state() -> Result<ServiceState> {
        let manager = service_manager(ServiceManagerAccess::CONNECT, "open service manager")?;
        match manager.open_service(SERVICE_NAME, ServiceAccess::QUERY_STATUS) {
            Ok(service) => Ok(map_service_state(query_windows_state(&service)?)),
            Err(err) if service_error_code(&err) == Some(ERROR_SERVICE_DOES_NOT_EXIST) => {
                Ok(ServiceState::NotInstalled)
            }
            Err(err) => Err(map_service_error("query service status", err)),
        }
    }

    pub fn run_broker_service() -> Result<()> {
        service_dispatcher::start(SERVICE_NAME, ffi_service_main)
            .map_err(|err| map_service_error("run broker service dispatcher", err))
    }

    fn broker_service_main(_arguments: Vec<OsString>) {
        if let Err(err) = run_broker_service_loop() {
            tracing::error!(%err, "broker service exited with error");
        }
    }

    fn run_broker_service_loop() -> windows_service::Result<()> {
        let (stop_tx, stop_rx) = mpsc::channel();
        let event_handler = move |control_event| -> ServiceControlHandlerResult {
            match control_event {
                ServiceControl::Stop => {
                    let _ = stop_tx.send(());
                    ServiceControlHandlerResult::NoError
                }
                ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
                _ => ServiceControlHandlerResult::NotImplemented,
            }
        };

        let status_handle = service_control_handler::register(SERVICE_NAME, event_handler)?;
        set_service_status(
            &status_handle,
            windows_service::service::ServiceState::Running,
            ServiceControlAccept::STOP,
        )?;

        let _ = stop_rx.recv();

        set_service_status(
            &status_handle,
            windows_service::service::ServiceState::StopPending,
            ServiceControlAccept::empty(),
        )?;
        set_service_status(
            &status_handle,
            windows_service::service::ServiceState::Stopped,
            ServiceControlAccept::empty(),
        )
    }

    fn set_service_status(
        status_handle: &ServiceStatusHandle,
        state: windows_service::service::ServiceState,
        accepted: ServiceControlAccept,
    ) -> windows_service::Result<()> {
        status_handle.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accepted,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::default(),
            process_id: None,
        })
    }

    fn stop_service_handle(service: &windows_service::service::Service) -> Result<()> {
        match query_windows_state(service)? {
            windows_service::service::ServiceState::Stopped => return Ok(()),
            windows_service::service::ServiceState::StopPending => {
                return wait_for_service_state(
                    service,
                    windows_service::service::ServiceState::Stopped,
                    "stop service",
                )
            }
            _ => {}
        }

        match service.stop() {
            Ok(_) => {}
            Err(err) if service_error_code(&err) == Some(ERROR_SERVICE_NOT_ACTIVE) => {
                return Ok(());
            }
            Err(err) => return Err(map_service_error("stop service", err)),
        }
        wait_for_service_state(
            service,
            windows_service::service::ServiceState::Stopped,
            "stop service",
        )
    }

    fn wait_for_service_state(
        service: &windows_service::service::Service,
        desired_state: windows_service::service::ServiceState,
        action: &'static str,
    ) -> Result<()> {
        let deadline = Instant::now() + SERVICE_WAIT_TIMEOUT;
        while Instant::now() < deadline {
            if query_windows_state(service)? == desired_state {
                return Ok(());
            }
            std::thread::sleep(SERVICE_POLL_INTERVAL);
        }

        Err(WinSymlinksError::new(
            ErrorCode::ServiceUnavailable,
            format!("timed out while waiting to {action}"),
        ))
    }

    fn query_windows_state(
        service: &windows_service::service::Service,
    ) -> Result<windows_service::service::ServiceState> {
        Ok(service
            .query_status()
            .map_err(|err| map_service_error("query service status", err))?
            .current_state)
    }

    fn open_service(
        manager: &ServiceManager,
        access: ServiceAccess,
        action: &'static str,
    ) -> Result<windows_service::service::Service> {
        manager
            .open_service(SERVICE_NAME, access)
            .map_err(|err| map_service_error(action, err))
    }

    fn service_manager(
        access: ServiceManagerAccess,
        action: &'static str,
    ) -> Result<ServiceManager> {
        ServiceManager::local_computer(None::<&str>, access)
            .map_err(|err| map_service_error(action, err))
    }

    fn broker_executable_path() -> Result<PathBuf> {
        let current_exe = std::env::current_exe().map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!("failed to resolve current executable path: {err}"),
            )
        })?;
        let Some(directory) = current_exe.parent() else {
            return Err(WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!(
                    "failed to resolve executable directory from {}",
                    current_exe.display()
                ),
            ));
        };

        Ok(directory.join("win-symlinks-broker.exe"))
    }

    fn map_service_state(state: windows_service::service::ServiceState) -> ServiceState {
        match state {
            windows_service::service::ServiceState::Stopped => ServiceState::Stopped,
            windows_service::service::ServiceState::StartPending => ServiceState::StartPending,
            windows_service::service::ServiceState::StopPending => ServiceState::StopPending,
            windows_service::service::ServiceState::Running => ServiceState::Running,
            windows_service::service::ServiceState::ContinuePending => {
                ServiceState::ContinuePending
            }
            windows_service::service::ServiceState::PausePending => ServiceState::PausePending,
            windows_service::service::ServiceState::Paused => ServiceState::Paused,
        }
    }

    fn map_service_error(action: &'static str, err: WindowsServiceError) -> WinSymlinksError {
        match service_error_code(&err) {
            Some(ERROR_ACCESS_DENIED) => WinSymlinksError::new(
                ErrorCode::PrivilegeRequired,
                format!(
                    "administrator privileges are required to {action}: {}",
                    service_error_detail(&err)
                ),
            ),
            Some(ERROR_SERVICE_DOES_NOT_EXIST) => WinSymlinksError::new(
                ErrorCode::ServiceNotInstalled,
                format!("{SERVICE_NAME} is not installed"),
            ),
            Some(ERROR_SERVICE_ALREADY_RUNNING) => WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!("{SERVICE_NAME} is already running"),
            ),
            Some(ERROR_SERVICE_NOT_ACTIVE) => WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!("{SERVICE_NAME} is not running"),
            ),
            Some(ERROR_SERVICE_EXISTS) => WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!("{SERVICE_NAME} is already installed"),
            ),
            _ => WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!("failed to {action}: {}", service_error_detail(&err)),
            ),
        }
    }

    fn service_error_code(err: &WindowsServiceError) -> Option<i32> {
        match err {
            WindowsServiceError::Winapi(io_error) => io_error.raw_os_error(),
            _ => None,
        }
    }

    fn service_error_detail(err: &WindowsServiceError) -> String {
        match err {
            WindowsServiceError::Winapi(io_error) => io_error.to_string(),
            _ => err.to_string(),
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::ServiceState;
    use crate::{ErrorCode, Result, WinSymlinksError};

    pub fn install_service() -> Result<()> {
        Err(unavailable("service install is only supported on Windows"))
    }

    pub fn uninstall_service() -> Result<()> {
        Err(unavailable(
            "service uninstall is only supported on Windows",
        ))
    }

    pub fn start_service() -> Result<()> {
        Err(unavailable("service start is only supported on Windows"))
    }

    pub fn stop_service() -> Result<()> {
        Err(unavailable("service stop is only supported on Windows"))
    }

    pub fn query_service_state() -> Result<ServiceState> {
        Ok(ServiceState::NotInstalled)
    }

    pub fn run_broker_service() -> Result<()> {
        Err(unavailable(
            "broker service host is only supported on Windows",
        ))
    }

    fn unavailable(message: &'static str) -> WinSymlinksError {
        WinSymlinksError::new(ErrorCode::ServiceUnavailable, message)
    }
}
