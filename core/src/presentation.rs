//! What the popover and the terminal view say about a reading, in #66's vocabulary: labels,
//! countdowns, severity and order. One module, so the two views cannot drift apart.

use crate::{
    model::{PercentageDisplay, ProviderUsageSnapshot, UsageSource, UsageStatus, UsageWindow},
    providers::antigravity::pool_name,
    service::STALE_AFTER_MS,
};

/// Where a provider sits in the order the user arranged, for sorting. Ids the order does not
/// mention rank equal and last, so they keep whatever order the caller sorts by next, and an empty
/// order ranks everything equal, which leaves the usage ordering untouched.
pub fn provider_rank(order: &[String], id: &str) -> usize {
    order.iter().position(|o| o == id).unwrap_or(order.len())
}

/// The user's `providerOrder`, then the most used first.
pub fn ordered<'a>(
    snapshots: impl IntoIterator<Item = &'a ProviderUsageSnapshot>,
    order: &[String],
) -> Vec<&'a ProviderUsageSnapshot> {
    let most = |snapshot: &ProviderUsageSnapshot| {
        snapshot
            .windows
            .iter()
            .map(|window| window.used_percent)
            .fold(0.0, f64::max)
    };
    let mut sorted = snapshots.into_iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| {
        provider_rank(order, &a.provider)
            .cmp(&provider_rank(order, &b.provider))
            .then(most(b).total_cmp(&most(a)))
    });
    sorted
}

/// The most constrained window; the first of equals.
pub fn tightest(snapshot: &ProviderUsageSnapshot) -> Option<&UsageWindow> {
    snapshot.windows.iter().reduce(|tightest, window| {
        if window.used_percent > tightest.used_percent {
            window
        } else {
            tightest
        }
    })
}

/// Minutes as the coarsest unit that still reads at a glance: `45m`, `3h 54m`, `5d 1h`.
pub fn format_minutes(minutes: u64) -> String {
    match minutes {
        0..=59 => format!("{minutes}m"),
        60..=1439 => format!("{}h {}m", minutes / 60, minutes % 60),
        _ => format!("{}d {}h", minutes / 1440, minutes % 1440 / 60),
    }
}

/// Time until `resets_at` at a glance: `45m`, `3h 54m`, `5d 1h`, or `now`.
pub fn countdown(resets_at: u64, now: u64) -> String {
    match resets_at.saturating_sub(now) / 60_000 {
        0 => "now".into(),
        minutes => format_minutes(minutes),
    }
}

/// `resets in 3h 54m`, `resets now`, or nothing when the reset is unknown.
pub fn format_reset(resets_at: Option<u64>, now: u64) -> String {
    match resets_at.map(|resets_at| countdown(resets_at, now)) {
        None => String::new(),
        Some(left) if left == "now" => "resets now".into(),
        Some(left) => format!("resets in {left}"),
    }
}

pub fn format_age(updated_at: u64, now: u64) -> String {
    match now.saturating_sub(updated_at) / 60_000 {
        0 => "Updated just now".into(),
        minutes => format!("Updated {} ago", format_minutes(minutes)),
    }
}

/// Where a reading came from, so a scraped or cached number is not mistaken for the API's.
pub fn source_label(source: UsageSource) -> &'static str {
    match source {
        UsageSource::Oauth | UsageSource::Api => "via API",
        UsageSource::Rpc => "via RPC",
        UsageSource::Cli => "via CLI",
        UsageSource::Cache => "from cache",
    }
}

/// Status records age only as of loading, so a reading that aged while the view stayed open
/// counts as stale too.
pub fn is_stale(snapshot: &ProviderUsageSnapshot, now: u64) -> bool {
    snapshot.status == UsageStatus::Stale
        || now.saturating_sub(snapshot.updated_at) > STALE_AFTER_MS
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Summary {
    pub text: String,
    pub stale: bool,
}

/// What a provider row shows beside its name: how old a stale reading is ("Updated 2h ago"), or
/// the bare countdown to the tightest window's reset ("3h 54m"), which the detailed row reads as
/// "resets in 3h 54m". A row with no windows says nothing here and shows its error instead.
///
/// The status field only records how old a snapshot was when it was loaded, so age is re-checked
/// here: an aged reading must not render a healthy countdown just because it loaded as `ok` (#54).
pub fn status_summary(snapshot: &ProviderUsageSnapshot, now: u64) -> Option<Summary> {
    let reset = tightest(snapshot).and_then(|window| window.resets_at);
    let summary = |text, stale| Some(Summary { text, stale });
    match reset {
        _ if snapshot.status == UsageStatus::Stale => {
            summary(format_age(snapshot.updated_at, now), true)
        }
        Some(at) if !is_stale(snapshot, now) => summary(countdown(at, now), false),
        _ if !snapshot.windows.is_empty() => summary(format_age(snapshot.updated_at, now), true),
        _ => None,
    }
}

/// The window's length. An ungrouped window is keyed on its label, because Claude's per-model
/// weekly windows ("Fable") share the weekly duration and must keep their name. A pooled window is
/// keyed on its duration; any other length is its label without the pool's name in front.
fn cadence(window: &UsageWindow, weekly: &str) -> String {
    let Some(group) = &window.group else {
        return if window.label == "Weekly" {
            weekly.into()
        } else {
            window.label.clone()
        };
    };
    match window.duration_minutes {
        Some(300) => "5h".into(),
        Some(10_080) => weekly.into(),
        _ => {
            let pool = pool_name(&group.name);
            match window.label.strip_prefix(&format!("{pool} ")) {
                Some(rest) => rest.into(),
                None if window.label == pool => String::new(),
                None => window.label.clone(),
            }
        }
    }
}

fn labelled(window: &UsageWindow, weekly: &str, separator: &str) -> String {
    let length = cadence(window, weekly);
    match &window.group {
        Some(group) if length.is_empty() => pool_name(&group.name),
        Some(group) => format!("{}{separator}{length}", pool_name(&group.name)),
        None => length,
    }
}

/// A meter in a provider row: "Gemini 5h", "Claude/GPT wk", "Gemini 4 Argon wk", "5h", "Fable".
pub fn row_label(window: &UsageWindow) -> String {
    labelled(window, "wk", " ")
}

/// An entry in the detail pane: "Gemini · Weekly", "Claude/GPT · 5h", "Weekly", "Fable".
pub fn detail_label(window: &UsageWindow) -> String {
    labelled(window, "Weekly", " · ")
}

/// The number a meter shows: an integer, used or remaining per the setting. `round` rather than
/// `{:.0}`, which rounds half to even.
pub fn display_percent(used_percent: f64, mode: PercentageDisplay) -> u8 {
    let used = used_percent.clamp(0.0, 100.0);
    let shown = match mode {
        PercentageDisplay::Used => used,
        PercentageDisplay::Remaining => 100.0 - used,
    };
    shown.round() as u8
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Low,
    Medium,
    High,
    Critical,
}

/// How much attention a meter needs, at the notification thresholds (50/85/90). Always by use, and
/// on the rounded number, so a window shown as "90%" is red even at 89.6%.
pub fn severity(used_percent: f64) -> Severity {
    match used_percent.round() {
        used if used >= 90.0 => Severity::Critical,
        used if used >= 85.0 => Severity::High,
        used if used >= 50.0 => Severity::Medium,
        _ => Severity::Low,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UsageGroup;

    const MINUTE: u64 = 60_000;

    fn meter(id: &str, duration: Option<u64>, group: Option<&str>, label: &str) -> UsageWindow {
        UsageWindow {
            id: id.into(),
            label: label.into(),
            used_percent: 0.0,
            duration_minutes: duration,
            resets_at: None,
            reset_description: None,
            group: group.map(|name| UsageGroup {
                name: name.into(),
                models: None,
            }),
        }
    }

    fn snapshot(windows: Vec<UsageWindow>) -> ProviderUsageSnapshot {
        ProviderUsageSnapshot {
            provider: "claude".into(),
            display_name: "Claude".into(),
            status: UsageStatus::Ok,
            plan: None,
            windows,
            updated_at: 0,
            source: Some(UsageSource::Oauth),
            error: None,
        }
    }

    fn claude() -> ProviderUsageSnapshot {
        let mut session = meter("session", Some(300), None, "5h");
        session.used_percent = 20.0;
        let mut weekly = meter("weekly", Some(10_080), None, "Weekly");
        weekly.used_percent = 70.0;
        snapshot(vec![session, weekly])
    }

    /// Labels as antigravity.rs builds them: the pool's name and window, or the pool alone.
    fn antigravity() -> Vec<UsageWindow> {
        let gemini = Some("Gemini Models");
        let third_party = Some("Claude and GPT models");
        vec![
            meter("gemini-5h", Some(300), gemini, "Gemini 5h"),
            meter("gemini-weekly", Some(10_080), gemini, "Gemini Weekly"),
            meter("3p-5h", Some(300), third_party, "Claude/GPT 5h"),
            meter("3p-weekly", Some(10_080), third_party, "Claude/GPT Weekly"),
            meter(
                "gemini-argon-wtu-weekly",
                Some(10_080),
                Some("Gemini 4 Argon"),
                "Gemini 4 Argon",
            ),
        ]
    }

    /// Claude's per-model limit shares the weekly length but keeps its own name.
    fn claude_windows() -> Vec<UsageWindow> {
        vec![
            meter("session", Some(300), None, "5h"),
            meter("weekly", Some(10_080), None, "Weekly"),
            meter("fable-weekly", Some(10_080), None, "Fable"),
        ]
    }

    fn labels(windows: &[UsageWindow], label: fn(&UsageWindow) -> String) -> Vec<String> {
        windows.iter().map(label).collect()
    }

    #[test]
    fn tightest_is_the_most_used_window() {
        assert_eq!(
            tightest(&claude()).map(|window| window.id.as_str()),
            Some("weekly")
        );
        assert_eq!(tightest(&snapshot(vec![])), None);
    }

    #[test]
    fn countdown_is_shared_by_every_view() {
        assert_eq!(countdown(0, 0), "now");
        assert_eq!(countdown(12 * MINUTE, 0), "12m");
        assert_eq!(countdown((26 * 60 + 5) * MINUTE, 0), "1d 2h");
    }

    #[test]
    fn a_stale_reading_reports_its_age_instead_of_its_reset() {
        let stale = ProviderUsageSnapshot {
            status: UsageStatus::Stale,
            ..claude()
        };
        assert_eq!(
            status_summary(&stale, 7 * MINUTE),
            Some(Summary {
                text: "Updated 7m ago".into(),
                stale: true
            })
        );
        assert_eq!(format_age(0, 0), "Updated just now");
    }

    #[test]
    fn a_current_reading_counts_down_to_the_tightest_reset() {
        let mut fresh = claude();
        fresh
            .windows
            .iter_mut()
            .for_each(|window| window.resets_at = Some(90 * MINUTE));
        assert_eq!(
            status_summary(&fresh, 0),
            Some(Summary {
                text: "1h 30m".into(),
                stale: false
            })
        );
        assert_eq!(status_summary(&snapshot(vec![]), 0), None);
    }

    #[test]
    fn a_reading_that_loaded_as_ok_ages_out_when_not_refreshed() {
        // The bug in #54: a snapshot the cache marked `ok` days ago kept rendering a healthy
        // countdown, because only the load-time status field was ever consulted.
        let two_days = 2 * 24 * 60 * MINUTE;
        let mut aged = claude();
        aged.windows
            .iter_mut()
            .for_each(|window| window.resets_at = Some(two_days + 90 * MINUTE));
        assert_eq!(
            status_summary(&aged, two_days),
            Some(Summary {
                text: "Updated 2d 0h ago".into(),
                stale: true
            })
        );
    }

    /// The sort the provider list runs: the configured order first, usage breaking the rest.
    fn sorted(order: &[&str], usage: &[(&str, f64)]) -> Vec<String> {
        let order = order.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        let snapshots = usage
            .iter()
            .map(|(id, used)| {
                let mut window = meter("weekly", Some(10_080), None, "Weekly");
                window.used_percent = *used;
                ProviderUsageSnapshot {
                    provider: id.to_string(),
                    ..snapshot(vec![window])
                }
            })
            .collect::<Vec<_>>();
        ordered(&snapshots, &order)
            .into_iter()
            .map(|snapshot| snapshot.provider.clone())
            .collect()
    }

    const USAGE: [(&str, f64); 3] = [("claude", 10.0), ("codex", 90.0), ("antigravity", 50.0)];

    #[test]
    fn no_order_leaves_the_usage_ordering_alone() {
        assert_eq!(sorted(&[], &USAGE), ["codex", "antigravity", "claude"]);
    }

    #[test]
    fn the_configured_order_comes_before_usage() {
        assert_eq!(
            sorted(&["claude", "antigravity", "codex"], &USAGE),
            ["claude", "antigravity", "codex"]
        );
    }

    #[test]
    fn providers_the_order_does_not_mention_follow_it_by_usage() {
        assert_eq!(provider_rank(&["claude".into()], "plugin:new"), 1);
        let usage = [("claude", 10.0), ("codex", 5.0), ("plugin:new", 80.0)];
        assert_eq!(
            sorted(&["codex"], &usage),
            ["codex", "plugin:new", "claude"]
        );
    }

    // The vocabulary table in issue #66, shared with the TUI (#67) and the Figma library (#68).

    #[test]
    fn pools_are_named_in_plain_words() {
        assert_eq!(pool_name("Gemini Models"), "Gemini");
        assert_eq!(pool_name("Claude and GPT models"), "Claude/GPT");
        assert_eq!(pool_name("Gemini 4 Argon"), "Gemini 4 Argon");
        assert_eq!(pool_name("Nova models"), "Nova");
    }

    #[test]
    fn row_meters_are_labelled_by_pool_and_cadence_or_by_cadence_alone() {
        assert_eq!(
            labels(&antigravity(), row_label),
            [
                "Gemini 5h",
                "Gemini wk",
                "Claude/GPT 5h",
                "Claude/GPT wk",
                "Gemini 4 Argon wk"
            ]
        );
        assert_eq!(labels(&claude_windows(), row_label), ["5h", "wk", "Fable"]);
        let daily = meter("x", Some(1440), Some("Gemini Models"), "Gemini Daily");
        assert_eq!(row_label(&daily), "Gemini Daily");
    }

    #[test]
    fn detail_entries_spell_the_cadence_out() {
        assert_eq!(
            labels(&antigravity(), detail_label),
            [
                "Gemini · 5h",
                "Gemini · Weekly",
                "Claude/GPT · 5h",
                "Claude/GPT · Weekly",
                "Gemini 4 Argon · Weekly"
            ]
        );
        assert_eq!(
            labels(&claude_windows(), detail_label),
            ["5h", "Weekly", "Fable"]
        );
    }

    #[test]
    fn a_pooled_window_of_any_other_length_keeps_its_name() {
        let nova = Some("Nova models");
        let hourly = meter("nova-1h", Some(60), nova, "Nova 1h");
        assert_eq!(
            [row_label(&hourly), detail_label(&hourly)],
            ["Nova 1h", "Nova · 1h"]
        );
        let lone = meter("nova", None, nova, "Nova");
        assert_eq!([row_label(&lone), detail_label(&lone)], ["Nova", "Nova"]);
        let odd = meter("odd", Some(60), nova, "Burst");
        assert_eq!(
            [row_label(&odd), detail_label(&odd)],
            ["Nova Burst", "Nova · Burst"]
        );
    }

    #[test]
    fn countdowns_and_ages_read_at_a_glance() {
        let at = |minutes: u64| countdown(minutes * MINUTE, 0);
        assert_eq!(
            [at(45), at(3 * 60 + 54), at((5 * 24 + 1) * 60 + 30), at(0)],
            ["45m", "3h 54m", "5d 1h", "now"]
        );
        assert_eq!(countdown(30_000, 0), "now");
        assert_eq!(
            countdown(0, MINUTE),
            "now",
            "a reset in the past is due now"
        );
        assert_eq!(format_reset(Some(234 * MINUTE), 0), "resets in 3h 54m");
        assert_eq!(format_reset(Some(0), 0), "resets now");
        assert_eq!(format_reset(None, 0), "");
        assert_eq!(format_age(0, 2 * MINUTE + 59_000), "Updated 2m ago");
        assert_eq!(format_age(0, 30_000), "Updated just now");
    }

    #[test]
    fn percent_is_an_integer_used_or_remaining() {
        assert_eq!(display_percent(17.4, PercentageDisplay::Used), 17);
        assert_eq!(display_percent(17.4, PercentageDisplay::Remaining), 83);
        assert_eq!(display_percent(140.0, PercentageDisplay::Remaining), 0);
        assert_eq!(
            display_percent(2.5, PercentageDisplay::Used),
            3,
            "half rounds up, as the popover always did"
        );
    }

    #[test]
    fn severity_is_by_use_at_the_notification_thresholds_on_the_rounded_number() {
        use Severity::*;
        let cases = [0.0, 49.0, 49.4, 49.5, 50.0, 84.0, 85.0, 89.0, 89.6, 100.0];
        assert_eq!(
            cases.map(severity),
            [Low, Low, Low, Medium, Medium, Medium, High, High, Critical, Critical]
        );
    }

    #[test]
    fn sources_name_where_a_reading_came_from() {
        assert_eq!(source_label(UsageSource::Oauth), "via API");
        assert_eq!(source_label(UsageSource::Cache), "from cache");
    }
}
