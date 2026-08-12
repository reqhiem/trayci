mod app;
mod autostart;
mod commands;
mod popover;
mod tray;

fn main() {
    let args = trayci_core::cli::cli_arguments(std::env::args());
    if trayci_core::cli::is_cli_mode(&args) {
        let mut repository = trayci_core::SettingsRepository::default();
        let settings = tauri::async_runtime::block_on(repository.load());
        let providers = app::providers(std::sync::Arc::new(std::sync::Mutex::new(settings)));
        let (code, output) = tauri::async_runtime::block_on(trayci_core::cli::run_cli(
            &args,
            &providers,
            env!("CARGO_PKG_VERSION"),
        ));
        print!("{output}");
        std::process::exit(code);
    }
    app::run();
}
