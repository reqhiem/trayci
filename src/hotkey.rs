//! Ctrl+Alt+U toggles the popover at the pointer, from anywhere.
use crate::popover::Popover;
use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};

/// Registered while the returned manager lives. Event loop thread only (Windows message queue).
pub fn register(popover: Popover) -> Option<GlobalHotKeyManager> {
    // An XWayland grab only sees keys typed into X11 windows: none beats a flaky shortcut.
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
        if event.id == shortcut.id() && event.state == HotKeyState::Pressed {
            let popover = popover.clone();
            let _ = slint::invoke_from_event_loop(move || popover.toggle(None));
        }
    }));
    Some(manager)
}
