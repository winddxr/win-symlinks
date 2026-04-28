fn main() {
    tracing_subscriber::fmt::init();

    if let Err(err) = win_symlinks::service::run_broker_service() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
