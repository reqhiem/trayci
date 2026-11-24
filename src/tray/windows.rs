use crate::popover::Popover;
use slint::winit_030::winit::dpi::PhysicalPosition;
use tokio::{runtime::Handle, sync::watch};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem},
    Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
};

/// Registers the tray icon; its tooltip follows `tooltips` while the sender lives.
pub fn spawn(_runtime: &Handle, popover: Popover, mut tooltips: watch::Receiver<String>) {
    // tray-icon's window needs the message pump winit runs, and the icon is not Send: it is built
    // on the Slint thread once the loop polls this, and lives in it.
    let spawned = slint::spawn_local(async move {
        let show = MenuItem::new("Show Trayci", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let menu = Menu::new();
        if let Err(error) = menu.append_items(&[&show, &quit]) {
            eprintln!("trayci: no tray menu: {error}");
        }
        // 1 is the icon build.rs embeds in the exe.
        let tray = Icon::from_resource(1, None)
            .map_err(|error| error.to_string())
            .and_then(|icon| {
                TrayIconBuilder::new()
                    .with_icon(icon)
                    .with_tooltip(tooltips.borrow_and_update().as_str())
                    .with_menu(Box::new(menu))
                    .with_menu_on_left_click(false)
                    .build()
                    .map_err(|error| error.to_string())
            });
        let tray = match tray {
            Ok(tray) => tray,
            Err(error) => {
                eprintln!("trayci: no tray icon: {error}");
                return;
            }
        };

        // Both handlers run inside the tray window's procedure, so the popover acts from the loop.
        TrayIconEvent::set_event_handler(Some({
            let popover = popover.clone();
            move |event| {
                let TrayIconEvent::Click {
                    position,
                    rect,
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                else {
                    return;
                };
                let anchor = if rect.size.width > 0 && rect.size.height > 0 {
                    PhysicalPosition::new(
                        rect.position.x + f64::from(rect.size.width) / 2.0,
                        rect.position.y + f64::from(rect.size.height) / 2.0,
                    )
                } else {
                    PhysicalPosition::new(position.x, position.y)
                };
                let popover = popover.clone();
                let _ = slint::invoke_from_event_loop(move || popover.toggle(Some(anchor)));
            }
        }));
        let (show, quit) = (show.into_id(), quit.into_id());
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if event.id == show {
                let popover = popover.clone();
                let _ = slint::invoke_from_event_loop(move || popover.show(None));
            } else if event.id == quit {
                let _ = slint::quit_event_loop();
            }
        }));

        while tooltips.changed().await.is_ok() {
            let tooltip = tooltips.borrow_and_update().clone();
            if let Err(error) = tray.set_tooltip(Some(tooltip)) {
                eprintln!("trayci: tray tooltip: {error}");
            }
        }
    });
    if let Err(error) = spawned {
        eprintln!("trayci: no tray icon: {error}");
    }
}
