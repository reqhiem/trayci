//! Where X11 says the popover may go, and the focus request muffin honours. winit keeps its X
//! connection to itself, so this opens one of its own: window ids are the server's, not a
//! connection's.
use super::Rect;
use slint::winit_030::winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};
use std::sync::OnceLock;
use x11rb::{
    connection::Connection,
    protocol::{
        xproto::{
            AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask,
            KeyButMask, PropMode, QueryPointerReply, WindowClass,
        },
        Event,
    },
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_ACTIVE_WINDOW,
        _NET_CURRENT_DESKTOP,
        _NET_WM_STATE,
        _NET_WM_STATE_ABOVE,
        _NET_WM_STATE_SKIP_PAGER,
        _NET_WM_STATE_SKIP_TASKBAR,
        _NET_WM_USER_TIME,
        _NET_WORKAREA,
    }
}

struct X11 {
    connection: RustConnection,
    root: u32,
    /// An unmapped window of our own: changing a property on it makes the server tell the time.
    clock: u32,
    atoms: Atoms,
}

/// Whether winit runs on X11. The same test winit makes, so the XWayland of a Wayland session is
/// never taken for the display the popover is on.
pub fn is_x11() -> bool {
    let set = |name| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    !set("WAYLAND_DISPLAY") && !set("WAYLAND_SOCKET") && set("DISPLAY")
}

fn x11() -> Option<&'static X11> {
    static X11: OnceLock<Option<X11>> = OnceLock::new();
    X11.get_or_init(|| {
        if !is_x11() {
            return None;
        }
        connect()
            .map_err(|error| eprintln!("trayci: no X11 connection of its own: {error}"))
            .ok()
    })
    .as_ref()
}

fn connect() -> Result<X11, Box<dyn std::error::Error>> {
    let (connection, screen) = x11rb::connect(None)?;
    let root = connection.setup().roots[screen].root;
    let atoms = Atoms::new(&connection)?.reply()?;
    let clock = connection.generate_id()?;
    connection.create_window(
        0,
        clock,
        root,
        0,
        0,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )?;
    Ok(X11 {
        connection,
        root,
        clock,
        atoms,
    })
}

/// The work area of the current desktop: the screen minus its panels (issue #42). It spans every
/// monitor; the caller intersects it with the one the popover goes to, as GDK did.
pub fn work_area(_point: (f64, f64)) -> Option<Rect> {
    let x11 = x11()?;
    let desktop = x11
        .cardinals(x11.atoms._NET_CURRENT_DESKTOP)
        .and_then(|desktop| desktop.first().copied())
        .unwrap_or(0) as usize;
    let areas = x11.cardinals(x11.atoms._NET_WORKAREA)?;
    let [x, y, width, height] = *areas.get(desktop * 4..desktop * 4 + 4)? else {
        return None;
    };
    Some(Rect {
        position: PhysicalPosition::new(x as i32, y as i32),
        size: PhysicalSize::new(width, height),
    })
}

/// The pointer, in screen pixels: where the popover opens when nothing else anchors it.
pub fn cursor_position() -> Option<PhysicalPosition<f64>> {
    pointer().map(|pointer| PhysicalPosition::new(pointer.root_x.into(), pointer.root_y.into()))
}

/// Whether the primary button is down, or `None` where the server cannot be asked.
pub fn button_held() -> Option<bool> {
    pointer().map(|pointer| pointer.mask.contains(KeyButMask::BUTTON1))
}

fn pointer() -> Option<QueryPointerReply> {
    let x11 = x11()?;
    x11.connection.query_pointer(x11.root).ok()?.reply().ok()
}

/// Keeps the just-mapped popover above other windows and out of the taskbar, and gives it the
/// keyboard. `None` when the window is not on X11.
///
/// Both have to wait for the map. winit asks for the window level while the window is still
/// unmapped, and a window manager ignores state requests for those, then drops the states again
/// each time the window is hidden. winit's `focus_window` sends `CurrentTime`, which muffin cannot
/// tie to a user interaction: it refuses it and only flags the window as demanding attention, so
/// the popover opened unfocused and neither Escape nor clicking away closed it (issue #38). The
/// server's time is newer than any interaction the window manager has recorded, so the same
/// request stamped with it is honoured.
pub fn present(window: &Window) -> Option<()> {
    let RawWindowHandle::Xlib(handle) = window.window_handle().ok()?.as_raw() else {
        return None;
    };
    let window = u32::try_from(handle.window).ok()?;
    let x11 = x11()?;
    let atoms = &x11.atoms;
    let time = x11.server_time()?;
    const ADD: u32 = 1;
    const FROM_APPLICATION: u32 = 1;
    let send = |kind, data: [u32; 5]| {
        x11.connection.send_event(
            false,
            x11.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            ClientMessageEvent::new(32, window, kind, data),
        )
    };
    send(
        atoms._NET_WM_STATE,
        [
            ADD,
            atoms._NET_WM_STATE_ABOVE,
            atoms._NET_WM_STATE_SKIP_TASKBAR,
            FROM_APPLICATION,
            0,
        ],
    )
    .ok()?;
    send(
        atoms._NET_WM_STATE,
        [ADD, atoms._NET_WM_STATE_SKIP_PAGER, 0, FROM_APPLICATION, 0],
    )
    .ok()?;
    x11.connection
        .change_property32(
            PropMode::REPLACE,
            window,
            atoms._NET_WM_USER_TIME,
            AtomEnum::CARDINAL,
            &[time],
        )
        .ok()?;
    send(atoms._NET_ACTIVE_WINDOW, [FROM_APPLICATION, time, 0, 0, 0]).ok()?;
    x11.connection.flush().ok()
}

impl X11 {
    fn cardinals(&self, property: u32) -> Option<Vec<u32>> {
        let reply = self
            .connection
            .get_property(false, self.root, property, AtomEnum::CARDINAL, 0, 1024)
            .ok()?
            .reply()
            .ok()?;
        let values = reply.value32()?.collect();
        Some(values)
    }

    /// What `gdk_x11_get_server_time` does: the PropertyNotify for a change carries the clock.
    fn server_time(&self) -> Option<u32> {
        self.connection
            .change_property8(
                PropMode::REPLACE,
                self.clock,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                b"t",
            )
            .ok()?;
        self.connection.flush().ok()?;
        loop {
            match self.connection.wait_for_event().ok()? {
                Event::PropertyNotify(event) if event.window == self.clock => {
                    return Some(event.time)
                }
                // The change failed, so no notify is coming.
                Event::Error(_) => return None,
                _ => {}
            }
        }
    }
}
