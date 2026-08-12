mod app;
mod autostart;
mod commands;
mod popover;
mod tray;

/// GTK ignores `gtk_window_move` on Wayland, and the popover is nothing but a positioned window:
/// it could not anchor to the tray icon, could not be dragged, and could not restore where it was
/// left. XWayland does honour all three, so prefer it and leave `GDK_BACKEND` as an escape hatch.
#[cfg(target_os = "linux")]
fn prefer_x11() {
    let set = |name: &str| std::env::var_os(name).is_some();
    if set("WAYLAND_DISPLAY") && set("DISPLAY") && !set("GDK_BACKEND") {
        std::env::set_var("GDK_BACKEND", "x11");
    }
}

fn main() {
    #[cfg(target_os = "linux")]
    prefer_x11();
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
