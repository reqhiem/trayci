//! Ctrl+Alt+U toggles the popover at the pointer, from anywhere.
use crate::popover::Popover;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};

/// Registers the shortcut for as long as the returned manager lives. Call it on the thread that
/// runs the event loop: on Windows the hotkey arrives through that thread's message queue.
pub fn register(popover: Popover) -> Option<GlobalHotKeyManager> {
    // A grab through XWayland only sees keys typed into X11 windows, so on Wayland there is no
    // shortcut at all rather than one that works only sometimes.
    #[cfg(target_os = "linux")]
    if !crate::popover::is_x11() {
        return None;
    }
    let shortcut = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyU);
    let manager = GlobalHotKeyManager::new()
        .and_then(|manager| manager.register(shortcut).map(|()| manager))
        .map_err(|error| eprintln!("trayci: no Ctrl+Alt+U shortcut: {error}"))
        .ok()?;
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state == HotKeyState::Pressed {
            let popover = popover.clone();
            let _ = slint::invoke_from_event_loop(move || popover.toggle(None));
        }
    }));
    Some(manager)
}
