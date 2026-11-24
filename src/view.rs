//! What the Slint views show, built from the service's state and the settings, and the way back
//! from an edited settings view to a patch.
use crate::ui::{MeterLine, MeterView, ProviderSetting, ProviderView, SettingsView, Severity};
use slint::{Model, ModelRc, SharedString, VecModel};
use trayci_core::{
    presentation, BuiltInProviderSettingsPatch, DisplayMode, NotificationSettingsPatch,
    PercentageDisplay, ProviderSettings, ProviderSettingsPatch, ProviderUsageSnapshot,
    TrayciSettings, TrayciSettingsPatch, UsageState, UsageStatus, UsageWindow,
};

/// Characters; "Claude/GPT wk" still fits half a provider row at any text size. A longer label
/// takes the whole line instead of being cut.
const WIDE_LABEL: usize = 14;

const BUILT_IN: [(&str, &str); 3] = [
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("antigravity", "Antigravity"),
];

/// Every provider, in the user's order and then by use.
pub fn providers(state: &UsageState, settings: &TrayciSettings, now: u64) -> Vec<ProviderView> {
    presentation::ordered(state.providers.values(), &settings.provider_order)
        .into_iter()
        .map(|snapshot| provider(snapshot, settings.percentage_display, now))
        .collect()
}

fn provider(snapshot: &ProviderUsageSnapshot, mode: PercentageDisplay, now: u64) -> ProviderView {
    let meter = |window| meter(window, mode, now);
    let meters = snapshot.windows.iter().map(meter).collect::<Vec<_>>();
    let summary = presentation::status_summary(snapshot, now);
    let tightest = presentation::tightest(snapshot);
    let message = match (snapshot.windows.is_empty(), snapshot.status) {
        (false, _) => "",
        (true, UsageStatus::Fetching) => "Loading…",
        (true, _) => snapshot.error.as_deref().unwrap_or("Usage unavailable"),
    };
    ProviderView {
        id: snapshot.provider.as_str().into(),
        name: snapshot.display_name.as_str().into(),
        plan: capitalized(snapshot.plan.as_deref().unwrap_or_default()).into(),
        loading: snapshot.status == UsageStatus::Fetching,
        failed: snapshot.windows.is_empty(),
        message: message.into(),
        summary_stale: summary.as_ref().is_some_and(|summary| summary.stale),
        summary: summary
            .map(|summary| summary.text)
            .unwrap_or_default()
            .into(),
        lines: model(lines(&meters).into_iter()),
        meters: model(meters.into_iter()),
        has_tightest: tightest.is_some(),
        tightest: tightest.map(meter).unwrap_or_default(),
        age: presentation::format_age(snapshot.updated_at, now).into(),
        source: snapshot
            .source
            .map(presentation::source_label)
            .unwrap_or_default()
            .into(),
        error: snapshot.error.as_deref().unwrap_or_default().into(),
    }
}

/// Two meters a line, left then right, so the bars of a column line up. A wide meter takes a line
/// of its own instead of being cut.
fn lines(meters: &[MeterView]) -> Vec<MeterLine> {
    let mut lines: Vec<MeterLine> = Vec::new();
    for meter in meters {
        match lines.last_mut() {
            Some(line) if !line.has_right && !line.left.wide && !meter.wide => {
                line.right = meter.clone();
                line.has_right = true;
            }
            _ => lines.push(MeterLine {
                left: meter.clone(),
                ..Default::default()
            }),
        }
    }
    lines
}

/// "free" → "Free", as the webview's `text-transform: capitalize` showed a plan.
fn capitalized(text: &str) -> String {
    text.split(' ')
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        })
        .collect::<Vec<String>>()
        .join(" ")
}

fn meter(window: &UsageWindow, mode: PercentageDisplay, now: u64) -> MeterView {
    let row_label = presentation::row_label(window);
    MeterView {
        id: window.id.as_str().into(),
        wide: row_label.chars().count() > WIDE_LABEL,
        row_label: row_label.into(),
        detail_label: presentation::detail_label(window).into(),
        used: window.used_percent.clamp(0.0, 100.0) as f32,
        percent: format!(
            "{}%",
            presentation::display_percent(window.used_percent, mode)
        )
        .into(),
        severity: match presentation::severity(window.used_percent) {
            presentation::Severity::Low => Severity::Low,
            presentation::Severity::Medium => Severity::Medium,
            presentation::Severity::High => Severity::High,
            presentation::Severity::Critical => Severity::Critical,
        },
        resets: presentation::format_reset(window.resets_at, now).into(),
        models: window
            .group
            .as_ref()
            .and_then(|group| group.models.as_deref())
            .unwrap_or_default()
            .into(),
    }
}

fn model<T: Clone + 'static>(items: impl Iterator<Item = T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items.collect::<Vec<_>>()))
}

fn theme_name(theme: trayci_core::Theme) -> &'static str {
    match theme {
        trayci_core::Theme::System => "system",
        trayci_core::Theme::Light => "light",
        trayci_core::Theme::Dark => "dark",
    }
}

fn provider_settings<'a>(settings: &'a TrayciSettings, id: &str) -> Option<&'a ProviderSettings> {
    match id {
        "claude" => Some(&settings.providers.claude),
        "codex" => Some(&settings.providers.codex),
        "antigravity" => Some(&settings.providers.antigravity),
        _ => None,
    }
}

/// The built-in providers in the user's order; the ones it does not mention keep theirs.
fn provider_order(settings: &TrayciSettings) -> Vec<(&'static str, &'static str)> {
    let mut order = BUILT_IN.to_vec();
    order.sort_by_key(|(id, _)| presentation::provider_rank(&settings.provider_order, id));
    order
}

pub fn settings(settings: &TrayciSettings, positions_real: bool) -> SettingsView {
    let notifications = &settings.notifications;
    let (position_x, position_y) = settings.window_position.unwrap_or_default();
    SettingsView {
        start_on_login: settings.start_on_login,
        refresh_on_resume: settings.refresh_on_resume,
        poll_minutes: i32::try_from(settings.poll_interval_minutes).unwrap_or(i32::MAX),
        theme: theme_name(settings.theme).into(),
        font_scale: settings.font_scale as f32,
        show_remaining: settings.percentage_display == PercentageDisplay::Remaining,
        compact: settings.display_mode == DisplayMode::Compact,
        notify_50: notifications.quota50,
        notify_85: notifications.quota85,
        notify_90: notifications.quota90,
        notify_reset: notifications.reset,
        providers: model(
            provider_order(settings)
                .into_iter()
                .map(|(id, label)| ProviderSetting {
                    id: id.into(),
                    label: label.into(),
                    enabled: provider_settings(settings, id)
                        .is_some_and(|provider| provider.enabled),
                }),
        ),
        has_position: settings.window_position.is_some(),
        position_x,
        position_y,
        positions_real,
    }
}

/// `Some(edited)` when it differs from `current`.
fn changed<T: PartialEq>(current: T, edited: T) -> Option<T> {
    (current != edited).then_some(edited)
}

/// What the user changed in `edited`, the settings view they sent back. The window position is
/// left out: it only moves through `set-position` and `reset-position`.
pub fn patch(current: &TrayciSettings, edited: &SettingsView) -> TrayciSettingsPatch {
    let notifications = NotificationSettingsPatch {
        quota50: changed(current.notifications.quota50, edited.notify_50),
        quota85: changed(current.notifications.quota85, edited.notify_85),
        quota90: changed(current.notifications.quota90, edited.notify_90),
        reset: changed(current.notifications.reset, edited.notify_reset),
    };
    let notifications_changed = [
        notifications.quota50,
        notifications.quota85,
        notifications.quota90,
        notifications.reset,
    ]
    .iter()
    .any(Option::is_some);

    let edited_providers = edited.providers.iter().collect::<Vec<_>>();
    let enabled = |id: &str| {
        let current = provider_settings(current, id)?.enabled;
        let edited = edited_providers.iter().find(|provider| provider.id == id)?;
        changed(current, edited.enabled).map(|enabled| ProviderSettingsPatch {
            enabled: Some(enabled),
            executable_path: None,
        })
    };
    let providers = BuiltInProviderSettingsPatch {
        claude: enabled("claude"),
        codex: enabled("codex"),
        antigravity: enabled("antigravity"),
    };
    let providers_changed =
        providers.claude.is_some() || providers.codex.is_some() || providers.antigravity.is_some();

    // An empty order sorts by use. Sending back the order the view showed must not pin it.
    let shown = provider_order(current)
        .into_iter()
        .map(|(id, _)| SharedString::from(id))
        .collect::<Vec<_>>();
    let order = edited_providers
        .iter()
        .map(|provider| provider.id.clone())
        .collect::<Vec<_>>();

    TrayciSettingsPatch {
        start_on_login: changed(current.start_on_login, edited.start_on_login),
        refresh_on_resume: changed(current.refresh_on_resume, edited.refresh_on_resume),
        poll_interval_minutes: u64::try_from(edited.poll_minutes)
            .ok()
            .and_then(|minutes| changed(current.poll_interval_minutes, minutes)),
        display_mode: changed(
            current.display_mode,
            if edited.compact {
                DisplayMode::Compact
            } else {
                DisplayMode::Detailed
            },
        ),
        percentage_display: changed(
            current.percentage_display,
            if edited.show_remaining {
                PercentageDisplay::Remaining
            } else {
                PercentageDisplay::Used
            },
        ),
        notifications: notifications_changed.then_some(notifications),
        providers: providers_changed.then_some(providers),
        provider_order: (order != shown).then(|| order.iter().map(ToString::to_string).collect()),
        window_position: None,
        theme: [
            trayci_core::Theme::System,
            trayci_core::Theme::Light,
            trayci_core::Theme::Dark,
        ]
        .into_iter()
        .find(|theme| edited.theme == theme_name(*theme))
        .and_then(|theme| changed(current.theme, theme)),
        // Through f32 1.15 comes back as 1.1499999761581420: compare and store at the precision
        // the setting has.
        font_scale: changed(
            (current.font_scale * 100.0).round(),
            (f64::from(edited.font_scale) * 100.0).round(),
        )
        .map(|scale| scale / 100.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use trayci_core::{UsageGroup, UsageSource};

    const NOW: u64 = 1_800_000_000_000;
    const MINUTE: u64 = 60_000;

    fn window(id: &str, label: &str, used_percent: f64) -> UsageWindow {
        UsageWindow {
            id: id.into(),
            label: label.into(),
            used_percent,
            duration_minutes: Some(10_080),
            resets_at: Some(NOW + 234 * MINUTE),
            reset_description: None,
            group: None,
        }
    }

    fn snapshot(id: &str, windows: Vec<UsageWindow>) -> ProviderUsageSnapshot {
        ProviderUsageSnapshot {
            provider: id.into(),
            display_name: id.into(),
            status: UsageStatus::Ok,
            plan: None,
            windows,
            updated_at: NOW,
            source: Some(UsageSource::Oauth),
            error: None,
        }
    }

    fn state(snapshots: Vec<ProviderUsageSnapshot>) -> UsageState {
        UsageState {
            providers: snapshots
                .into_iter()
                .map(|snapshot| (snapshot.provider.clone(), snapshot))
                .collect::<HashMap<_, _>>(),
            ..Default::default()
        }
    }

    fn ids(views: &[ProviderView]) -> Vec<String> {
        views.iter().map(|view| view.id.to_string()).collect()
    }

    #[test]
    fn providers_follow_the_user_order_then_usage() {
        let usage = state(vec![
            snapshot("claude", vec![window("weekly", "Weekly", 10.0)]),
            snapshot("codex", vec![window("weekly", "Weekly", 90.0)]),
            snapshot("antigravity", vec![window("weekly", "Weekly", 50.0)]),
        ]);
        let mut settings = TrayciSettings::default();
        assert_eq!(
            ids(&providers(&usage, &settings, NOW)),
            ["codex", "antigravity", "claude"]
        );
        settings.provider_order = vec!["claude".into()];
        assert_eq!(
            ids(&providers(&usage, &settings, NOW)),
            ["claude", "codex", "antigravity"]
        );
    }

    #[test]
    fn a_provider_view_is_formatted_for_the_user() {
        let mut gemini = window("gemini-weekly", "Gemini Weekly", 89.6);
        gemini.group = Some(UsageGroup {
            name: "Gemini Models".into(),
            models: Some("Gemini Flash, Gemini Pro".into()),
        });
        let mut long = window("x", "Gemini 4 Argon", 10.0);
        long.group = Some(UsageGroup {
            name: "Gemini 4 Argon".into(),
            models: None,
        });
        let mut antigravity = snapshot("antigravity", vec![gemini, long]);
        antigravity.plan = Some("pro plus".into());
        let usage = state(vec![antigravity]);
        let settings = TrayciSettings {
            percentage_display: PercentageDisplay::Remaining,
            ..Default::default()
        };
        let view = &providers(&usage, &settings, NOW)[0];
        assert_eq!(view.plan, "Pro Plus");
        assert_eq!(view.summary, "3h 54m");
        assert!(!view.summary_stale);
        assert_eq!(view.source, "via API");
        assert_eq!(view.message, "");
        let meter = view.meters.row_data(0).expect("first meter");
        assert_eq!(meter.row_label, "Gemini wk");
        assert_eq!(meter.detail_label, "Gemini · Weekly");
        assert_eq!(meter.percent, "10%", "remaining, rounded from 10.4");
        assert_eq!(meter.severity, Severity::Critical, "by use, rounded to 90");
        assert_eq!(meter.resets, "resets in 3h 54m");
        assert_eq!(meter.models, "Gemini Flash, Gemini Pro");
        assert!(!meter.wide);
        assert!(view.meters.row_data(1).is_some_and(|meter| meter.wide));
        assert!(view.has_tightest);
        assert_eq!(view.tightest.id, "gemini-weekly");
    }

    #[test]
    fn meters_pair_up_unless_one_is_wide() {
        let meter = |id: &str, wide| MeterView {
            id: id.into(),
            wide,
            ..Default::default()
        };
        let meters = [
            meter("a", false),
            meter("b", false),
            meter("wide", true),
            meter("c", false),
            meter("d", false),
            meter("e", false),
        ];
        let shape = lines(&meters)
            .iter()
            .map(|line| {
                (
                    line.left.id.to_string(),
                    line.has_right.then(|| line.right.id.to_string()),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            shape,
            [
                ("a".into(), Some("b".into())),
                ("wide".into(), None),
                ("c".into(), Some("d".into())),
                ("e".into(), None),
            ]
        );
    }

    #[test]
    fn a_provider_without_windows_says_why() {
        let loading = ProviderUsageSnapshot {
            status: UsageStatus::Fetching,
            ..snapshot("codex", vec![])
        };
        let failed = ProviderUsageSnapshot {
            status: UsageStatus::Error,
            error: Some("Not signed in".into()),
            ..snapshot("claude", vec![])
        };
        let views = providers(
            &state(vec![loading, failed]),
            &TrayciSettings::default(),
            NOW,
        );
        let message = |id: &str| {
            views
                .iter()
                .find(|view| view.id == id)
                .map(|view| (view.failed, view.loading, view.message.to_string()))
        };
        assert_eq!(message("codex"), Some((true, true, "Loading…".into())));
        assert_eq!(
            message("claude"),
            Some((true, false, "Not signed in".into()))
        );
    }

    #[test]
    fn sending_the_settings_back_unchanged_patches_nothing() {
        let current = TrayciSettings {
            font_scale: 1.15,
            ..Default::default()
        };
        let patch = patch(&current, &settings(&current, true));
        assert_eq!(
            format!("{patch:?}"),
            format!("{:?}", TrayciSettingsPatch::default())
        );
    }

    #[test]
    fn each_edit_becomes_its_own_field_of_the_patch() {
        let current = TrayciSettings::default();
        let mut edited = settings(&current, true);
        edited.compact = true;
        edited.show_remaining = true;
        edited.notify_85 = true;
        edited.theme = "light".into();
        edited.font_scale = 1.3;
        edited.poll_minutes = 30;
        let patch = patch(&current, &edited);
        assert_eq!(patch.display_mode, Some(DisplayMode::Compact));
        assert_eq!(patch.percentage_display, Some(PercentageDisplay::Remaining));
        assert_eq!(
            patch.notifications.as_ref().map(|n| (n.quota50, n.quota85)),
            Some((None, Some(true)))
        );
        assert_eq!(patch.theme, Some(trayci_core::Theme::Light));
        assert_eq!(patch.font_scale, Some(1.3));
        assert_eq!(patch.poll_interval_minutes, Some(30));
        assert_eq!(patch.start_on_login, None);
        assert!(patch.providers.is_none());
        assert!(patch.provider_order.is_none());
        assert!(patch.window_position.is_none());
    }

    #[test]
    fn an_edit_made_before_the_last_save_shows_keeps_that_save() {
        let shown = TrayciSettings::default();
        let mut first = settings(&shown, true);
        first.theme = "light".into();
        let mut saved = shown.clone();
        trayci_core::settings::merge(&mut saved, patch(&shown, &first)).expect("valid patch");

        // Made from the view that still shows the old theme.
        let mut second = settings(&shown, true);
        second.font_scale = 1.3;
        // Against the saved settings, the old theme the view still showed looks like an edit.
        assert_eq!(patch(&saved, &second).theme, Some(shown.theme));
        let edit = patch(&shown, &second);
        assert_eq!(edit.theme, None);
        trayci_core::settings::merge(&mut saved, edit).expect("valid patch");
        assert_eq!(saved.theme, trayci_core::Theme::Light);
        assert_eq!(saved.font_scale, 1.3);
    }

    #[test]
    fn provider_switches_and_order_patch_only_when_moved() {
        let current = TrayciSettings::default();
        let mut edited = settings(&current, true);
        let mut rows = edited.providers.iter().collect::<Vec<_>>();
        rows[1].enabled = false;
        edited.providers = model(rows.clone().into_iter());
        let switched = patch(&current, &edited);
        assert_eq!(
            switched
                .providers
                .as_ref()
                .map(|p| (p.claude.is_some(), p.codex.as_ref().and_then(|c| c.enabled))),
            Some((false, Some(false)))
        );
        assert!(
            switched.provider_order.is_none(),
            "the shown order is not a choice"
        );

        rows.reverse();
        edited.providers = model(rows.into_iter());
        assert_eq!(
            patch(&current, &edited).provider_order,
            Some(vec![
                "antigravity".to_string(),
                "codex".into(),
                "claude".into()
            ])
        );
    }
}
