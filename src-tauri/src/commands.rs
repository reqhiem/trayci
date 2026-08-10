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
    state: State<'_, AppState>,
    patch: TrayciSettingsPatch,
) -> Result<TrayciSettings, String> {
    let settings = state
        .repository
        .lock()
        .await
        .update(patch)
        .await
        .map_err(|error| error.to_string())?;
    *state.settings.lock().expect("settings lock") = settings.clone();
    autostart::set_enabled(settings.start_on_login)
        .await
        .map_err(|error| error.to_string())?;
    state.service.settings_changed().await;
    Ok(settings)
}

#[tauri::command]
pub fn hide_popover(app: AppHandle) -> Result<(), String> {
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
