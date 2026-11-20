// Release builds link for the GUI subsystem so Windows opens no console window for the tray app;
// the attribute is ignored on other targets. Debug builds keep the console for `pnpm dev`/`pnpm cli`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod commands;
mod popover;
mod tray;

fn main() {
    let args = trayci_core::cli::cli_arguments(std::env::args());
    if trayci_core::cli::is_cli_mode(&args) {
        #[cfg(windows)]
        attach_parent_console();
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
    // ponytail: Windows keeps launching the tray for every other invocation: the GUI-subsystem exe
    // cannot hold an interactive terminal, so a console `trayci-cli.exe` is the follow-up for a
    // Windows terminal view.
    #[cfg(target_os = "linux")]
    {
        use std::io::IsTerminal;
        let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
        if args.is_empty() && interactive {
            let mut repository = trayci_core::SettingsRepository::default();
            let settings = tauri::async_runtime::block_on(repository.load());
            let providers =
                app::providers(std::sync::Arc::new(std::sync::Mutex::new(settings.clone())));
            if let Err(error) = trayci_core::tui::run(providers, &settings) {
                eprintln!("trayci: {error}");
                std::process::exit(1);
            }
            std::process::exit(0);
        }
        // Debug builds stay in the foreground so `pnpm dev` keeps the app it watches.
        if args.first().is_some_and(|arg| arg == "tray") && interactive && !cfg!(debug_assertions) {
            detach();
        }
    }
    app::run();
}

/// Start the tray app without the terminal and return the prompt at once. Its own process group
/// keeps Ctrl-C and the shell's hangup away from it.
#[cfg(target_os = "linux")]
fn detach() -> ! {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let spawned = autostart::executable().and_then(|executable| {
        Command::new(executable)
            .arg("tray")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
    });
    match spawned {
        Ok(_) => std::process::exit(0),
        Err(error) => {
            eprintln!("trayci: could not start the tray app: {error}");
            std::process::exit(1);
        }
    }
}

/// Borrow the shell's console for CLI output, unless stdout is already piped or redirected.
// ponytail: the shell does not wait for a GUI-subsystem exe, so an unpiped interactive run prints
// after the prompt and leaves no exit code; a console `trayci-cli` bin (required-features, built
// only by dist:win) fixes that if interactive CLI use on Windows matters.
#[cfg(windows)]
fn attach_parent_console() {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};

    if std::io::stdout().as_raw_handle().is_null() {
        // SAFETY: plain Win32 call; failure (no parent console) only means there is nowhere to print.
        unsafe { AttachConsole(ATTACH_PARENT_PROCESS) };
    }
}
