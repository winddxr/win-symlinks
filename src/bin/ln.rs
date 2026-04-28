use clap::Parser;
use std::path::PathBuf;
use win_symlinks::ipc::CreateSymlinkRequest;
use win_symlinks::symlink::{CreateSymlinkOptions, DirectCreateOutcome, TargetKind};
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
        link_path: command.request.link_path.clone(),
        target_path: command.request.target_path.clone(),
        target_kind: command.request.target_kind,
        replace_existing_symlink: command.request.replace_existing_symlink,
        allow_unprivileged_direct_create: true,
    };

    tracing::debug!(
        request = ?command.request,
        no_target_directory = command.no_target_directory
    );

    match win_symlinks::symlink::try_direct_create(&options)? {
        DirectCreateOutcome::Created => Ok(()),
        DirectCreateOutcome::NeedsBroker => Err(WinSymlinksError::new(
            ErrorCode::ServiceUnavailable,
            "direct symbolic link creation needs the broker; broker IPC is not implemented yet",
        )),
    }
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

    Ok(ParsedLinkCommand {
        request: CreateSymlinkRequest::new(
            cli.paths[1].clone(),
            cli.paths[0].clone(),
            cli.win_kind,
            cli.force,
        ),
        no_target_directory: cli.no_target_directory,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    fn parse(args: &[&str]) -> ParsedLinkCommand {
        parse_link_command(Cli::try_parse_from(args).expect("valid clap args"))
            .expect("valid ln command")
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
