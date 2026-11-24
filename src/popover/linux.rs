//! Where X11 says the popover may go. W3 reads `_NET_WORKAREA` and the pointer through the X
//! server; until then the popover falls back to the monitor's bounds and its last anchor.
use super::Rect;
use slint::winit_030::winit::dpi::PhysicalPosition;

/// The work area of the monitor holding `point`: the monitor minus its panels (issue #42).
pub fn work_area(_point: (f64, f64)) -> Option<Rect> {
    None
}

/// The pointer, in screen pixels: where the popover opens when nothing else anchors it.
pub fn cursor_position() -> Option<PhysicalPosition<f64>> {
    None
}
