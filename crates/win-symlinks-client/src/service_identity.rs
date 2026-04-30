use crate::Result;

pub const SERVICE_NAME: &str = "WinSymlinksBroker";

pub fn query_service_process_id() -> Result<Option<u32>> {
    platform::query_service_process_id()
}

#[cfg(windows)]
mod platform {
    use super::SERVICE_NAME;
    use crate::{ErrorCode, Result, WinSymlinksError};
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{GetLastError, ERROR_SERVICE_DOES_NOT_EXIST},
            System::Services::{
                CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx,
                SC_MANAGER_CONNECT, SC_STATUS_PROCESS_INFO, SERVICE_QUERY_STATUS,
                SERVICE_STATUS_PROCESS,
            },
        },
    };

    pub fn query_service_process_id() -> Result<Option<u32>> {
        let manager = unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT) }
            .map_err(|err| {
                WinSymlinksError::new(
                    ErrorCode::ServiceUnavailable,
                    format!("failed to open service manager: {err}"),
                )
            })?;
        let manager = OwnedServiceHandle(manager);

        let service_name = wide_null(SERVICE_NAME);
        let service = unsafe {
            OpenServiceW(
                manager.raw(),
                PCWSTR(service_name.as_ptr()),
                SERVICE_QUERY_STATUS,
            )
        }
        .map_err(|err| {
            if unsafe { GetLastError() } == ERROR_SERVICE_DOES_NOT_EXIST {
                WinSymlinksError::new(
                    ErrorCode::ServiceNotInstalled,
                    format!("{SERVICE_NAME} is not installed"),
                )
            } else {
                WinSymlinksError::new(
                    ErrorCode::ServiceUnavailable,
                    format!("failed to open {SERVICE_NAME}: {err}"),
                )
            }
        })?;
        let service = OwnedServiceHandle(service);

        let mut status_buffer = vec![0u8; std::mem::size_of::<SERVICE_STATUS_PROCESS>()];
        let mut bytes_needed = 0;
        unsafe {
            QueryServiceStatusEx(
                service.raw(),
                SC_STATUS_PROCESS_INFO,
                Some(&mut status_buffer),
                &mut bytes_needed,
            )
        }
        .map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!("failed to query {SERVICE_NAME} process id: {err}"),
            )
        })?;

        let status = unsafe { &*(status_buffer.as_ptr() as *const SERVICE_STATUS_PROCESS) };
        if status.dwProcessId == 0 {
            Ok(None)
        } else {
            Ok(Some(status.dwProcessId))
        }
    }

    fn wide_null(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }

    struct OwnedServiceHandle(windows::Win32::System::Services::SC_HANDLE);

    impl OwnedServiceHandle {
        fn raw(&self) -> windows::Win32::System::Services::SC_HANDLE {
            self.0
        }
    }

    impl Drop for OwnedServiceHandle {
        fn drop(&mut self) {
            let _ = unsafe { CloseServiceHandle(self.0) };
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use crate::Result;

    pub fn query_service_process_id() -> Result<Option<u32>> {
        Ok(None)
    }
}
