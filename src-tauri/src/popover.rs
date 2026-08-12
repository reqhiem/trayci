use std::sync::Mutex;
use tauri::{
    LogicalSize, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

pub const LABEL: &str = "popover";
pub const MIN_WIDTH: f64 = 340.0;
/// Width of the master column: the popover grows rightwards from it.
pub const COLUMN_WIDTH: f64 = 360.0;
pub const MAX_WIDTH: f64 = 1200.0;
pub const MIN_HEIGHT: f64 = 70.0;
pub const MAX_HEIGHT: f64 = 1000.0;

pub struct PopoverState {
    anchor: Mutex<Option<PhysicalPosition<f64>>>,
    /// Where the user dragged the popover; sticky across sessions.
    custom_position: Mutex<Option<PhysicalPosition<i32>>>,
    /// Set while the user drags the header, so window-manager moves are not mistaken for one.
    dragging: Mutex<bool>,
    /// A drag waiting to be written to settings.
    pending: Mutex<Option<PhysicalPosition<i32>>>,
    scale: Mutex<f64>,
}

impl Default for PopoverState {
    fn default() -> Self {
        Self {
            anchor: Mutex::new(None),
            custom_position: Mutex::new(None),
            dragging: Mutex::new(false),
            pending: Mutex::new(None),
            scale: Mutex::new(1.0),
        }
    }
}

impl PopoverState {
    pub fn set_custom_position(&self, position: Option<(i32, i32)>) {
        *self.custom_position.lock().expect("popover position lock") =
            position.map(|(x, y)| PhysicalPosition::new(x, y));
    }

    pub fn set_dragging(&self, dragging: bool) {
        *self.dragging.lock().expect("popover dragging lock") = dragging;
    }

    fn take_pending(&self) -> Option<PhysicalPosition<i32>> {
        self.pending.lock().expect("popover pending lock").take()
    }
}

/// GTK reports a Wayland toplevel as sitting at its surface origin and cannot move it back, so a
/// move reported there is neither the position the user sees nor one we could ever restore. Storing
/// it would park the popover in a corner the first time the settings are read on an X11 session.
fn positions_are_real() -> bool {
    if !cfg!(target_os = "linux") {
        return true;
    }
    std::env::var("GDK_BACKEND").is_ok_and(|backend| backend == "x11")
        || std::env::var_os("WAYLAND_DISPLAY").is_none()
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
            save_position(&app_handle);
            let _ = hide.hide();
        }
        WindowEvent::Moved(position) => {
            let state = app_handle.state::<PopoverState>();
            if !*state.dragging.lock().expect("popover dragging lock") || !positions_are_real() {
                return;
            }
            state.set_custom_position(Some((position.x, position.y)));
            *state.pending.lock().expect("popover pending lock") = Some(*position);
        }
        _ => {}
    });
    Ok(window)
}

/// Writes a dragged position to settings. Called when the popover closes, not while it moves.
pub fn save_position(app: &tauri::AppHandle) {
    let Some(position) = app.state::<PopoverState>().take_pending() else {
        return;
    };
    let Some(app_state) = app.try_state::<crate::app::AppState>() else {
        return;
    };
    let patch = trayci_core::TrayciSettingsPatch {
        window_position: Some(Some((position.x, position.y))),
        ..Default::default()
    };
    tauri::async_runtime::block_on(async {
        if let Ok(updated) = app_state.repository.lock().await.update(patch).await {
            *app_state.settings.lock().expect("settings lock") = updated;
        }
    });
}

/// Drops a dragged position and snaps the popover back to the tray.
pub fn reanchor(app: &tauri::AppHandle) -> tauri::Result<()> {
    let state = app.state::<PopoverState>();
    state.set_custom_position(None);
    let _ = state.take_pending();
    match app.get_webview_window(LABEL) {
        Some(window) => position_window(&window, &state),
        None => Ok(()),
    }
}

pub fn set_scale(app: &tauri::AppHandle, scale: f64) -> tauri::Result<()> {
    let state = app.state::<PopoverState>();
    *state.scale.lock().expect("popover scale lock") = scale;
    match app.get_webview_window(LABEL) {
        Some(window) => window.set_zoom(scale),
        None => Ok(()),
    }
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
        save_position(app);
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
    // An unmapped window reports no size, so place it again now that the compositor sized it.
    position_window(&window, &state)?;
    window.set_focus()
}

pub fn resize(app: &tauri::AppHandle, width: f64, height: f64) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(LABEL) else {
        return Ok(());
    };
    let state = app.state::<PopoverState>();
    let scale = *state.scale.lock().expect("popover scale lock");
    window.set_size(LogicalSize::new(
        (width * scale).clamp(MIN_WIDTH, MAX_WIDTH),
        (height * scale).clamp(MIN_HEIGHT, MAX_HEIGHT),
    ))?;
    position_window(&window, &state)
}

fn position_window(window: &WebviewWindow, state: &PopoverState) -> tauri::Result<()> {
    let custom = *state.custom_position.lock().expect("popover position lock");
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
    // The master column, not the whole window, is what stays centred on the tray icon, so opening
    // the detail pane grows to the side instead of sliding the rows out from under the pointer.
    let column = COLUMN_WIDTH * *state.scale.lock().expect("popover scale lock");
    let (x, y) = placement(
        anchor,
        window.outer_size()?,
        column * monitor.scale_factor(),
        custom,
        *monitor.work_area(),
    );
    state.set_dragging(false);
    window.set_position(PhysicalPosition::new(x, y))
}

fn placement(
    anchor: PhysicalPosition<f64>,
    size: PhysicalSize<u32>,
    column: f64,
    custom: Option<PhysicalPosition<i32>>,
    area: tauri::PhysicalRect<i32, u32>,
) -> (i32, i32) {
    let left = area.position.x;
    let top = area.position.y;
    let fit = |value: i32, length: u32, min: i32, max: i32| {
        value.clamp(min, (max - length as i32).max(min))
    };
    let right = left + area.size.width as i32;
    let bottom = top + area.size.height as i32;
    if let Some(dragged) = custom {
        return (
            fit(dragged.x, size.width, left, right),
            fit(dragged.y, size.height, top, bottom),
        );
    }
    let x = fit(
        anchor.x.round() as i32 - (column.min(size.width as f64) / 2.0) as i32,
        size.width,
        left,
        right,
    );
    let above = anchor.y.round() as i32 - size.height as i32 - 8;
    let y = if above >= top {
        above
    } else {
        fit(anchor.y.round() as i32 + 8, size.height, top, bottom)
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(width: u32, height: u32) -> tauri::PhysicalRect<i32, u32> {
        tauri::PhysicalRect {
            position: PhysicalPosition::new(0, 0),
            size: PhysicalSize::new(width, height),
        }
    }

    #[test]
    fn placement_stays_in_work_area() {
        let size = PhysicalSize::new(360, 260);
        assert_eq!(
            placement(
                PhysicalPosition::new(5.0, 5.0),
                size,
                360.0,
                None,
                area(1920, 1080)
            ),
            (0, 13)
        );
        assert_eq!(
            placement(
                PhysicalPosition::new(1910.0, 1070.0),
                size,
                360.0,
                None,
                area(1920, 1080)
            ),
            (1560, 802)
        );
    }

    #[test]
    fn placement_centres_the_master_column_not_the_detail_pane() {
        let anchor = PhysicalPosition::new(960.0, 1060.0);
        let closed = placement(
            anchor,
            PhysicalSize::new(360, 260),
            360.0,
            None,
            area(1920, 1080),
        );
        let opened = placement(
            anchor,
            PhysicalSize::new(680, 260),
            360.0,
            None,
            area(1920, 1080),
        );
        assert_eq!(closed, (780, 792));
        assert_eq!(opened.0, closed.0, "the detail pane grows to the right");
    }

    #[test]
    fn placement_clamps_a_dragged_popover_back_on_screen() {
        assert_eq!(
            placement(
                PhysicalPosition::new(960.0, 1060.0),
                PhysicalSize::new(680, 260),
                360.0,
                Some(PhysicalPosition::new(1800, 900)),
                area(1920, 1080)
            ),
            (1240, 820)
        );
    }
}
