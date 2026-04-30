use std::sync::{atomic::AtomicBool, Arc};

pub use win_symlinks_client::protocol::{PIPE_NAME, PROTOCOL_VERSION, REQUEST_TIMEOUT_MS};
pub use win_symlinks_client::{BrokerResponse, CreateSymlinkRequest, Operation};

pub fn check_broker_pipe() -> crate::Result<()> {
    win_symlinks_client::pipe::check_broker_pipe()
}

pub fn run_broker_pipe_server(should_stop: Arc<AtomicBool>) -> crate::Result<()> {
    platform::run_broker_pipe_server(should_stop)
}

pub fn wake_broker_pipe_server() {
    platform::wake_broker_pipe_server()
}

#[cfg(windows)]
mod platform {
    use super::{
        BrokerResponse, CreateSymlinkRequest, Operation, PIPE_NAME, PROTOCOL_VERSION,
        REQUEST_TIMEOUT_MS,
    };
    use crate::{path_policy, symlink, ErrorCode, Result, WinSymlinksError};
    use std::ffi::OsStr;
    use std::os::windows::{ffi::OsStrExt, fs::MetadataExt};
    use std::path::{Path, PathBuf};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::time::{SystemTime, UNIX_EPOCH};
    use uuid::Uuid;
    use windows::{
        core::{PCWSTR, PWSTR},
        Win32::{
            Foundation::{
                CloseHandle, GetLastError, LocalFree, ERROR_BROKEN_PIPE, ERROR_MORE_DATA,
                ERROR_NO_DATA, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL,
            },
            Security::{
                Authorization::{
                    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                    SDDL_REVISION_1,
                },
                GetTokenInformation, RevertToSelf, TokenGroups, TokenUser, PSECURITY_DESCRIPTOR,
                SECURITY_ATTRIBUTES, TOKEN_GROUPS, TOKEN_QUERY, TOKEN_USER,
            },
            Storage::FileSystem::{
                CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_ADD_FILE,
                FILE_ADD_SUBDIRECTORY, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
                FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
                OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
            },
            System::Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
                ImpersonateNamedPipeClient, WaitNamedPipeW, PIPE_READMODE_MESSAGE,
                PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
            },
            System::Threading::{GetCurrentThread, GetCurrentThreadId, OpenThreadToken},
        },
    };

    const PIPE_BUFFER_SIZE: u32 = 64 * 1024;
    const PIPE_SECURITY_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";

    pub fn run_broker_pipe_server(should_stop: Arc<AtomicBool>) -> Result<()> {
        while !should_stop.load(Ordering::SeqCst) {
            let pipe = create_server_pipe()?;
            match connect_pipe_client(pipe.raw()) {
                Ok(()) => {}
                Err(err) => {
                    tracing::warn!(%err, "failed to connect named pipe client");
                    continue;
                }
            }

            if should_stop.load(Ordering::SeqCst) {
                continue;
            }

            if let Err(err) = process_pipe_client(pipe.raw()) {
                tracing::warn!(%err, "failed to process named pipe client");
            }
        }

        Ok(())
    }

    pub fn wake_broker_pipe_server() {
        let pipe_name = wide_null(PIPE_NAME);
        let available = unsafe { WaitNamedPipeW(PCWSTR(pipe_name.as_ptr()), 100).as_bool() };
        if !available {
            return;
        }

        if let Ok(handle) = unsafe {
            CreateFileW(
                PCWSTR(pipe_name.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        } {
            let _handle = OwnedHandle(handle);
        }
    }

    fn create_server_pipe() -> Result<OwnedHandle> {
        let pipe_name = wide_null(PIPE_NAME);
        let mut security_descriptor = PSECURITY_DESCRIPTOR::default();
        let security_sddl = wide_null(PIPE_SECURITY_SDDL);
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(security_sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut security_descriptor,
                None,
            )
            .map_err(|err| {
                WinSymlinksError::new(
                    ErrorCode::ServiceUnavailable,
                    format!("failed to build broker pipe security descriptor: {err}"),
                )
            })?;
        }

        let security_attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security_descriptor.0,
            bInheritHandle: false.into(),
        };

        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(pipe_name.as_ptr()),
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                PIPE_BUFFER_SIZE,
                PIPE_BUFFER_SIZE,
                REQUEST_TIMEOUT_MS as u32,
                Some(&security_attributes),
            )
        };
        unsafe {
            let _ = LocalFree(Some(HLOCAL(security_descriptor.0)));
        }

        if handle.is_invalid() {
            Err(WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!(
                    "failed to create WinSymlinksBroker pipe: Windows error {}",
                    unsafe { GetLastError() }.0
                ),
            ))
        } else {
            Ok(OwnedHandle(handle))
        }
    }

    fn connect_pipe_client(pipe: HANDLE) -> Result<()> {
        match unsafe { ConnectNamedPipe(pipe, None) } {
            Ok(()) => Ok(()),
            Err(_) if unsafe { GetLastError() } == ERROR_PIPE_CONNECTED => Ok(()),
            Err(err) => Err(WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!("failed to accept broker pipe client: {err}"),
            )),
        }
    }

    fn process_pipe_client(pipe: HANDLE) -> Result<()> {
        let request_bytes = read_message(pipe)?;
        if request_bytes.is_empty() {
            return Ok(());
        }

        let response = broker_response_for_request(&request_bytes, pipe);
        write_message(
            pipe,
            &serde_json::to_vec(&response).map_err(|err| {
                WinSymlinksError::new(
                    ErrorCode::ServiceUnavailable,
                    format!("failed to serialize broker response: {err}"),
                )
            })?,
        )?;
        let _ = unsafe { FlushFileBuffers(pipe) };
        let _ = unsafe { DisconnectNamedPipe(pipe) };
        Ok(())
    }

    fn broker_response_for_request(request_bytes: &[u8], pipe: HANDLE) -> BrokerResponse {
        let request = match serde_json::from_slice::<CreateSymlinkRequest>(request_bytes) {
            Ok(request) => request,
            Err(err) => {
                return BrokerResponse::error(
                    Uuid::nil(),
                    ErrorCode::ServiceUnavailable,
                    format!("invalid broker request JSON: {err}"),
                );
            }
        };

        match process_create_symlink_request(request.clone(), pipe) {
            Ok(()) => BrokerResponse::ok(request.request_id),
            Err(err) => BrokerResponse::error(request.request_id, err.code(), err.message()),
        }
    }

    fn process_create_symlink_request(request: CreateSymlinkRequest, pipe: HANDLE) -> Result<()> {
        validate_request_schema(&request)?;

        let mut impersonation = ClientImpersonation::start(pipe)?;
        let caller = impersonation.identify_client()?;

        let normalized_link_path = path_policy::normalize_for_policy(&request.link_path)?;
        let target_for_inspection =
            target_path_for_inspection(&normalized_link_path, &request.target_path)?;
        let _normalized_target_path = path_policy::normalize_for_policy(&target_for_inspection)?;

        let config = crate::config::load_config()?;
        let blacklist = path_policy::merge_source_blacklist(&config.additional_source_blacklist);
        if let Some(entry) = blacklist.is_blocked(&normalized_link_path)? {
            return Err(WinSymlinksError::new(
                ErrorCode::SourceBlacklisted,
                format!(
                    "link path is blocked by source blacklist: {}",
                    entry.path.display()
                ),
            ));
        }

        verify_parent_create_access(&normalized_link_path)?;
        impersonation.revert()?;

        let target_kind = symlink::decide_target_kind(&target_for_inspection, request.target_kind)?;
        let link_state = symlink::inspect_link_path_state(&normalized_link_path)?;
        let replacement_plan =
            symlink::plan_replacement(link_state, request.replace_existing_symlink)?;

        if replacement_plan == symlink::ReplacementPlan::ReplaceExistingSymlink {
            remove_existing_symlink_after_recheck(&normalized_link_path)?;
        }

        match symlink::create_symbolic_link(
            &normalized_link_path,
            &request.target_path,
            target_kind,
            false,
        ) {
            Ok(()) => {
                write_audit_log(
                    &caller.sid_string,
                    &normalized_link_path,
                    &request.target_path,
                    target_kind,
                )?;
                Ok(())
            }
            Err(err) if replacement_plan == symlink::ReplacementPlan::ReplaceExistingSymlink => {
                Err(WinSymlinksError::new(
                    ErrorCode::ReplacementPartiallyCompleted,
                    format!(
                        "existing symbolic link was removed, but creating the replacement failed: {err}"
                    ),
                ))
            }
            Err(err) => Err(err),
        }
    }

    pub(super) fn validate_request_schema(request: &CreateSymlinkRequest) -> Result<()> {
        if request.version != PROTOCOL_VERSION {
            return Err(WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                format!(
                    "unsupported broker protocol version {}; expected {}",
                    request.version, PROTOCOL_VERSION
                ),
            ));
        }

        if request.operation != Operation::CreateSymlink {
            return Err(WinSymlinksError::new(
                ErrorCode::ServiceUnavailable,
                "unsupported broker operation",
            ));
        }

        Ok(())
    }

    pub(super) fn target_path_for_inspection(
        link_path: &Path,
        target_path: &Path,
    ) -> Result<PathBuf> {
        if target_path.is_absolute() {
            return Ok(target_path.to_path_buf());
        }

        let Some(parent) = link_path.parent() else {
            return Err(WinSymlinksError::new(
                ErrorCode::PathNormalizationFailed,
                format!("link path has no parent directory: {}", link_path.display()),
            ));
        };
        Ok(parent.join(target_path))
    }

    fn verify_parent_create_access(link_path: &Path) -> Result<()> {
        let Some(parent) = link_path.parent() else {
            return Err(WinSymlinksError::new(
                ErrorCode::CallerParentWriteDenied,
                format!("link path has no parent directory: {}", link_path.display()),
            ));
        };

        let parent = wide_null_os(parent.as_os_str());
        let handle = unsafe {
            CreateFileW(
                PCWSTR(parent.as_ptr()),
                FILE_ADD_FILE.0 | FILE_ADD_SUBDIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        }
        .map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::CallerParentWriteDenied,
                format!("caller cannot create entries in link parent directory: {err}"),
            )
        })?;
        let _handle = OwnedHandle(handle);

        Ok(())
    }

    fn remove_existing_symlink_after_recheck(link_path: &Path) -> Result<()> {
        let link_state = symlink::inspect_link_path_state(link_path)?;
        symlink::plan_replacement(link_state, true)?;

        let metadata = std::fs::symlink_metadata(link_path).map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::CreateSymlinkFailed,
                format!("failed to re-inspect existing symbolic link before replacement: {err}"),
            )
        })?;

        let result = if metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
            std::fs::remove_dir(link_path)
        } else {
            std::fs::remove_file(link_path)
        };

        result.map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::CreateSymlinkFailed,
                format!("failed to remove existing symbolic link before replacement: {err}"),
            )
        })
    }

    fn write_audit_log(
        caller_sid: &str,
        link_path: &Path,
        target_path: &Path,
        target_kind: symlink::TargetKind,
    ) -> Result<()> {
        let Some(program_data) = std::env::var_os("ProgramData") else {
            return Err(WinSymlinksError::new(
                ErrorCode::CreateSymlinkFailed,
                "ProgramData is not set; cannot write privileged operation audit log",
            ));
        };
        let log_dir = PathBuf::from(program_data).join("win-symlinks");
        std::fs::create_dir_all(&log_dir).map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::CreateSymlinkFailed,
                format!("failed to create audit log directory: {err}"),
            )
        })?;

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| {
                WinSymlinksError::new(
                    ErrorCode::CreateSymlinkFailed,
                    format!("system clock is before Unix epoch: {err}"),
                )
            })?
            .as_secs();
        let entry = serde_json::json!({
            "timestamp_unix_seconds": timestamp,
            "operation": "create_symlink",
            "caller_sid": caller_sid,
            "link_path": link_path,
            "target_path": target_path,
            "target_kind": target_kind.as_protocol_value(),
        });
        let line = format!(
            "{}\n",
            serde_json::to_string(&entry).map_err(|err| {
                WinSymlinksError::new(
                    ErrorCode::CreateSymlinkFailed,
                    format!("failed to serialize audit log entry: {err}"),
                )
            })?
        );
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_dir.join("manifest.jsonl"))
            .and_then(|mut file| {
                use std::io::Write;
                file.write_all(line.as_bytes())
            })
            .map_err(|err| {
                WinSymlinksError::new(
                    ErrorCode::CreateSymlinkFailed,
                    format!("failed to append audit log entry: {err}"),
                )
            })
    }

    struct ClientIdentity {
        sid_string: String,
    }

    struct ClientImpersonation {
        active: bool,
    }

    impl ClientImpersonation {
        fn start(pipe: HANDLE) -> Result<Self> {
            unsafe { ImpersonateNamedPipeClient(pipe) }.map_err(|err| {
                WinSymlinksError::new(
                    ErrorCode::RemoteClientRejected,
                    format!("failed to impersonate broker pipe client: {err}"),
                )
            })?;

            Ok(Self { active: true })
        }

        fn identify_client(&self) -> Result<ClientIdentity> {
            let token = open_current_thread_token()?;
            let user_sid = token_user_sid_string(token.raw())?;
            if user_sid == "S-1-5-7" {
                return Err(WinSymlinksError::new(
                    ErrorCode::RemoteClientRejected,
                    "anonymous broker pipe clients are not accepted",
                ));
            }

            let group_sids = token_group_sid_strings(token.raw())?;
            if group_sids.iter().any(|sid| sid == "S-1-5-2") {
                return Err(WinSymlinksError::new(
                    ErrorCode::RemoteClientRejected,
                    "network logon broker pipe clients are not accepted",
                ));
            }
            if group_sids.iter().any(|sid| sid == "S-1-5-7") {
                return Err(WinSymlinksError::new(
                    ErrorCode::RemoteClientRejected,
                    "anonymous broker pipe clients are not accepted",
                ));
            }

            Ok(ClientIdentity {
                sid_string: user_sid,
            })
        }

        fn revert(&mut self) -> Result<()> {
            if self.active {
                unsafe { RevertToSelf() }.map_err(|err| {
                    WinSymlinksError::new(
                        ErrorCode::ServiceUnavailable,
                        format!(
                            "failed to return broker thread {} to service token: {err}",
                            unsafe { GetCurrentThreadId() }
                        ),
                    )
                })?;
                self.active = false;
            }

            Ok(())
        }
    }

    impl Drop for ClientImpersonation {
        fn drop(&mut self) {
            if self.active {
                let _ = unsafe { RevertToSelf() };
            }
        }
    }

    fn open_current_thread_token() -> Result<OwnedHandle> {
        let mut token = HANDLE::default();
        unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut token) }.map_err(
            |err| {
                WinSymlinksError::new(
                    ErrorCode::RemoteClientRejected,
                    format!("failed to open impersonated client token: {err}"),
                )
            },
        )?;

        Ok(OwnedHandle(token))
    }

    fn token_user_sid_string(token: HANDLE) -> Result<String> {
        let bytes = get_token_information(token, TokenUser)?;
        let token_user = unsafe { &*(bytes.as_ptr() as *const TOKEN_USER) };
        sid_to_string(token_user.User.Sid)
    }

    fn token_group_sid_strings(token: HANDLE) -> Result<Vec<String>> {
        let bytes = get_token_information(token, TokenGroups)?;
        let token_groups = unsafe { &*(bytes.as_ptr() as *const TOKEN_GROUPS) };
        let groups = unsafe {
            std::slice::from_raw_parts(
                token_groups.Groups.as_ptr(),
                token_groups.GroupCount as usize,
            )
        };

        groups
            .iter()
            .map(|group| sid_to_string(group.Sid))
            .collect()
    }

    fn get_token_information(
        token: HANDLE,
        class: windows::Win32::Security::TOKEN_INFORMATION_CLASS,
    ) -> Result<Vec<u8>> {
        let mut required = 0;
        let _ = unsafe { GetTokenInformation(token, class, None, 0, &mut required) };
        if required == 0 {
            return Err(WinSymlinksError::new(
                ErrorCode::RemoteClientRejected,
                format!(
                    "failed to determine token information size: Windows error {}",
                    unsafe { GetLastError() }.0
                ),
            ));
        }

        let mut bytes = vec![0u8; required as usize];
        unsafe {
            GetTokenInformation(
                token,
                class,
                Some(bytes.as_mut_ptr().cast()),
                required,
                &mut required,
            )
        }
        .map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::RemoteClientRejected,
                format!("failed to inspect impersonated client token: {err}"),
            )
        })?;

        Ok(bytes)
    }

    fn sid_to_string(sid: windows::Win32::Security::PSID) -> Result<String> {
        let mut string_sid = PWSTR::null();
        unsafe { ConvertSidToStringSidW(sid, &mut string_sid) }.map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::RemoteClientRejected,
                format!("failed to convert client SID to string: {err}"),
            )
        })?;

        let sid_string = unsafe { string_sid.to_string() }.map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::RemoteClientRejected,
                format!("failed to read client SID string: {err}"),
            )
        })?;
        unsafe {
            let _ = LocalFree(Some(HLOCAL(string_sid.0.cast())));
        }

        Ok(sid_string)
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

    fn wide_null_os(value: &OsStr) -> Vec<u16> {
        value.encode_wide().chain(Some(0)).collect()
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
    use crate::{ErrorCode, Result, WinSymlinksError};
    use std::sync::{atomic::AtomicBool, Arc};

    pub fn run_broker_pipe_server(_should_stop: Arc<AtomicBool>) -> Result<()> {
        Err(WinSymlinksError::new(
            ErrorCode::ServiceUnavailable,
            "broker IPC server is only supported on Windows",
        ))
    }

    pub fn wake_broker_pipe_server() {}
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;
    #[cfg(windows)]
    use std::path::PathBuf;
    #[cfg(windows)]
    use uuid::Uuid;

    #[cfg(windows)]
    fn request_id() -> Uuid {
        Uuid::parse_str("018f5b2a-7f3a-7b7a-9c21-000000000001").unwrap()
    }

    #[cfg(windows)]
    fn request_with_id(request_id: Uuid) -> CreateSymlinkRequest {
        CreateSymlinkRequest {
            version: PROTOCOL_VERSION,
            request_id,
            operation: Operation::CreateSymlink,
            link_path: PathBuf::from("link"),
            target_path: PathBuf::from("target"),
            target_kind: Some(crate::TargetKind::File),
            replace_existing_symlink: false,
        }
    }

    #[cfg(windows)]
    #[test]
    fn broker_rejects_unsupported_protocol_version() {
        let request = CreateSymlinkRequest {
            version: PROTOCOL_VERSION + 1,
            ..request_with_id(request_id())
        };

        let err = platform::validate_request_schema(&request).unwrap_err();

        assert_eq!(err.code(), crate::ErrorCode::ServiceUnavailable);
        assert!(err.message().contains("protocol version"));
    }

    #[cfg(windows)]
    #[test]
    fn broker_inspects_relative_targets_from_link_parent() {
        let link_path = PathBuf::from(r"C:\workspace\links\pkg");
        let target_path = PathBuf::from(r"..\shared\pkg");

        let resolved = platform::target_path_for_inspection(&link_path, &target_path).unwrap();

        assert_eq!(resolved, PathBuf::from(r"C:\workspace\links\..\shared\pkg"));
    }
}
