use crate::protocol::CreateSymlinkRequest;

pub fn submit_create_symlink_request(request: CreateSymlinkRequest) -> crate::Result<()> {
    platform::submit_create_symlink_request(request)
}

pub fn check_broker_pipe() -> crate::Result<()> {
    platform::check_broker_pipe()
}

#[cfg(windows)]
mod platform {
    use crate::protocol::{
        response_to_result, BrokerResponse, CreateSymlinkRequest, PIPE_CONNECT_TIMEOUT_MS,
        PIPE_NAME, REQUEST_TIMEOUT_MS,
    };
    use crate::{service_identity, ErrorCode, Result, WinSymlinksError};
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::mpsc;
    use std::time::Duration;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{
                CloseHandle, GetLastError, ERROR_BROKEN_PIPE, ERROR_MORE_DATA, ERROR_NO_DATA,
                GENERIC_READ, GENERIC_WRITE, HANDLE,
            },
            Storage::FileSystem::{
                CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ,
                FILE_SHARE_WRITE, OPEN_EXISTING,
            },
            System::Pipes::{
                GetNamedPipeServerProcessId, SetNamedPipeHandleState, WaitNamedPipeW,
                PIPE_READMODE_MESSAGE,
            },
        },
    };

    const PIPE_BUFFER_SIZE: u32 = 64 * 1024;

    enum ClientEvent {
        Connected,
        Complete(Result<BrokerResponse>),
    }

    pub fn submit_create_symlink_request(request: CreateSymlinkRequest) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        let worker_request = request.clone();
        std::thread::spawn(move || {
            let response = connect_and_send_request(&worker_request, tx.clone());
            let _ = tx.send(ClientEvent::Complete(response));
        });

        match rx.recv_timeout(Duration::from_millis(PIPE_CONNECT_TIMEOUT_MS)) {
            Ok(ClientEvent::Connected) => {}
            Ok(ClientEvent::Complete(Ok(response))) => {
                return response_to_result(&request, response);
            }
            Ok(ClientEvent::Complete(Err(err))) => return Err(err),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(WinSymlinksError::new(
                    ErrorCode::ServiceUnavailable,
                    "timed out after 3 seconds while connecting to WinSymlinksBroker",
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(WinSymlinksError::new(
                    ErrorCode::ServiceUnavailable,
                    "broker client worker exited before connecting",
                ));
            }
        }

        match rx.recv_timeout(Duration::from_millis(REQUEST_TIMEOUT_MS)) {
            Ok(ClientEvent::Complete(Ok(response))) => response_to_result(&request, response),
            Ok(ClientEvent::Complete(Err(err))) => Err(err),
            Ok(ClientEvent::Connected) => Err(WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                "broker client received an unexpected duplicate connection event",
            )),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                "timed out after 30 seconds while waiting for WinSymlinksBroker response",
            )),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                "broker client worker exited before returning a response",
            )),
        }
    }

    pub fn check_broker_pipe() -> Result<()> {
        let _pipe = connect_verified_pipe()?;
        Ok(())
    }

    fn connect_and_send_request(
        request: &CreateSymlinkRequest,
        tx: mpsc::Sender<ClientEvent>,
    ) -> Result<BrokerResponse> {
        let pipe = connect_verified_pipe()?;
        let _ = tx.send(ClientEvent::Connected);
        write_message(
            pipe.raw(),
            &serde_json::to_vec(request).map_err(|err| {
                WinSymlinksError::new(
                    ErrorCode::ServiceUnavailable,
                    format!("failed to serialize broker request: {err}"),
                )
            })?,
        )?;
        let response_bytes = read_message(pipe.raw())?;
        serde_json::from_slice::<BrokerResponse>(&response_bytes).map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!("failed to parse broker response: {err}"),
            )
        })
    }

    fn connect_verified_pipe() -> Result<OwnedHandle> {
        let pipe_name = wide_null(PIPE_NAME);
        let available = unsafe {
            WaitNamedPipeW(PCWSTR(pipe_name.as_ptr()), PIPE_CONNECT_TIMEOUT_MS as u32).as_bool()
        };
        if !available {
            return Err(WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!(
                    "WinSymlinksBroker pipe was unavailable within 3 seconds: Windows error {}",
                    unsafe { GetLastError() }.0
                ),
            ));
        }

        let handle = unsafe {
            CreateFileW(
                PCWSTR(pipe_name.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!("failed to connect to WinSymlinksBroker pipe: {err}"),
            )
        })?;
        let handle = OwnedHandle(handle);

        verify_pipe_server_identity(handle.raw())?;

        let read_mode = PIPE_READMODE_MESSAGE;
        unsafe {
            SetNamedPipeHandleState(handle.raw(), Some(&read_mode), None, None).map_err(|err| {
                WinSymlinksError::new(
                    ErrorCode::ServiceUnavailable,
                    format!("failed to set broker pipe read mode: {err}"),
                )
            })?;
        }

        Ok(handle)
    }

    fn verify_pipe_server_identity(pipe: HANDLE) -> Result<()> {
        let mut server_process_id = 0;
        unsafe {
            GetNamedPipeServerProcessId(pipe, &mut server_process_id).map_err(|err| {
                WinSymlinksError::new(
                    ErrorCode::ServiceIdentityMismatch,
                    format!("failed to identify broker pipe server process: {err}"),
                )
            })?;
        }

        let service_process_id = service_identity::query_service_process_id()?;
        if service_process_id == Some(server_process_id) {
            Ok(())
        } else {
            Err(WinSymlinksError::new(
                ErrorCode::ServiceIdentityMismatch,
                format!(
                    "pipe server process {server_process_id} does not match running WinSymlinksBroker service process {:?}",
                    service_process_id
                ),
            ))
        }
    }

    fn read_message(pipe: HANDLE) -> Result<Vec<u8>> {
        let mut message = Vec::new();
        loop {
            let mut chunk = vec![0; PIPE_BUFFER_SIZE as usize];
            let mut bytes_read = 0;
            match unsafe { ReadFile(pipe, Some(&mut chunk), Some(&mut bytes_read), None) } {
                Ok(()) => {
                    message.extend_from_slice(&chunk[..bytes_read as usize]);
                    return Ok(message);
                }
                Err(err) => {
                    let last_error = unsafe { GetLastError() };
                    if last_error == ERROR_MORE_DATA {
                        message.extend_from_slice(&chunk[..bytes_read as usize]);
                        continue;
                    }
                    if (last_error == ERROR_BROKEN_PIPE || last_error == ERROR_NO_DATA)
                        && message.is_empty()
                    {
                        return Ok(message);
                    }
                    return Err(WinSymlinksError::new(
                        ErrorCode::ServiceUnavailable,
                        format!("failed to read broker pipe message: {err}"),
                    ));
                }
            }
        }
    }

    fn write_message(pipe: HANDLE, message: &[u8]) -> Result<()> {
        let mut written_total = 0;
        while written_total < message.len() {
            let mut written = 0;
            unsafe {
                WriteFile(
                    pipe,
                    Some(&message[written_total..]),
                    Some(&mut written),
                    None,
                )
            }
            .map_err(|err| {
                WinSymlinksError::new(
                    ErrorCode::ServiceUnavailable,
                    format!("failed to write broker pipe message: {err}"),
                )
            })?;
            written_total += written as usize;
        }
        Ok(())
    }

    fn wide_null(value: &str) -> Vec<u16> {
        OsStr::new(value).encode_wide().chain(Some(0)).collect()
    }

    struct OwnedHandle(HANDLE);

    impl OwnedHandle {
        fn raw(&self) -> HANDLE {
            self.0
        }
    }

    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::CreateSymlinkRequest;
    use crate::{ErrorCode, Result, WinSymlinksError};

    pub fn submit_create_symlink_request(_request: CreateSymlinkRequest) -> Result<()> {
        Err(WinSymlinksError::new(
            ErrorCode::ServiceUnavailable,
            "broker IPC is only supported on Windows",
        ))
    }

    pub fn check_broker_pipe() -> Result<()> {
        Err(WinSymlinksError::new(
            ErrorCode::ServiceUnavailable,
            "broker IPC is only supported on Windows",
        ))
    }
}
