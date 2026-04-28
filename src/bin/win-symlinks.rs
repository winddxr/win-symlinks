use clap::{Parser, Subcommand};
use win_symlinks::config::{default_config_path, AppConfig};
use win_symlinks::service;
use win_symlinks::{ErrorCode, WinSymlinksError};

#[derive(Debug, Parser)]
#[command(
    name = "win-symlinks",
    version,
    about = "Manage and diagnose WinSymlinksBroker."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    Doctor,
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ServiceCommand {
    Install,
    Uninstall,
    Start,
    Stop,
    Status,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Show,
}

fn main() {
    tracing_subscriber::fmt::init();

    if let Err(err) = run(Cli::parse()) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<(), WinSymlinksError> {
    match cli.command {
        Command::Service { command } => service_command(command),
        Command::Doctor => Err(WinSymlinksError::new(
            ErrorCode::ServiceUnavailable,
            "doctor diagnostics are not implemented yet",
        )),
        Command::Config {
            command: ConfigCommand::Show,
        } => show_config(),
    }
}

fn service_command(command: ServiceCommand) -> Result<(), WinSymlinksError> {
    match command {
        ServiceCommand::Install => {
            service::install_service()?;
            println!(
                "installed {} ({})",
                service::SERVICE_NAME,
                service::SERVICE_DISPLAY_NAME
            );
            Ok(())
        }
        ServiceCommand::Uninstall => {
            service::uninstall_service()?;
            println!("uninstalled {}", service::SERVICE_NAME);
            Ok(())
        }
        ServiceCommand::Start => {
            service::start_service()?;
            println!("started {}", service::SERVICE_NAME);
            Ok(())
        }
        ServiceCommand::Stop => {
            service::stop_service()?;
            println!("stopped {}", service::SERVICE_NAME);
            Ok(())
        }
        ServiceCommand::Status => {
            let state = service::query_service_state()?;
            println!("service: {}", service::SERVICE_NAME);
            println!("display_name: {}", service::SERVICE_DISPLAY_NAME);
            println!(
                "installed: {}",
                state != win_symlinks::service::ServiceState::NotInstalled
            );
            println!("state: {state}");
            Ok(())
        }
    }
}

fn show_config() -> Result<(), WinSymlinksError> {
    let config = AppConfig::default();
    let effective_source_blacklist =
        win_symlinks::path_policy::merge_source_blacklist(&config.additional_source_blacklist);
    let json = serde_json::json!({
        "config_path": default_config_path(),
        "effective_config": config,
        "built_in_source_blacklist": win_symlinks::path_policy::built_in_source_blacklist().entries(),
        "effective_source_blacklist": effective_source_blacklist.entries(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&json).map_err(|err| {
            WinSymlinksError::new(
                ErrorCode::CreateSymlinkFailed,
                format!("failed to serialize configuration: {err}"),
            )
        })?
    );
    Ok(())
}
