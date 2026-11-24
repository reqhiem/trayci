//! The tray icon: a left click toggles the popover, the menu shows it or quits.
use trayci_core::{presentation::display_percent, TrayciSettings};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::spawn;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::spawn;

/// "Trayci · Codex 40% · Claude 12%": each provider's tightest weekly window, the busiest first.
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
            weekly_used(snapshot).map(|used| {
                let percent = display_percent(used, settings.percentage_display);
                format!("{} {percent}%", snapshot.display_name)
            })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use trayci_core::{
        PercentageDisplay, ProviderUsageSnapshot, UsageSource, UsageState, UsageStatus, UsageWindow,
    };

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
            group: None,
        }
    }
}
