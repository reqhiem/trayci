use tauri::{
    image::Image,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    PhysicalPosition,
};
#[cfg(target_os = "linux")]
use tauri::{
    menu::{Menu, MenuItem},
    Manager,
};
use trayci_core::{PercentageDisplay, TrayciSettings};

pub const ID: &str = "trayci";

pub fn create(app: &tauri::AppHandle) -> tauri::Result<()> {
    let icon = Image::from_bytes(include_bytes!("../../resources/icons/32x32.png"))?;
    let builder = TrayIconBuilder::with_id(ID)
        .icon(icon)
        .tooltip("Trayci")
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                position,
                rect,
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let size = rect.size.to_physical::<u32>(1.0);
                let origin = rect.position.to_physical::<f64>(1.0);
                let center = (size.width > 0 && size.height > 0).then(|| {
                    PhysicalPosition::new(
                        origin.x + f64::from(size.width) / 2.0,
                        origin.y + f64::from(size.height) / 2.0,
                    )
                });
                let _ = crate::popover::toggle(tray.app_handle(), position, center);
            }
        });

    #[cfg(target_os = "linux")]
    let builder = {
        // AppIndicator needs a menu to display the icon and does not emit tray click events.
        let show = MenuItem::with_id(app, "show", "Show Trayci", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&show, &quit])?;
        builder
            .menu(&menu)
            .on_menu_event(|app, event| match event.id().as_ref() {
                "show" => {
                    if let Some(window) = app.get_webview_window(crate::popover::LABEL) {
                        let position = window.cursor_position().unwrap_or_default();
                        let _ = crate::popover::show(app, position, None);
                    }
                }
                "quit" => app.exit(0),
                _ => {}
            })
    };

    builder.build(app)?;
    Ok(())
}

pub fn tooltip(state: &trayci_core::UsageState, settings: &TrayciSettings) -> String {
    let mut providers = state.providers.values().collect::<Vec<_>>();
    providers.sort_by(|left, right| {
        weekly_used(right)
            .unwrap_or(0.0)
            .total_cmp(&weekly_used(left).unwrap_or(0.0))
            .then_with(|| left.display_name.cmp(&right.display_name))
    });
    let summary = providers
        .into_iter()
        .filter_map(|snapshot| {
            weekly_used(snapshot)
                .map(|used| display_percent(used, settings.percentage_display))
                .map(|percent| format!("{} {:.0}%", snapshot.display_name, percent))
        })
        .collect::<Vec<_>>();
    if summary.is_empty() {
        "Trayci".into()
    } else {
        format!("Trayci · {}", summary.join(" · "))
    }
}

fn weekly_used(snapshot: &trayci_core::ProviderUsageSnapshot) -> Option<f64> {
    snapshot
        .windows
        .iter()
        .filter(|window| window.duration_minutes == Some(7 * 24 * 60) || window.id == "weekly")
        .map(|window| window.used_percent)
        .reduce(f64::max)
}

fn display_percent(used: f64, mode: PercentageDisplay) -> f64 {
    match mode {
        PercentageDisplay::Used => used,
        PercentageDisplay::Remaining => 100.0 - used,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use trayci_core::{ProviderUsageSnapshot, UsageSource, UsageState, UsageStatus, UsageWindow};

    #[test]
    fn tooltip_uses_the_tightest_weekly_percentage() {
        let snapshot = ProviderUsageSnapshot {
            provider: "codex".into(),
            display_name: "Codex".into(),
            status: UsageStatus::Ok,
            plan: None,
            windows: vec![
                window("five-hour", 90.0, Some(300)),
                window("weekly", 25.0, Some(10_080)),
                window("model-weekly", 40.0, Some(10_080)),
            ],
            updated_at: 0,
            source: Some(UsageSource::Api),
            error: None,
        };
        let state = UsageState {
            providers: HashMap::from([("codex".into(), snapshot)]),
            ..Default::default()
        };
        let mut settings = TrayciSettings::default();

        assert_eq!(tooltip(&state, &settings), "Trayci · Codex 40%");
        settings.percentage_display = PercentageDisplay::Remaining;
        assert_eq!(tooltip(&state, &settings), "Trayci · Codex 60%");
    }

    fn window(id: &str, used_percent: f64, duration_minutes: Option<u64>) -> UsageWindow {
        UsageWindow {
            id: id.into(),
            label: id.into(),
            used_percent,
            duration_minutes,
            resets_at: None,
            reset_description: None,
        }
    }
}
