use crate::{autostart, commands, popover, tray};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, RunEvent};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::Mutex as AsyncMutex;
use trayci_core::{
    service::Subscription, QuotaNotifier, SettingsRepository, TrayciSettings, UsageProvider,
    UsageService,
};

pub struct AppState {
    pub service: Arc<UsageService>,
    pub repository: Arc<AsyncMutex<SettingsRepository>>,
    pub settings: Arc<Mutex<TrayciSettings>>,
    _subscription: Mutex<Option<Subscription>>,
}

pub fn providers(settings: Arc<Mutex<TrayciSettings>>) -> Vec<Box<dyn UsageProvider>> {
    use trayci_core::providers::{AntigravityProvider, ClaudeProvider, CodexProvider};

    let claude = Arc::clone(&settings);
    let codex = Arc::clone(&settings);
    vec![
        Box::new(ClaudeProvider::new(move || {
            claude.lock().expect("settings lock").clone()
        })),
        Box::new(CodexProvider::new(move || {
            codex.lock().expect("settings lock").clone()
        })),
        Box::new(AntigravityProvider::new(move || {
            settings.lock().expect("settings lock").clone()
        })),
    ]
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window(popover::LABEL) {
                let position = window.cursor_position().unwrap_or_default();
                let _ = popover::show(app, position, None);
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        if let Some(window) = app.get_webview_window(popover::LABEL) {
                            let position = window.cursor_position().unwrap_or_default();
                            let _ = popover::toggle(app, position, None);
                        }
                    }
                })
                .build(),
        )
        .manage(popover::PopoverState::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_usage_state,
            commands::refresh_all,
            commands::get_settings,
            commands::update_settings,
            commands::begin_popover_drag,
            commands::hide_popover,
            commands::resize_popover,
            commands::quit_app,
        ])
        .setup(|app| {
            let mut repository = SettingsRepository::default();
            let settings = tauri::async_runtime::block_on(repository.load());
            tauri::async_runtime::block_on(autostart::set_enabled(settings.start_on_login))?;
            app.state::<popover::PopoverState>()
                .set_custom_position(settings.window_position);
            let font_scale = settings.font_scale;
            let current = Arc::new(Mutex::new(settings));
            let settings_for_service = Arc::clone(&current);
            let service = Arc::new(UsageService::new(
                providers(Arc::clone(&current)),
                move || settings_for_service.lock().expect("settings lock").clone(),
            ));

            popover::create(app.handle())?;
            popover::set_scale(app.handle(), font_scale)?;
            tray::create(app.handle())?;
            use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
            if let Ok(shortcut) = "Ctrl+Alt+U".parse::<Shortcut>() {
                let _ = app.global_shortcut().register(shortcut);
            }
            let handle = app.handle().clone();
            let notification_settings = Arc::clone(&current);
            let notifier = Arc::new(Mutex::new(QuotaNotifier::default()));
            let subscription = service.subscribe(move |state| {
                let _ = handle.emit_to(popover::LABEL, "usage:state-changed", &state);
                if let Some(icon) = handle.tray_by_id(tray::ID) {
                    let settings = notification_settings.lock().expect("settings lock");
                    let _ = icon.set_tooltip(Some(tray::tooltip(&state, &settings)));
                }
                let settings = notification_settings
                    .lock()
                    .expect("settings lock")
                    .notifications
                    .clone();
                let requests =
                    notifier
                        .lock()
                        .expect("notifier lock")
                        .update(&state, &settings, now_ms());
                for request in requests {
                    let _ = handle
                        .notification()
                        .builder()
                        .title(request.title)
                        .body(request.body)
                        .show();
                }
            });
            app.manage(AppState {
                service: Arc::clone(&service),
                repository: Arc::new(AsyncMutex::new(repository)),
                settings: current,
                _subscription: Mutex::new(Some(subscription)),
            });
            tauri::async_runtime::spawn(async move { service.start().await });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Trayci");

    app.run(|handle, event| {
        if matches!(event, RunEvent::ExitRequested { .. }) {
            popover::save_position(handle);
            if let Some(state) = handle.try_state::<AppState>() {
                tauri::async_runtime::block_on(state.service.stop());
            }
        }
    });
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
