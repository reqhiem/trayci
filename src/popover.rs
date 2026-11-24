use crate::ui::{self, Bridge};
use slint::{
    winit_030::{
        winit::{
            dpi::{PhysicalPosition, PhysicalSize},
            event::WindowEvent,
        },
        EventResult, WinitWindowAccessor,
    },
    ComponentHandle,
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

/// Width of the master column: the popover grows rightwards from it.
const COLUMN_WIDTH: f64 = 360.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub position: PhysicalPosition<i32>,
    pub size: PhysicalSize<u32>,
}

impl Rect {
    fn contains(&self, (x, y): (f64, f64)) -> bool {
        let (left, top) = (f64::from(self.position.x), f64::from(self.position.y));
        (left..left + f64::from(self.size.width)).contains(&x)
            && (top..top + f64::from(self.size.height)).contains(&y)
    }
}

#[derive(Default)]
struct State {
    /// Where the popover opened from: the tray icon, or the pointer.
    anchor: Option<PhysicalPosition<f64>>,
    /// Where the user put the popover; sticky across sessions.
    custom: Option<PhysicalPosition<i32>>,
    blurred_at: Option<Instant>,
    /// Whether the popover has had focus since it was shown: X11 reports a focus-out as the
    /// window maps, before the focus-in, and only a later one means the user clicked away.
    focused: bool,
}

/// The popover window's placement and visibility. Cheap to clone and to send to another thread,
/// but every method must run on the Slint thread: post it there with `invoke_from_event_loop`.
#[derive(Clone)]
pub struct Popover {
    ui: slint::Weak<ui::Popover>,
    state: Arc<Mutex<State>>,
}

impl Popover {
    pub fn new(ui: &ui::Popover, custom: Option<(i32, i32)>) -> Self {
        let popover = Self {
            ui: ui.as_weak(),
            state: Arc::default(),
        };
        popover.state().custom = custom.map(|(x, y)| PhysicalPosition::new(x, y));
        let events = popover.clone();
        ui.window().on_winit_window_event(move |_, event| {
            // Deferred: hiding or moving the window from inside its own event dispatch re-enters
            // the backend.
            let popover = events.clone();
            match event {
                WindowEvent::Focused(true) => popover.state().focused = true,
                WindowEvent::Focused(false) if std::mem::take(&mut popover.state().focused) => {
                    slint::Timer::single_shot(Duration::ZERO, move || popover.blurred());
                }
                // The window follows its content's size, and growing taller from the tray icon
                // would run off a bottom panel's monitor.
                WindowEvent::Resized(_) => {
                    slint::Timer::single_shot(Duration::ZERO, move || popover.position());
                }
                _ => {}
            }
            EventResult::Propagate
        });
        popover
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("popover state lock")
    }

    pub fn toggle(&self, anchor: PhysicalPosition<f64>) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let blurred_at = self.state().blurred_at.take();
        if ui.window().is_visible() {
            return self.hide();
        }
        if hidden_by_this_click(blurred_at, Instant::now()) {
            return;
        }
        self.show(Some(anchor));
    }

    /// Opens at `anchor`, or at the pointer when there is none.
    pub fn show(&self, anchor: Option<PhysicalPosition<f64>>) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        if let Some(anchor) = anchor.or_else(platform::cursor_position) {
            self.state().anchor = Some(anchor);
        }
        self.position();
        if let Err(error) = ui.window().show() {
            eprintln!("trayci: could not show the popover: {error}");
            return;
        }
        // A window that was never mapped has no winit window to measure until now.
        self.position();
        ui.window()
            .with_winit_window(|window| window.focus_window());
    }

    pub fn hide(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        self.state().focused = false;
        if let Err(error) = ui.window().hide() {
            eprintln!("trayci: could not hide the popover: {error}");
        }
        ui.global::<Bridge>().invoke_reset_transient();
    }

    fn blurred(&self) {
        self.state().blurred_at = Some(Instant::now());
        self.hide();
    }

    /// Places the popover at a typed position, the manual counterpart of a drag (issue #39).
    ///
    /// The stored value is what was typed; `placement` clamps it to the work area on every use,
    /// the same way it does for a dragged one.
    pub fn place(&self, position: (i32, i32)) {
        self.state().custom = Some(PhysicalPosition::new(position.0, position.1));
        self.position();
    }

    /// Drops a saved position and snaps the popover back to the tray.
    pub fn reanchor(&self) {
        self.state().custom = None;
        self.position();
    }

    fn position(&self) {
        let Some(ui) = self.ui.upgrade() else {
            return;
        };
        let window = ui.window();
        let (anchor, custom) = {
            let state = self.state();
            (state.anchor, state.custom)
        };
        // Nothing to place against yet: leave it where the window manager puts it.
        let Some(anchor) = anchor.or(custom.map(|c| PhysicalPosition::new(c.x.into(), c.y.into())))
        else {
            return;
        };
        let point = work_area_point(anchor, custom);
        let Some(area) = platform::work_area(point).or_else(|| monitor_bounds(&ui, point)) else {
            return;
        };
        let size = window.size();
        // The master column, not the whole window, is what stays centred on the tray icon, so
        // opening the detail pane grows to the side instead of sliding the rows out from under
        // the pointer.
        let scale = f64::from(ui.global::<ui::Theme>().get_scale());
        let column = COLUMN_WIDTH * scale * f64::from(window.scale_factor());
        let (x, y) = placement(
            anchor,
            PhysicalSize::new(size.width, size.height),
            column,
            custom,
            area,
        );
        window.set_position(slint::PhysicalPosition::new(x, y));
    }
}

/// The bounds of the monitor holding `point`, for a platform that cannot name its work area.
fn monitor_bounds(ui: &ui::Popover, point: (f64, f64)) -> Option<Rect> {
    ui.window()
        .with_winit_window(|window| {
            let bounds = |monitor: slint::winit_030::winit::monitor::MonitorHandle| Rect {
                position: monitor.position(),
                size: monitor.size(),
            };
            window
                .available_monitors()
                .map(bounds)
                .find(|rect| rect.contains(point))
                .or_else(|| window.primary_monitor().map(bounds))
        })
        .flatten()
}

/// Whether a position the user sets survives. A Wayland compositor places toplevels itself, so
/// there the coordinates would be written to settings and never applied.
pub fn positions_are_real() -> bool {
    !cfg!(target_os = "linux")
        || std::env::var_os("WAYLAND_DISPLAY").is_none()
        || std::env::var("SLINT_BACKEND").is_ok_and(|backend| backend.ends_with("x11"))
}

/// On Windows, a tray press blurs (and so hides) the popover before the click reaches `toggle`.
// ponytail: fixed grace; a press held longer than this reopens the popover.
const BLUR_GRACE: Duration = Duration::from_millis(300);

fn hidden_by_this_click(blurred_at: Option<Instant>, now: Instant) -> bool {
    blurred_at.is_some_and(|at| now.saturating_duration_since(at) < BLUR_GRACE)
}

/// The point whose monitor decides which work area the popover is clamped to.
///
/// It has to be where the popover is going, not where the pointer is. Resolving the monitor from
/// the anchor clamps a position saved on one screen to the work area of another: opening from the
/// tray menu, where the pointer is always on the panel's monitor, dragged the popover off the
/// screen it had been left on (issue #42).
fn work_area_point(
    anchor: PhysicalPosition<f64>,
    custom: Option<PhysicalPosition<i32>>,
) -> (f64, f64) {
    custom.map_or((anchor.x, anchor.y), |custom| {
        (f64::from(custom.x), f64::from(custom.y))
    })
}

fn placement(
    anchor: PhysicalPosition<f64>,
    size: PhysicalSize<u32>,
    column: f64,
    custom: Option<PhysicalPosition<i32>>,
    area: Rect,
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

    fn area(x: i32, y: i32, width: u32, height: u32) -> Rect {
        Rect {
            position: PhysicalPosition::new(x, y),
            size: PhysicalSize::new(width, height),
        }
    }

    #[test]
    fn a_tray_click_that_blurred_the_popover_does_not_reopen_it() {
        let now = Instant::now();
        assert!(hidden_by_this_click(Some(now), now + BLUR_GRACE / 2));
        assert!(!hidden_by_this_click(Some(now), now + BLUR_GRACE));
        assert!(!hidden_by_this_click(None, now));
    }

    #[test]
    fn the_work_area_follows_the_saved_position_not_the_pointer() {
        let anchor = PhysicalPosition::new(1900.0, 1040.0);
        assert_eq!(work_area_point(anchor, None), (1900.0, 1040.0));
        assert_eq!(
            work_area_point(anchor, Some(PhysicalPosition::new(2496, 602))),
            (2496.0, 602.0),
            "a popover left on the second monitor is placed against that monitor's work area, \
             however far the tray icon is from it"
        );
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
                area(0, 0, 1920, 1080)
            ),
            (0, 13)
        );
        assert_eq!(
            placement(
                PhysicalPosition::new(1910.0, 1070.0),
                size,
                360.0,
                None,
                area(0, 0, 1920, 1080)
            ),
            (1560, 802)
        );
    }

    #[test]
    fn placement_clears_a_top_panel() {
        // A 40px top panel: the tray icon sits inside it, above the work area, so the popover has
        // no room above the anchor and drops below it, stopping at the panel's edge.
        assert_eq!(
            placement(
                PhysicalPosition::new(960.0, 20.0),
                PhysicalSize::new(360, 260),
                360.0,
                None,
                area(0, 40, 1920, 1040)
            ),
            (780, 40)
        );
    }

    #[test]
    fn placement_respects_side_panels() {
        let size = PhysicalSize::new(360, 260);
        // A 60px left panel: centring the column on the icon would start at -150.
        assert_eq!(
            placement(
                PhysicalPosition::new(30.0, 540.0),
                size,
                360.0,
                None,
                area(60, 0, 1860, 1080)
            ),
            (60, 272)
        );
        // A 60px right panel: the popover ends flush with the work area at 1500 + 360 = 1860.
        assert_eq!(
            placement(
                PhysicalPosition::new(1890.0, 540.0),
                size,
                360.0,
                None,
                area(0, 0, 1860, 1080)
            ),
            (1500, 272)
        );
    }

    #[test]
    fn placement_handles_negative_monitor_origins() {
        let size = PhysicalSize::new(360, 260);
        // A monitor left of the primary spans x -1920..0.
        assert_eq!(
            placement(
                PhysicalPosition::new(-1915.0, 540.0),
                size,
                360.0,
                None,
                area(-1920, 0, 1920, 1080)
            ),
            (-1920, 272)
        );
        assert_eq!(
            placement(
                PhysicalPosition::new(-10.0, 1070.0),
                size,
                360.0,
                None,
                area(-1920, 0, 1920, 1080)
            ),
            (-360, 802),
            "the right edge of a monitor at a negative origin is still its own edge"
        );
        // A monitor above the primary spans y -1080..0, with the tray icon at its top.
        assert_eq!(
            placement(
                PhysicalPosition::new(960.0, -1070.0),
                size,
                360.0,
                None,
                area(0, -1080, 1920, 1080)
            ),
            (780, -1062)
        );
    }

    #[test]
    fn placement_uses_the_work_area_of_the_anchored_monitor() {
        let anchor = PhysicalPosition::new(3180.0, 700.0);
        let size = PhysicalSize::new(360, 260);
        // A 1280x720 secondary right of the primary, with a 30px top panel of its own.
        assert_eq!(
            placement(anchor, size, 360.0, None, area(1920, 30, 1280, 690)),
            (2840, 432)
        );
        // The same anchor against the primary's work area lands nowhere near it, which is why the
        // caller has to resolve the monitor before asking for a placement.
        assert_eq!(
            placement(anchor, size, 360.0, None, area(0, 0, 1920, 1040)).0,
            1560
        );
    }

    #[test]
    fn placement_survives_a_work_area_smaller_than_the_popover() {
        let size = PhysicalSize::new(360, 260);
        assert_eq!(
            placement(
                PhysicalPosition::new(100.0, 140.0),
                size,
                360.0,
                None,
                area(0, 0, 200, 150)
            ),
            (0, 0)
        );
        // Clamping is to the work area's origin, not to zero.
        assert_eq!(
            placement(
                PhysicalPosition::new(150.0, 90.0),
                size,
                360.0,
                None,
                area(100, 50, 200, 150)
            ),
            (100, 50)
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
            area(0, 0, 1920, 1080),
        );
        let opened = placement(
            anchor,
            PhysicalSize::new(680, 260),
            360.0,
            None,
            area(0, 0, 1920, 1080),
        );
        let widest = placement(
            anchor,
            PhysicalSize::new(1040, 260),
            360.0,
            None,
            area(0, 0, 1920, 1080),
        );
        assert_eq!(closed, (780, 792));
        assert_eq!(opened.0, closed.0, "the detail pane grows to the right");
        assert_eq!(widest.0, closed.0, "and keeps growing to the right");
    }

    #[test]
    fn placement_clamps_a_dragged_popover_back_on_screen() {
        assert_eq!(
            placement(
                PhysicalPosition::new(960.0, 1060.0),
                PhysicalSize::new(680, 260),
                360.0,
                Some(PhysicalPosition::new(1800, 900)),
                area(0, 0, 1920, 1080)
            ),
            (1240, 820)
        );
    }

    #[test]
    fn placement_keeps_a_dragged_position_across_resizes() {
        let anchor = PhysicalPosition::new(960.0, 1060.0);
        let dragged = Some(PhysicalPosition::new(1200, 400));
        let work = area(0, 0, 1920, 1080);
        assert_eq!(
            placement(anchor, PhysicalSize::new(360, 260), 360.0, dragged, work),
            (1200, 400)
        );
        assert_eq!(
            placement(anchor, PhysicalSize::new(680, 260), 360.0, dragged, work),
            (1200, 400),
            "opening the detail pane leaves the dragged corner alone"
        );
        // Until growing rightwards would run off the work area, which slides it back on.
        assert_eq!(
            placement(
                anchor,
                PhysicalSize::new(680, 260),
                360.0,
                Some(PhysicalPosition::new(1500, 400)),
                work
            ),
            (1240, 400)
        );
    }
}
