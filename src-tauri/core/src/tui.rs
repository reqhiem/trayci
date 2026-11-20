//! `trayci` in a terminal (#67): the popover's numbers in #66's vocabulary. Paints from the cache,
//! fetches the enabled providers once, then again on `r` and at most every `pollIntervalMinutes`.
//! Read-only: it never writes `cache.json` or the settings.

use crate::{
    cli::{countdown, format_minutes, format_reset, now_ms},
    model::*,
    providers::{abort_all_children, antigravity::pool_name},
    service::{empty_snapshot, enabled_provider_ids, failed_snapshot, STALE_AFTER_MS},
    CacheRepository, UsageProvider,
};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute, queue,
    style::{Color, PrintStyledContent, Stylize},
    terminal::{self, ClearType},
};
use futures::FutureExt;
use std::{
    collections::HashMap,
    io::{self, Write},
    panic::AssertUnwindSafe,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const FOOTER: &str = "q quit  r refresh  c view  u used/left";
const BAR_MAX: usize = 20;
const COMPACT_BAR: usize = 8;

/// The row cadence: `5h`, `wk`, or the window's own label. Grouped (pool) windows read their
/// length; ungrouped ones their label, because Claude's `Fable` window is weekly too.
pub fn cadence(window: &UsageWindow) -> String {
    let grouped = window.group.as_ref();
    let duration = grouped.and(window.duration_minutes);
    if window.label == "5h" || duration == Some(300) {
        "5h".into()
    } else if window.label.eq_ignore_ascii_case("weekly") || duration == Some(10_080) {
        "wk".into()
    } else if let Some(group) = grouped {
        let pool = pool_name(&group.name);
        window
            .label
            .strip_prefix(pool.as_str())
            .unwrap_or(&window.label)
            .trim()
            .into()
    } else {
        window.label.clone()
    }
}

/// `<pool> <cadence>` for a pool's window (`Gemini 5h`, `Gemini 4 Argon wk`), else the cadence.
pub fn row_label(window: &UsageWindow) -> String {
    let cadence = cadence(window);
    match &window.group {
        Some(group) if cadence.is_empty() => pool_name(&group.name),
        Some(group) => format!("{} {cadence}", pool_name(&group.name)),
        None => cadence,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Severity {
    Neutral,
    Amber,
    Orange,
    Red,
}

/// #66's meter colors, on the notification thresholds (50/85/90). Always from *used*.
pub fn severity(used: f64) -> Severity {
    match used {
        used if used >= 90.0 => Severity::Red,
        used if used >= 85.0 => Severity::Orange,
        used if used >= 50.0 => Severity::Amber,
        _ => Severity::Neutral,
    }
}

pub fn format_age(updated_at: u64, now: u64) -> String {
    match now.saturating_sub(updated_at) / 60_000 {
        0 => "Updated just now".into(),
        minutes => format!("Updated {} ago", format_minutes(minutes)),
    }
}

fn percent(used: f64, remaining: bool) -> String {
    format!("{:.0}%", if remaining { 100.0 - used } else { used })
}

fn tightest(snapshot: &ProviderUsageSnapshot) -> Option<&UsageWindow> {
    snapshot.windows.iter().reduce(|tightest, window| {
        if window.used_percent > tightest.used_percent {
            window
        } else {
            tightest
        }
    })
}

/// Status records age only as of loading, so a reading that aged while the view stayed open
/// counts as stale too.
fn is_stale(snapshot: &ProviderUsageSnapshot, now: u64) -> bool {
    snapshot.status == UsageStatus::Stale
        || now.saturating_sub(snapshot.updated_at) > STALE_AFTER_MS
}

/// The user's `providerOrder`, then the most used first, as the popover sorts.
pub fn ordered<'a>(
    snapshots: impl IntoIterator<Item = &'a ProviderUsageSnapshot>,
    order: &[String],
) -> Vec<&'a ProviderUsageSnapshot> {
    let rank = |id: &str| order.iter().position(|o| o == id).unwrap_or(order.len());
    let most = |snapshot: &ProviderUsageSnapshot| {
        snapshot
            .windows
            .iter()
            .map(|window| window.used_percent)
            .fold(0.0, f64::max)
    };
    let mut sorted = snapshots.into_iter().collect::<Vec<_>>();
    sorted.sort_by(|a, b| {
        rank(&a.provider)
            .cmp(&rank(&b.provider))
            .then(most(b).total_cmp(&most(a)))
    });
    sorted
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Tone {
    Plain,
    Bold,
    Muted,
    Warning,
    Critical,
    Meter(Severity),
    Track,
}

pub type Line = Vec<(String, Tone)>;

#[derive(Clone, Copy, Debug, Default)]
pub struct View {
    pub compact: bool,
    pub remaining: bool,
}

fn width_of(line: &Line) -> usize {
    line.iter().map(|(text, _)| text.chars().count()).sum()
}

/// Cut a line to `width` columns.
fn clip(line: Line, width: usize) -> Line {
    let mut left = width;
    line.into_iter()
        .filter_map(|(text, tone)| {
            let kept = text.chars().take(left).collect::<String>();
            left -= kept.chars().count();
            (!kept.is_empty()).then_some((kept, tone))
        })
        .collect()
}

/// `left` and `right` on one line, `right` flush right; `left` gives way when it does not fit.
fn spread(left: Line, right: Line, width: usize) -> Line {
    let room = width.saturating_sub(width_of(&right) + 1);
    let mut line = clip(left, room);
    let gap = width.saturating_sub(width_of(&line) + width_of(&right));
    line.push((" ".repeat(gap), Tone::Plain));
    line.extend(right);
    line
}

fn bar(used: f64, width: usize) -> Line {
    let filled = ((used / 100.0 * width as f64).round() as usize).min(width);
    vec![
        ("━".repeat(filled), Tone::Meter(severity(used))),
        ("─".repeat(width - filled), Tone::Track),
    ]
}

fn number(used: f64, remaining: bool, width: usize) -> (String, Tone) {
    let tone = if severity(used) == Severity::Red {
        Tone::Critical
    } else {
        Tone::Plain
    };
    (format!("{:>width$}", percent(used, remaining)), tone)
}

fn title(snapshot: &ProviderUsageSnapshot) -> Line {
    let mut line = vec![(snapshot.display_name.clone(), Tone::Bold)];
    if let Some(plan) = &snapshot.plan {
        line.push((format!(" {plan}"), Tone::Muted));
    }
    line
}

/// The muted line a provider shows instead of meters.
fn no_data(snapshot: &ProviderUsageSnapshot) -> String {
    match (&snapshot.error, snapshot.status) {
        (Some(error), _) => error.clone(),
        (None, UsageStatus::Fetching | UsageStatus::Idle) => "Loading…".into(),
        (None, _) => "Usage unavailable".into(),
    }
}

/// Column widths of the detailed view, shared by every provider so the bars line up.
struct Columns {
    label: usize,
    bar: usize,
    bare: bool,
}

fn reset_text(window: &UsageWindow, bare: bool, now: u64) -> String {
    match window.resets_at {
        Some(at) if bare => countdown(at, now),
        at => format_reset(at, now),
    }
}

fn columns(providers: &[&ProviderUsageSnapshot], now: u64, width: usize) -> Columns {
    let windows = || providers.iter().flat_map(|snapshot| &snapshot.windows);
    let label = windows()
        .map(|window| row_label(window).chars().count())
        .max()
        .unwrap_or(0);
    let fit = |bare: bool| {
        let reset = windows()
            .map(|window| reset_text(window, bare, now).chars().count())
            .max()
            .unwrap_or(0);
        // "  label bar pct  reset"
        let bar = width.saturating_sub(2 + label + 1 + 1 + 4 + 2 + reset);
        Columns {
            label,
            bar: bar.min(BAR_MAX),
            bare,
        }
    };
    let full = fit(false);
    if full.bar >= 6 {
        full
    } else {
        fit(true)
    }
}

fn detailed(
    snapshot: &ProviderUsageSnapshot,
    view: View,
    columns: &Columns,
    now: u64,
    width: usize,
) -> Vec<Line> {
    let right = if snapshot.windows.is_empty() {
        vec![]
    } else if is_stale(snapshot, now) {
        vec![(format_age(snapshot.updated_at, now), Tone::Warning)]
    } else {
        let reset = tightest(snapshot).and_then(|window| window.resets_at);
        vec![(format_reset(reset, now), Tone::Muted)]
    };
    let mut lines = vec![spread(title(snapshot), right, width)];
    for window in &snapshot.windows {
        let mut line = vec![(
            format!("  {:<width$} ", row_label(window), width = columns.label),
            Tone::Muted,
        )];
        if columns.bar > 0 {
            line.extend(bar(window.used_percent, columns.bar));
            line.push((" ".into(), Tone::Plain));
        }
        line.push(number(window.used_percent, view.remaining, 4));
        line.push((
            format!("  {}", reset_text(window, columns.bare, now)),
            Tone::Muted,
        ));
        lines.push(line);
    }
    if snapshot.windows.is_empty() || snapshot.error.is_some() {
        lines.push(vec![(format!("  {}", no_data(snapshot)), Tone::Muted)]);
    }
    lines
}

fn compact(snapshot: &ProviderUsageSnapshot, view: View, now: u64, width: usize) -> Line {
    let Some(window) = tightest(snapshot) else {
        return spread(
            title(snapshot),
            vec![(no_data(snapshot), Tone::Muted)],
            width,
        );
    };
    let mut right = if is_stale(snapshot, now) {
        vec![(format_age(snapshot.updated_at, now), Tone::Warning)]
    } else {
        let left = window.resets_at.map(|at| countdown(at, now));
        vec![(left.unwrap_or_default(), Tone::Muted)]
    };
    right.push((" ".into(), Tone::Plain));
    right.extend(bar(window.used_percent, COMPACT_BAR));
    right.push(number(window.used_percent, view.remaining, 5));
    spread(title(snapshot), right, width)
}

/// The whole screen, top to bottom, each line at most `width` columns.
pub fn render(
    providers: &[&ProviderUsageSnapshot],
    view: View,
    refreshing: bool,
    now: u64,
    width: usize,
) -> Vec<Line> {
    let status = if refreshing { "Refreshing…" } else { "" };
    let mut lines = vec![
        spread(
            vec![("Usage".into(), Tone::Bold)],
            vec![(status.into(), Tone::Muted)],
            width,
        ),
        vec![],
    ];
    let columns = columns(providers, now, width);
    for (index, snapshot) in providers.iter().enumerate() {
        if view.compact {
            lines.push(compact(snapshot, view, now, width));
        } else {
            if index > 0 {
                lines.push(vec![]);
            }
            lines.extend(detailed(snapshot, view, &columns, now, width));
        }
    }
    lines.push(vec![]);
    lines.push(vec![(FOOTER.into(), Tone::Muted)]);
    lines.into_iter().map(|line| clip(line, width)).collect()
}

/// Raw mode and the alternate screen for as long as it lives, and restored on panic too.
struct Screen;

impl Screen {
    fn enter() -> io::Result<Self> {
        // Only a panic on this thread ends the view; a provider's panic on a runtime worker is
        // caught and shown as that provider's error.
        let owner = std::thread::current().id();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if std::thread::current().id() == owner {
                restore();
            }
            previous(info);
        }));
        let screen = Screen;
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), terminal::EnterAlternateScreen, cursor::Hide)?;
        Ok(screen)
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        restore();
    }
}

fn restore() {
    let _ = execute!(io::stdout(), cursor::Show, terminal::LeaveAlternateScreen);
    let _ = terminal::disable_raw_mode();
}

fn draw(lines: &[Line]) -> io::Result<()> {
    let (_, height) = terminal::size()?;
    let mut out = io::stdout().lock();
    let mut row = 0;
    for line in lines.iter().take(height.into()) {
        queue!(out, cursor::MoveTo(0, row))?;
        for (text, tone) in line {
            let text = text.as_str();
            let styled = match tone {
                Tone::Plain => text.stylize(),
                Tone::Bold => text.bold(),
                Tone::Muted | Tone::Track => text.dark_grey(),
                Tone::Warning | Tone::Meter(Severity::Amber) => text.yellow(),
                Tone::Critical => text.red().bold(),
                Tone::Meter(Severity::Neutral) => text.grey(),
                Tone::Meter(Severity::Orange) => text.with(Color::AnsiValue(208)),
                Tone::Meter(Severity::Red) => text.red(),
            };
            queue!(out, PrintStyledContent(styled))?;
        }
        queue!(out, terminal::Clear(ClearType::UntilNewLine))?;
        row += 1;
    }
    queue!(
        out,
        cursor::MoveTo(0, row),
        terminal::Clear(ClearType::FromCursorDown)
    )?;
    out.flush()
}

type Fetched = (usize, Result<ProviderUsageSnapshot, ProviderError>);

fn start_fetch(
    runtime: &tokio::runtime::Runtime,
    providers: &Arc<Vec<Box<dyn UsageProvider>>>,
    cancellation: &CancellationToken,
    sender: &mpsc::Sender<Fetched>,
) -> usize {
    for index in 0..providers.len() {
        let (providers, cancellation, sender) =
            (Arc::clone(providers), cancellation.clone(), sender.clone());
        runtime.spawn(async move {
            let context = UsageFetchContext {
                cancellation,
                reason: UsageFetchReason::Manual,
                now: now_ms(),
            };
            let result = AssertUnwindSafe(providers[index].fetch_usage(&context))
                .catch_unwind()
                .await
                .unwrap_or_else(|_| Err(ProviderError::new(ProviderErrorKind::Unknown, "panic")));
            let _ = sender.send((index, result));
        });
    }
    providers.len()
}

pub fn run(providers: Vec<Box<dyn UsageProvider>>, settings: &TrayciSettings) -> io::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let enabled = enabled_provider_ids(settings);
    let providers: Arc<Vec<_>> = Arc::new(
        providers
            .into_iter()
            .filter(|provider| enabled.iter().any(|id| id == provider.id()))
            .collect(),
    );
    let mut snapshots = runtime.block_on(CacheRepository::default().load(now_ms()));
    snapshots.retain(|id, _| enabled.contains(id));
    for provider in providers.iter() {
        snapshots.entry(provider.id().into()).or_insert_with(|| {
            empty_snapshot(provider.as_ref(), UsageStatus::Fetching, None, now_ms())
        });
    }
    let mut view = View {
        compact: settings.display_mode == DisplayMode::Compact,
        remaining: settings.percentage_display == PercentageDisplay::Remaining,
    };
    let interval = Duration::from_secs(settings.poll_interval_minutes * 60);
    let cancellation = CancellationToken::new();
    let (sender, results) = mpsc::channel::<Fetched>();
    let mut pending = 0;
    let mut fetched_at: Option<Instant> = None;

    let screen = Screen::enter()?;
    loop {
        // One fetch at a time; the next only once the poll interval has passed since it finished.
        if pending == 0 && fetched_at.map_or(true, |at| at.elapsed() >= interval) {
            pending = start_fetch(&runtime, &providers, &cancellation, &sender);
            fetched_at = Some(Instant::now());
        }
        let (width, _) = terminal::size()?;
        let shown = ordered(snapshots.values(), &settings.provider_order);
        draw(&render(&shown, view, pending > 0, now_ms(), width.into()))?;

        if event::poll(Duration::from_millis(500))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            break
                        }
                        KeyCode::Char('r') if pending == 0 => fetched_at = None,
                        KeyCode::Char('c') => view.compact = !view.compact,
                        KeyCode::Char('u') => view.remaining = !view.remaining,
                        _ => {}
                    }
                }
            }
        }
        while let Ok((index, result)) = results.try_recv() {
            pending -= 1;
            if pending == 0 {
                fetched_at = Some(Instant::now());
            }
            apply(&mut snapshots, providers[index].as_ref(), result, now_ms());
        }
    }
    cancellation.cancel();
    abort_all_children();
    runtime.shutdown_background();
    drop(screen);
    Ok(())
}

fn apply(
    snapshots: &mut HashMap<ProviderId, ProviderUsageSnapshot>,
    provider: &dyn UsageProvider,
    result: Result<ProviderUsageSnapshot, ProviderError>,
    now: u64,
) {
    let previous = snapshots.remove(provider.id());
    let snapshot = match result {
        Ok(snapshot) => snapshot.normalize(),
        Err(error) => failed_snapshot(provider, previous, error.kind, now),
    };
    snapshots.insert(provider.id().into(), snapshot);
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;
    const MINUTE: u64 = 60_000;

    fn window(id: &str, label: &str, duration: u64, group: Option<&str>) -> UsageWindow {
        UsageWindow {
            id: id.into(),
            label: label.into(),
            used_percent: 17.0,
            duration_minutes: Some(duration),
            resets_at: Some(NOW + 234 * MINUTE),
            reset_description: None,
            group: group.map(|name| UsageGroup {
                name: name.into(),
                models: None,
            }),
        }
    }

    fn snapshot(provider: &str, windows: Vec<UsageWindow>) -> ProviderUsageSnapshot {
        ProviderUsageSnapshot {
            provider: provider.into(),
            display_name: provider.into(),
            status: UsageStatus::Ok,
            plan: None,
            windows,
            updated_at: NOW,
            source: None,
            error: None,
        }
    }

    /// The windows `cache.json` holds today for Claude and Antigravity.
    fn claude() -> ProviderUsageSnapshot {
        snapshot(
            "claude",
            vec![
                window("session", "5h", 300, None),
                window("weekly", "Weekly", 10_080, None),
                window("fable-weekly", "Fable", 10_080, None),
            ],
        )
    }

    fn antigravity() -> ProviderUsageSnapshot {
        snapshot(
            "antigravity",
            vec![
                window("gemini-5h", "Gemini 5h", 300, Some("Gemini Models")),
                window(
                    "gemini-weekly",
                    "Gemini Weekly",
                    10_080,
                    Some("Gemini Models"),
                ),
                window("3p-5h", "Claude/GPT 5h", 300, Some("Claude and GPT models")),
                window(
                    "3p-weekly",
                    "Claude/GPT Weekly",
                    10_080,
                    Some("Claude and GPT models"),
                ),
                window(
                    "gemini-argon-wtu-weekly",
                    "Gemini 4 Argon",
                    10_080,
                    Some("Gemini 4 Argon"),
                ),
            ],
        )
    }

    fn text(line: &Line) -> String {
        line.iter().map(|(text, _)| text.as_str()).collect()
    }

    #[test]
    fn row_labels_follow_the_vocabulary_table() {
        let labels = |snapshot: ProviderUsageSnapshot| {
            snapshot.windows.iter().map(row_label).collect::<Vec<_>>()
        };
        assert_eq!(labels(claude()), ["5h", "wk", "Fable"]);
        assert_eq!(
            labels(antigravity()),
            [
                "Gemini 5h",
                "Gemini wk",
                "Claude/GPT 5h",
                "Claude/GPT wk",
                "Gemini 4 Argon wk"
            ]
        );
        let daily = window("x", "Gemini Daily", 1440, Some("Gemini Models"));
        assert_eq!(row_label(&daily), "Gemini Daily");
    }

    #[test]
    fn countdown_and_age_read_at_a_glance() {
        assert_eq!(countdown(NOW + 45 * MINUTE, NOW), "45m");
        assert_eq!(countdown(NOW + 234 * MINUTE, NOW), "3h 54m");
        assert_eq!(countdown(NOW + (5 * 1440 + 60) * MINUTE, NOW), "5d 1h");
        assert_eq!(countdown(NOW + 30_000, NOW), "now");
        assert_eq!(countdown(NOW - MINUTE, NOW), "now");
        assert_eq!(
            format_reset(Some(NOW + 234 * MINUTE), NOW),
            "resets in 3h 54m"
        );
        assert_eq!(format_age(NOW - 2 * MINUTE, NOW), "Updated 2m ago");
        assert_eq!(format_age(NOW - 10_000, NOW), "Updated just now");
    }

    #[test]
    fn severity_follows_the_notification_thresholds() {
        assert_eq!(severity(49.9), Severity::Neutral);
        assert_eq!(severity(50.0), Severity::Amber);
        assert_eq!(severity(84.9), Severity::Amber);
        assert_eq!(severity(85.0), Severity::Orange);
        assert_eq!(severity(89.9), Severity::Orange);
        assert_eq!(severity(90.0), Severity::Red);
    }

    #[test]
    fn the_meter_shows_used_even_when_the_number_shows_remaining() {
        assert_eq!(percent(92.0, true), "8%");
        assert_eq!(number(92.0, true, 4), ("  8%".to_string(), Tone::Critical));
        assert_eq!(bar(92.0, 10)[0].1, Tone::Meter(Severity::Red));
    }

    #[test]
    fn providers_follow_the_user_order_then_usage() {
        let mut codex = snapshot("codex", vec![window("weekly", "Weekly", 10_080, None)]);
        codex.windows[0].used_percent = 99.0;
        let (claude, antigravity) = (claude(), antigravity());
        let all = [&codex, &claude, &antigravity];
        let order = ["antigravity".to_string(), "claude".to_string()];
        let ids = |sorted: Vec<&ProviderUsageSnapshot>| {
            sorted
                .iter()
                .map(|snapshot| snapshot.provider.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(ordered(all, &order)),
            ["antigravity", "claude", "codex"]
        );
        assert_eq!(ids(ordered(all, &[]))[0], "codex");
    }

    #[test]
    fn detailed_view_names_the_tightest_reset_and_every_window() {
        let claude = claude();
        let lines = render(&[&claude], View::default(), false, NOW, 60);
        let lines = lines.iter().map(text).collect::<Vec<_>>();
        assert_eq!(
            lines[2].trim_end(),
            format!("claude{}resets in 3h 54m", " ".repeat(38))
        );
        assert!(lines[3].starts_with("  5h    "));
        assert!(lines[3].ends_with(" 17%  resets in 3h 54m"));
        assert_eq!(lines.last().unwrap(), FOOTER);
    }

    #[test]
    fn compact_view_is_one_line_per_provider() {
        let (claude, antigravity) = (claude(), antigravity());
        let view = View {
            compact: true,
            remaining: true,
        };
        let lines = render(&[&claude, &antigravity], view, true, NOW, 50);
        let lines = lines.iter().map(text).collect::<Vec<_>>();
        assert!(lines[0].ends_with("Refreshing…"));
        assert!(lines[2].starts_with("claude"));
        assert!(lines[2].ends_with("3h 54m ━───────  83%"), "{}", lines[2]);
        assert!(lines[3].starts_with("antigravity"));
        assert_eq!(lines.len(), 6);
    }

    #[test]
    fn everything_fits_forty_columns() {
        let mut antigravity = antigravity();
        antigravity.windows[4].resets_at = Some(NOW + (6 * 1440 + 23 * 60) * MINUTE);
        antigravity.windows[4].used_percent = 100.0;
        for compact in [false, true] {
            let view = View {
                compact,
                remaining: false,
            };
            for line in render(&[&antigravity], view, true, NOW, 40) {
                assert!(width_of(&line) <= 40, "{}", text(&line));
            }
        }
        let lines = render(&[&antigravity], View::default(), false, NOW, 40);
        let argon = text(&lines[7]);
        assert!(
            argon.contains("━━"),
            "the bar survives at 40 columns: {argon}"
        );
        assert!(argon.ends_with("100%  6d 23h"), "{argon}");
    }

    #[test]
    fn stale_and_failed_providers_say_so() {
        let mut aged = claude();
        aged.updated_at = NOW - 2 * 60 * MINUTE;
        aged.error = Some("Temporarily rate limited".into());
        let mut failed = snapshot("codex", vec![]);
        failed.error = Some("Not signed in".into());
        let loading = ProviderUsageSnapshot {
            status: UsageStatus::Fetching,
            ..snapshot("antigravity", vec![])
        };
        let lines = render(&[&aged, &failed, &loading], View::default(), false, NOW, 60);
        let header = &lines[2];
        assert!(text(header).ends_with("Updated 2h 0m ago"));
        assert_eq!(header.last().unwrap().1, Tone::Warning);
        let lines = lines.iter().map(text).collect::<Vec<_>>();
        assert_eq!(lines[6], "  Temporarily rate limited");
        assert_eq!(lines[9], "  Not signed in");
        assert_eq!(lines[12], "  Loading…");
    }
}
