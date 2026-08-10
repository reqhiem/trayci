use std::sync::Mutex;
use tauri::{
    LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};

pub const LABEL: &str = "popover";
pub const MIN_WIDTH: f64 = 340.0;
pub const MAX_WIDTH: f64 = 720.0;
pub const MIN_HEIGHT: f64 = 70.0;
pub const MAX_HEIGHT: f64 = 620.0;

#[derive(Default)]
pub struct PopoverState {
    anchor: Mutex<Option<PhysicalPosition<f64>>>,
    pub custom_position: Mutex<Option<PhysicalPosition<i32>>>,
}

pub fn create(app: &tauri::AppHandle) -> tauri::Result<WebviewWindow> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("Trayci")
        .inner_size(360.0, 140.0)
        .min_inner_size(MIN_WIDTH, MIN_HEIGHT)
        .max_inner_size(MAX_WIDTH, MAX_HEIGHT)
        .decorations(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(false)
        .build()?;

    let hide = window.clone();
    let app_handle = app.clone();
    window.on_window_event(move |event| match event {
        WindowEvent::Focused(false) => {
            let _ = hide.hide();
        }
        WindowEvent::Moved(pos) => {
            if hide.is_visible().unwrap_or(false) {
                let state = app_handle.state::<PopoverState>();
                *state.custom_position.lock().expect("popover pos lock") = Some(*pos);
                if let Some(app_state) = app_handle.try_state::<crate::app::AppState>() {
                    let repository = app_state.repository.clone();
                    let patch = trayci_core::TrayciSettingsPatch {
                        window_position: Some(Some((pos.x, pos.y))),
                        ..Default::default()
                    };
                    tauri::async_runtime::spawn(async move {
                        let mut repo = repository.lock().await;
                        let _ = repo.update(patch).await;
                    });
                }
            }
        }
        _ => {}
    });
    Ok(window)
}

pub fn toggle(
    app: &tauri::AppHandle,
    position: PhysicalPosition<f64>,
    tray_center: Option<PhysicalPosition<f64>>,
) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(LABEL) else {
        return Ok(());
    };
    if window.is_visible()? {
        return window.hide();
    }
    show(app, position, tray_center)
}

pub fn show(
    app: &tauri::AppHandle,
    position: PhysicalPosition<f64>,
    tray_center: Option<PhysicalPosition<f64>>,
) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(LABEL) else {
        return Ok(());
    };
    let state = app.state::<PopoverState>();
    {
        let mut anchor = state.anchor.lock().expect("popover anchor lock");
        *anchor = tray_center.or(Some(position));
    }
    position_window(&window, &state)?;
    window.show()?;
    window.set_focus()
}

pub fn resize(app: &tauri::AppHandle, width: f64, height: f64) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(LABEL) else {
        return Ok(());
    };
    window.set_size(LogicalSize::new(
        width.clamp(MIN_WIDTH, MAX_WIDTH),
        height.clamp(MIN_HEIGHT, MAX_HEIGHT),
    ))?;
    position_window(&window, &app.state::<PopoverState>())
}

fn position_window(window: &WebviewWindow, state: &PopoverState) -> tauri::Result<()> {
    if let Some(pos) = *state.custom_position.lock().expect("popover custom_position lock") {
        return window.set_position(pos);
    }
    let anchor = state
        .anchor
        .lock()
        .expect("popover anchor lock")
        .unwrap_or(window.cursor_position()?);
    let monitor = window
        .monitor_from_point(anchor.x, anchor.y)?
        .or(window.current_monitor()?)
        .or(window.primary_monitor()?);
    let Some(monitor) = monitor else {
        return Ok(());
    };
    let area = monitor.work_area();
    let size = window.outer_size()?;
    let left = area.position.x;
    let top = area.position.y;
    let right = left + area.size.width as i32;
    let bottom = top + area.size.height as i32;
    let (x, y) = placement(anchor, size.width, size.height, left, top, right, bottom);
    window.set_position(PhysicalPosition::new(x, y))
}

fn placement(
    anchor: PhysicalPosition<f64>,
    width: u32,
    height: u32,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) -> (i32, i32) {
    let x =
        (anchor.x.round() as i32 - width as i32 / 2).clamp(left, (right - width as i32).max(left));
    let above = anchor.y.round() as i32 - height as i32 - 8;
    let y = if above >= top {
        above
    } else {
        (anchor.y.round() as i32 + 8).min((bottom - height as i32).max(top))
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placement_stays_in_work_area() {
        assert_eq!(
            placement(PhysicalPosition::new(5.0, 5.0), 360, 260, 0, 0, 1920, 1080),
            (0, 13)
        );
        assert_eq!(
            placement(
                PhysicalPosition::new(1910.0, 1070.0),
                360,
                260,
                0,
                0,
                1920,
                1080
            ),
            (1560, 802)
        );
    }
}
