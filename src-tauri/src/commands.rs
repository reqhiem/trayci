use crate::{app::AppState, autostart, popover};
use tauri::{AppHandle, Manager, State};
use trayci_core::{TrayciSettings, TrayciSettingsPatch, UsageFetchReason, UsageState};

#[tauri::command]
pub async fn get_usage_state(state: State<'_, AppState>) -> Result<UsageState, String> {
    Ok(state.service.get_state().await)
}

#[tauri::command]
pub async fn refresh_all(state: State<'_, AppState>) -> Result<UsageState, String> {
    Ok(state.service.refresh_all(UsageFetchReason::Manual).await)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<TrayciSettings, String> {
    Ok(state.settings.lock().expect("settings lock").clone())
}

#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: TrayciSettingsPatch,
) -> Result<TrayciSettings, String> {
    let cleared_position = patch.window_position == Some(None);
    let previous = state.settings.lock().expect("settings lock").clone();
    let settings = state
        .repository
        .lock()
        .await
        .update(patch)
        .await
        .map_err(|error| error.to_string())?;
    *state.settings.lock().expect("settings lock") = settings.clone();
    if cleared_position {
        popover::reanchor(&app).map_err(|error| error.to_string())?;
    }
    popover::set_scale(&app, settings.font_scale).map_err(|error| error.to_string())?;
    autostart::set_enabled(settings.start_on_login)
        .await
        .map_err(|error| error.to_string())?;
    // Themes, text size and percentage style change nothing a provider would answer differently,
    // so they must not spend a request: toggling them used to refresh every provider.
    state
        .service
        .settings_changed(previous.providers != settings.providers)
        .await;
    Ok(settings)
}

/// The webview reports the start of a header drag; only then is a window move the user's.
#[tauri::command]
pub fn begin_popover_drag(app: AppHandle) {
    app.state::<popover::PopoverState>().set_dragging(true);
}

#[tauri::command]
pub fn hide_popover(app: AppHandle) -> Result<(), String> {
    popover::save_position(&app);
    app.get_webview_window(popover::LABEL)
        .map(|window| window.hide().map_err(|error| error.to_string()))
        .unwrap_or(Ok(()))
}

#[tauri::command]
pub fn resize_popover(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    if !width.is_finite() || !height.is_finite() {
        return Err("popover dimensions must be finite".into());
    }
    popover::resize(&app, width, height).map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn quit_app(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    state.service.stop().await;
    app.exit(0);
    Ok(())
}
