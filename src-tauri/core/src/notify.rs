use crate::model::{NotificationSettings, UsageState, UsageStatus, UsageWindow};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationRequest {
    pub title: String,
    pub body: String,
}

#[derive(Clone)]
struct WindowState {
    used_percent: f64,
    resets_at: Option<u64>,
    notified: HashSet<u8>,
}

#[derive(Default)]
pub struct QuotaNotifier {
    windows: HashMap<String, WindowState>,
}

impl QuotaNotifier {
    pub fn update(
        &mut self,
        state: &UsageState,
        settings: &NotificationSettings,
        now: u64,
    ) -> Vec<NotificationRequest> {
        let mut requests = Vec::new();
        for snapshot in state
            .providers
            .values()
            .filter(|snapshot| matches!(snapshot.status, UsageStatus::Ok | UsageStatus::Stale))
        {
            for window in &snapshot.windows {
                let key = format!("{}:{}", snapshot.provider, window.id);
                let previous = self.windows.get(&key);
                if snapshot.status == UsageStatus::Stale {
                    self.windows.insert(
                        key,
                        WindowState {
                            used_percent: window.used_percent,
                            resets_at: window.resets_at,
                            notified: thresholds_at(window.used_percent).collect(),
                        },
                    );
                    continue;
                }

                let reset = previous.is_some_and(|old| did_reset(old, window, now));
                let mut notified = if reset {
                    HashSet::new()
                } else {
                    previous.map(|old| old.notified.clone()).unwrap_or_default()
                };
                if reset && settings.reset {
                    requests.push(NotificationRequest {
                        title: format!("{} quota reset", snapshot.display_name),
                        body: format!("{} quota is available again.", window.label),
                    });
                }

                let reached = thresholds_at(window.used_percent)
                    .filter(|threshold| {
                        enabled(settings, *threshold) && !notified.contains(threshold)
                    })
                    .collect::<Vec<_>>();
                notified.extend(reached.iter().copied());
                let reached = reached.last().copied();
                if let Some(threshold) = reached {
                    requests.push(NotificationRequest {
                        title: format!("{} quota at {threshold}%", snapshot.display_name),
                        body: format!("{} has used {:.0}%.", window.label, window.used_percent),
                    });
                }
                self.windows.insert(
                    key,
                    WindowState {
                        used_percent: window.used_percent,
                        resets_at: window.resets_at,
                        notified,
                    },
                );
            }
        }
        requests
    }
}

fn thresholds_at(used_percent: f64) -> impl Iterator<Item = u8> {
    [50, 85, 90]
        .into_iter()
        .filter(move |threshold| used_percent >= f64::from(*threshold))
}

fn enabled(settings: &NotificationSettings, threshold: u8) -> bool {
    match threshold {
        50 => settings.quota50,
        85 => settings.quota85,
        90 => settings.quota90,
        _ => false,
    }
}

fn did_reset(previous: &WindowState, current: &UsageWindow, now: u64) -> bool {
    matches!((previous.resets_at, current.resets_at), (Some(old), Some(new)) if old <= now && new > old)
        || (current.used_percent <= 5.0 && current.used_percent < previous.used_percent)
}
