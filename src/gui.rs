//! The tray app: the usage service on a tokio runtime, the Slint popover on this thread, and the
//! tray icon between them.
use crate::{
    autostart, hotkey,
    popover::{self, Popover},
    single_instance, tray,
    ui::{self, Bridge},
    view,
};
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{
    error::Error,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    runtime::{Handle, Runtime},
    sync::{watch, Mutex as AsyncMutex},
};
use trayci_core::{
    presentation::format_age, NotificationRequest, QuotaNotifier, SettingsRepository,
    TrayciSettings, TrayciSettingsPatch, UsageFetchReason, UsageService, UsageState,
};

/// Everything a callback needs. Sendable, so tokio tasks can hold it; the Slint side is only ever
/// touched on the event loop.
struct App {
    ui: slint::Weak<ui::Popover>,
    popover: Popover,
    runtime: Handle,
    service: Arc<UsageService>,
    repository: AsyncMutex<SettingsRepository>,
    settings: Arc<Mutex<TrayciSettings>>,
    /// What the bridge last showed. Edits are diffed against it, not against the saved settings:
    /// those can be one save ahead of what the edit was made from, and the diff would revert it.
    shown: Mutex<Option<TrayciSettings>>,
    state: Mutex<UsageState>,
}

pub fn run(runtime: Runtime) -> Result<(), Box<dyn Error>> {
    let Some(instance) = single_instance::claim() else {
        return Ok(());
    };
    let mut repository = SettingsRepository::default();
    let settings = runtime.block_on(repository.load());
    if let Err(error) = runtime.block_on(autostart::set_enabled(settings.start_on_login)) {
        eprintln!("trayci: could not update start on login: {error}");
    }
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_window_attributes_hook(popover::window_attributes)
        .select()?;
    let ui = ui::Popover::new()?;
    let popover = Popover::new(&ui, settings.window_position);
    let settings = Arc::new(Mutex::new(settings));
    let service = Arc::new(UsageService::new(
        crate::providers(Arc::clone(&settings)),
        {
            let settings = Arc::clone(&settings);
            move || settings.lock().expect("settings lock").clone()
        },
    ));
    let app = Arc::new(App {
        ui: ui.as_weak(),
        popover: popover.clone(),
        runtime: runtime.handle().clone(),
        service: Arc::clone(&service),
        repository: AsyncMutex::new(repository),
        settings,
        shown: Mutex::default(),
        state: Mutex::default(),
    });
    app.render();
    bind(&ui, &app);
    popover.on_save({
        let app = Arc::downgrade(&app);
        move |position| {
            if let Some(app) = app.upgrade() {
                app.update(position_patch(Some(position)));
            }
        }
    });

    let (tooltips, tooltip) = watch::channel("Trayci".to_string());
    let notifier = Mutex::new(QuotaNotifier::default());
    let subscription = service.subscribe({
        let app = Arc::clone(&app);
        move |state| {
            let settings = app.settings();
            tooltips.send_replace(tray::tooltip(&state, &settings));
            let requests = notifier.lock().expect("notifier lock").update(
                &state,
                &settings.notifications,
                now_ms(),
            );
            for request in requests {
                notify(&app.runtime, request);
            }
            *app.state.lock().expect("state lock") = state;
            app.render_later();
        }
    });
    tray::spawn(runtime.handle(), popover.clone(), tooltip);
    let _shortcut = hotkey::register(popover.clone());
    instance.serve({
        let popover = popover.clone();
        move || {
            let popover = popover.clone();
            let _ = slint::invoke_from_event_loop(move || popover.show(None));
        }
    });
    runtime.spawn({
        let service = Arc::clone(&service);
        async move { service.start().await }
    });

    // Countdowns and ages move on while no new reading arrives.
    let clock = slint::Timer::default();
    clock.start(slint::TimerMode::Repeated, Duration::from_secs(60), {
        let app = Arc::clone(&app);
        move || app.render()
    });

    let result = slint::run_event_loop_until_quit();
    // What `RunEvent::ExitRequested` did under Tauri, awaited here: `App::update` only spawns, and
    // the runtime would drop the write.
    if let Some(position) = popover.take_pending() {
        let saved = runtime.block_on(async {
            let patch = position_patch(Some(position));
            app.repository.lock().await.update(patch).await
        });
        if let Err(error) = saved {
            eprintln!("trayci: could not save the popover position: {error}");
        }
    }
    drop(subscription);
    runtime.block_on(service.stop());
    Ok(result?)
}

fn bind(ui: &ui::Popover, app: &Arc<App>) {
    let bridge = ui.global::<Bridge>();
    bridge.on_refresh({
        let app = Arc::clone(app);
        move || {
            let service = Arc::clone(&app.service);
            app.runtime
                .spawn(async move { service.refresh_all(UsageFetchReason::Manual).await });
        }
    });
    bridge.on_update_settings({
        let app = Arc::clone(app);
        move |edited| {
            let shown = app.shown.lock().expect("shown lock").clone();
            let shown = shown.unwrap_or_else(|| app.settings());
            app.update(view::patch(&shown, &edited))
        }
    });
    bridge.on_set_position({
        let app = Arc::clone(app);
        move |x, y| {
            app.popover.place((x, y));
            app.update(position_patch(Some((x, y))));
        }
    });
    bridge.on_reset_position({
        let app = Arc::clone(app);
        move || {
            app.popover.reanchor();
            app.update(position_patch(None));
        }
    });
    bridge.on_begin_drag({
        let popover = app.popover.clone();
        move || popover.begin_drag()
    });
    bridge.on_hide({
        let popover = app.popover.clone();
        move || popover.hide()
    });
    bridge.on_quit(|| {
        let _ = slint::quit_event_loop();
    });
}

impl App {
    fn settings(&self) -> TrayciSettings {
        self.settings.lock().expect("settings lock").clone()
    }

    /// Refills the bridge. Slint thread only.
    fn render(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let state = self.state.lock().expect("state lock").clone();
        let settings = self.settings();
        let now = now_ms();
        let bridge = ui.global::<Bridge>();
        bridge.set_providers(ModelRc::new(VecModel::from(view::providers(
            &state, &settings, now,
        ))));
        bridge.set_refreshing(state.is_refreshing);
        bridge.set_refreshed_ago(
            state
                .last_refresh_completed_at
                .map(|at| format_age(at, now))
                .unwrap_or_default()
                .into(),
        );
        let mut shown = self.shown.lock().expect("shown lock");
        // Setting an equal struct still replaces the providers model, which rebuilds their rows
        // and drops the keyboard focus on them.
        if shown.as_ref() != Some(&settings) {
            bridge.set_settings(view::settings(&settings, popover::positions_are_real()));
            *shown = Some(settings);
        }
    }

    fn render_later(self: &Arc<Self>) {
        let app = Arc::clone(self);
        let _ = slint::invoke_from_event_loop(move || app.render());
    }

    fn show_error(&self, error: String) {
        let _ = self.ui.upgrade_in_event_loop(move |ui| {
            ui.global::<Bridge>().set_error(error.into());
        });
    }

    /// Saves the patch, then applies what it changed.
    fn update(self: &Arc<Self>, patch: TrayciSettingsPatch) {
        let app = Arc::clone(self);
        self.runtime.spawn(async move {
            // Read and stored under the repository lock: with two quick edits, a `previous` read
            // outside it can already hold the other save, and a provider switch would then not
            // count as a change.
            let (previous, settings) = {
                let mut repository = app.repository.lock().await;
                let previous = repository.get();
                let settings = match repository.update(patch).await {
                    Ok(settings) => settings,
                    Err(error) => return app.show_error(error.to_string()),
                };
                *app.settings.lock().expect("settings lock") = settings.clone();
                (previous, settings)
            };
            app.show_error(String::new());
            if let Err(error) = autostart::set_enabled(settings.start_on_login).await {
                app.show_error(error.to_string());
            }
            // Themes, text size and percentage style change nothing a provider would answer
            // differently, so they must not spend a request: toggling them used to refresh every
            // provider.
            app.service
                .settings_changed(previous.providers != settings.providers)
                .await;
        });
    }
}

#[cfg(target_os = "linux")]
fn notify(runtime: &Handle, request: NotificationRequest) {
    runtime.spawn(async move {
        let shown = notify_rust::Notification::new()
            .appname("Trayci")
            .summary(&request.title)
            .body(&request.body)
            .show_async()
            .await;
        if let Err(error) = shown {
            eprintln!("trayci: notification failed: {error}");
        }
    });
}

/// W4 sends these as Windows toasts.
#[cfg(windows)]
fn notify(_runtime: &Handle, _request: NotificationRequest) {}

fn position_patch(position: Option<(i32, i32)>) -> TrayciSettingsPatch {
    TrayciSettingsPatch {
        window_position: Some(position),
        ..Default::default()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
