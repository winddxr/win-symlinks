use clap::Parser;
use std::io;
use std::path::{Path, PathBuf};
use win_symlinks::client::CreateSymlinkOptions;
use win_symlinks::ipc::CreateSymlinkRequest;
use win_symlinks::symlink::TargetKind;
use win_symlinks::{ErrorCode, WinSymlinksError};

#[derive(Debug, Parser)]
#[command(
    name = "ln",
    version,
    about = "Create Windows symbolic links with Linux-like ln -s syntax."
)]
struct Cli {
    #[arg(short = 's', long = "symbolic")]
    symbolic: bool,

    #[arg(short = 'f', long = "force")]
    force: bool,

    #[arg(short = 'T')]
    no_target_directory: bool,

    #[arg(long = "win-kind", value_enum)]
    win_kind: Option<TargetKind>,

    #[arg(value_name = "PATH")]
    paths: Vec<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedLinkCommand {
    request: CreateSymlinkRequest,
    no_target_directory: bool,
}

fn main() {
    tracing_subscriber::fmt::init();

    if let Err(err) = run(Cli::parse()) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), WinSymlinksError> {
    let command = parse_link_command(cli)?;
    let options = CreateSymlinkOptions {
        target_path: command.request.target_path.clone(),
        link_path: command.request.link_path.clone(),
        target_kind: command.request.target_kind,
        replace_existing_symlink: command.request.replace_existing_symlink,
    };

    tracing::debug!(
        request = ?command.request,
        no_target_directory = command.no_target_directory
    );

    win_symlinks::client::create_symlink(options)
}

fn parse_link_command(cli: Cli) -> Result<ParsedLinkCommand, WinSymlinksError> {
    if !cli.symbolic {
        return Err(WinSymlinksError::new(
            ErrorCode::UnsupportedMode,
            "only symbolic link mode is supported; use ln -s TARGET LINK_NAME",
        ));
    }
    if cli.paths.len() != 2 {
        return Err(WinSymlinksError::new(
            ErrorCode::UnsupportedMode,
            "expected exactly TARGET and LINK_NAME",
        ));
    }

    let link_path = resolve_link_path(&cli.paths[1], &cli.paths[0], cli.no_target_directory)?;

    Ok(ParsedLinkCommand {
        request: CreateSymlinkRequest::new(
            link_path,
            cli.paths[0].clone(),
            cli.win_kind,
            cli.force,
        ),
        no_target_directory: cli.no_target_directory,
    })
}

fn resolve_link_path(
    link_path: &Path,
    target_path: &Path,
    no_target_directory: bool,
) -> Result<PathBuf, WinSymlinksError> {
    if no_target_directory {
        return Ok(link_path.to_path_buf());
    }

    match std::fs::metadata(link_path) {
        Ok(metadata) if metadata.is_dir() => {
            let target_name = target_path.file_name().ok_or_else(|| {
                WinSymlinksError::new(
                    ErrorCode::PathNormalizationFailed,
                    format!(
                        "target path has no final component for destination directory: {}",
                        target_path.display()
                    ),
                )
            })?;
            Ok(link_path.join(target_name))
        }
        Ok(_) => Ok(link_path.to_path_buf()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(link_path.to_path_buf()),
        Err(err) => Err(WinSymlinksError::new(
            ErrorCode::CreateSymlinkFailed,
            format!("failed to inspect link path: {err}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn parse(args: &[&str]) -> ParsedLinkCommand {
        parse_link_command(Cli::try_parse_from(args).expect("valid clap args"))
            .expect("valid ln command")
    }

    fn unique_temp_dir() -> PathBuf {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time is after UNIX epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("win-symlinks-ln-test-{id}"))
    }

    #[test]
    fn parses_symbolic_link_command() {
        let command = parse(&["ln", "-s", "target.txt", "link.txt"]);

        assert_eq!(command.request.target_path, PathBuf::from("target.txt"));
        assert_eq!(command.request.link_path, PathBuf::from("link.txt"));
        assert_eq!(command.request.target_kind, None);
        assert!(!command.request.replace_existing_symlink);
        assert!(!command.no_target_directory);
    }

    #[test]
    fn parses_force_symbolic_link_command() {
        let command = parse(&["ln", "-sf", "target.txt", "link.txt"]);

        assert!(command.request.replace_existing_symlink);
    }

    #[test]
    fn parses_no_target_directory_symbolic_link_command() {
        let command = parse(&["ln", "-sT", "target.txt", "link.txt"]);

        assert!(command.no_target_directory);
    }

    #[test]
    fn places_link_inside_existing_destination_directory_without_t() {
        let link_dir = unique_temp_dir();
        fs::create_dir(&link_dir).expect("create temporary link directory");

        let command = parse_link_command(
            Cli::try_parse_from([
                "ln",
                "-s",
                "target.txt",
                link_dir.to_str().expect("temporary path is valid UTF-8"),
            ])
            .expect("valid clap args"),
        )
        .expect("valid ln command");

        assert_eq!(command.request.link_path, link_dir.join("target.txt"));

        fs::remove_dir(link_dir).expect("remove temporary link directory");
    }

    #[test]
    fn no_target_directory_keeps_existing_destination_directory_as_link_path() {
        let link_dir = unique_temp_dir();
        fs::create_dir(&link_dir).expect("create temporary link directory");

        let command = parse_link_command(
            Cli::try_parse_from([
                "ln",
                "-sT",
                "target.txt",
                link_dir.to_str().expect("temporary path is valid UTF-8"),
            ])
            .expect("valid clap args"),
        )
        .expect("valid ln command");

        assert_eq!(command.request.link_path, link_dir);

        fs::remove_dir(link_dir).expect("remove temporary link directory");
    }

    #[test]
    fn parses_windows_target_kind_hints() {
        let file_command = parse(&["ln", "-s", "--win-kind=file", "target.txt", "link.txt"]);
        let dir_command = parse(&["ln", "-s", "--win-kind=dir", "target-dir", "link-dir"]);

        assert_eq!(file_command.request.target_kind, Some(TargetKind::File));
        assert_eq!(dir_command.request.target_kind, Some(TargetKind::Dir));
    }

    #[test]
    fn rejects_hardlink_style_command() {
        let err = parse_link_command(
            Cli::try_parse_from(["ln", "target.txt", "link.txt"]).expect("valid clap args"),
        )
        .expect_err("hardlink mode is unsupported");

        assert_eq!(err.code(), ErrorCode::UnsupportedMode);
        assert!(err.message().contains("only symbolic link mode"));
    }

    #[test]
    fn help_and_version_are_handled_by_clap_before_run() {
        let help = Cli::try_parse_from(["ln", "--help"]).expect_err("help exits from clap");
        let version =
            Cli::try_parse_from(["ln", "--version"]).expect_err("version exits from clap");

        assert_eq!(help.kind(), ErrorKind::DisplayHelp);
        assert_eq!(version.kind(), ErrorKind::DisplayVersion);
    }
}
