use clap::Parser;
use std::path::PathBuf;
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

fn main() {
    tracing_subscriber::fmt::init();

    if let Err(err) = run(Cli::parse()) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), WinSymlinksError> {
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

    let request = CreateSymlinkRequest::new(
        cli.paths[1].clone(),
        cli.paths[0].clone(),
        cli.win_kind,
        cli.force,
    );

    tracing::debug!(?request, no_target_directory = cli.no_target_directory);

    Err(WinSymlinksError::new(
        ErrorCode::ServiceUnavailable,
        "ln argument parsing is ready; direct and broker link creation are not implemented yet",
    ))
}
