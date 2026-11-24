//! Where Windows says the popover may go. winit makes the process per-monitor DPI aware, so these
//! are physical pixels, the same space as the tray icon's rect.
use super::Rect;
use slint::winit_030::winit::dpi::{PhysicalPosition, PhysicalSize};
use windows_sys::Win32::{
    Foundation::POINT,
    Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST},
    UI::{
        Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON},
        WindowsAndMessaging::{GetCursorPos, GetSystemMetrics, SM_SWAPBUTTON},
    },
};

/// The work area of the monitor holding `point`: the monitor minus its taskbar (issue #42).
pub fn work_area((x, y): (f64, f64)) -> Option<Rect> {
    let point = POINT {
        x: x.round() as i32,
        y: y.round() as i32,
    };
    let mut info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: plain Win32 calls; `info` carries the size GetMonitorInfoW checks.
    let found =
        unsafe { GetMonitorInfoW(MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST), &mut info) };
    let work = info.rcWork;
    (found != 0).then(|| Rect {
        position: PhysicalPosition::new(work.left, work.top),
        size: PhysicalSize::new(
            work.right.saturating_sub(work.left).max(0) as u32,
            work.bottom.saturating_sub(work.top).max(0) as u32,
        ),
    })
}

/// The pointer, in screen pixels: where the popover opens when nothing else anchors it.
pub fn cursor_position() -> Option<PhysicalPosition<f64>> {
    let mut point = POINT::default();
    // SAFETY: plain Win32 call writing into a local.
    let found = unsafe { GetCursorPos(&mut point) };
    (found != 0).then(|| PhysicalPosition::new(point.x.into(), point.y.into()))
}

/// Whether the primary button is down. GetAsyncKeyState reads the physical buttons, so with the
/// buttons swapped the primary one is the right.
pub fn button_held() -> Option<bool> {
    // SAFETY: plain Win32 calls.
    let primary = if unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0 {
        VK_RBUTTON
    } else {
        VK_LBUTTON
    };
    // SAFETY: as above.
    let state = unsafe { GetAsyncKeyState(i32::from(primary)) };
    // The top bit, the sign, is whether the button is down now.
    Some(state < 0)
}
