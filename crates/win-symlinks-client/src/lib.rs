pub mod direct;
pub mod error;
pub mod pipe;
pub mod protocol;
pub mod service_identity;

use std::path::{Path, PathBuf};

pub use direct::TargetKind;
pub use error::{ErrorCode, Result, WinSymlinksError};
pub use protocol::{BrokerResponse, CreateSymlinkRequest, Operation, PROTOCOL_VERSION};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateSymlinkOptions {
    pub target_path: PathBuf,
    pub link_path: PathBuf,
    pub target_kind: Option<TargetKind>,
    pub replace_existing_symlink: bool,
}

impl CreateSymlinkOptions {
    pub fn new(target_path: impl Into<PathBuf>, link_path: impl Into<PathBuf>) -> Self {
        Self {
            target_path: target_path.into(),
            link_path: link_path.into(),
            target_kind: None,
            replace_existing_symlink: false,
        }
    }

    pub fn target_kind(mut self, target_kind: TargetKind) -> Self {
        self.target_kind = Some(target_kind);
        self
    }

    pub fn replace_existing_symlink(mut self, replace: bool) -> Self {
        self.replace_existing_symlink = replace;
        self
    }
}

pub fn create_symlink(options: CreateSymlinkOptions) -> Result<()> {
    let request = request_from_options(options)?;
    let direct_options = direct::DirectCreateOptions {
        link_path: request.link_path.clone(),
        target_path: request.target_path.clone(),
        target_kind: request.target_kind,
        replace_existing_symlink: request.replace_existing_symlink,
        allow_unprivileged_direct_create: true,
    };

    match direct::try_direct_create(&direct_options)? {
        direct::DirectCreateOutcome::Created => Ok(()),
        direct::DirectCreateOutcome::NeedsBroker => pipe::submit_create_symlink_request(request),
    }
}

pub fn create_symlink_via_broker(options: CreateSymlinkOptions) -> Result<()> {
    pipe::submit_create_symlink_request(request_from_options(options)?)
}

fn request_from_options(options: CreateSymlinkOptions) -> Result<CreateSymlinkRequest> {
    Ok(CreateSymlinkRequest::new(
        absolute_link_path(&options.link_path)?,
        options.target_path,
        options.target_kind,
        options.replace_existing_symlink,
    ))
}

fn absolute_link_path(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    Ok(std::env::current_dir()
        .map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::PathNormalizationFailed,
                format!("failed to read current directory for link path: {err}"),
            )
        })?
        .join(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_new_uses_ln_target_then_link_order() {
        let options = CreateSymlinkOptions::new("target.txt", "link.txt");

        assert_eq!(options.target_path, PathBuf::from("target.txt"));
        assert_eq!(options.link_path, PathBuf::from("link.txt"));
        assert_eq!(options.target_kind, None);
        assert!(!options.replace_existing_symlink);
    }

    #[test]
    fn builder_methods_set_target_kind_and_replace_flag() {
        let options = CreateSymlinkOptions::new("target-dir", "link-dir")
            .target_kind(TargetKind::Dir)
            .replace_existing_symlink(true);

        assert_eq!(options.target_kind, Some(TargetKind::Dir));
        assert!(options.replace_existing_symlink);
    }

    #[test]
    fn relative_link_path_resolves_against_client_cwd() {
        let path = absolute_link_path(Path::new("link.txt")).unwrap();

        assert!(path.is_absolute());
        assert!(path.ends_with("link.txt"));
    }

    #[test]
    fn request_conversion_preserves_target_kind_and_replace_flag() {
        let request = request_from_options(
            CreateSymlinkOptions::new("target.txt", "link.txt")
                .target_kind(TargetKind::File)
                .replace_existing_symlink(true),
        )
        .unwrap();

        assert_eq!(request.target_path, PathBuf::from("target.txt"));
        assert!(request.link_path.is_absolute());
        assert!(request.link_path.ends_with("link.txt"));
        assert_eq!(request.target_kind, Some(TargetKind::File));
        assert!(request.replace_existing_symlink);
    }
}
