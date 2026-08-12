use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;
use trayci_core::providers::{
    antigravity::{normalize_antigravity_quota, parse_antigravity_usage},
    claude::{normalize_claude_usage, parse_claude_usage},
    codex::{normalize_codex_rate_limits, parse_codex_usage},
    common::{active_child_count, run_pty, strip_terminal_codes, PtyOptions},
};
use trayci_core::ProviderErrorKind;

const NOW: u64 = 1_754_656_000_000;

#[test]
fn claude_oauth_and_pty_variants() {
    let windows = normalize_claude_usage(&json!({"limits":[
        {"group":"session","percent":1,"resets_at":"2026-08-08T14:00:00Z"},
        {"group":"weekly","percent":29,"resets_at":null},
        {"group":"weekly","percent":20,"scope":{"model":{"display_name":"Fable"}}},
        {"group":"monthly","percent":99}
    ]}));
    assert_eq!(
        windows
            .iter()
            .map(|value| (value.id.as_str(), value.used_percent))
            .collect::<Vec<_>>(),
        [("session", 1.0), ("weekly", 29.0), ("fable-weekly", 20.0)]
    );
    let parsed = parse_claude_usage(include_str!("fixtures/claude/usage-standard.txt"), NOW);
    assert_eq!(
        parsed
            .iter()
            .map(|value| (value.id.as_str(), value.used_percent))
            .collect::<Vec<_>>(),
        [("session", 62.0), ("weekly", 31.0), ("fable-weekly", 20.0)]
    );
    assert_eq!(parsed[0].resets_at, Some(NOW + 135 * 60_000));
}

#[test]
fn claude_legacy_ignores_unknown_keys() {
    let windows = normalize_claude_usage(
        &json!({"five_hour":{"utilization":0.5},"seven_day":{"utilization":31},"seven_day_sonnet":{"utilization":12},"seven_day_unknown":{"utilization":99}}),
    );
    assert_eq!(
        windows
            .iter()
            .map(|value| (value.id.as_str(), value.used_percent))
            .collect::<Vec<_>>(),
        [("session", 0.5), ("weekly", 31.0), ("sonnet-weekly", 12.0)]
    );
}

#[test]
fn codex_rpc_and_pty_variants() {
    let (windows, plan) = normalize_codex_rate_limits(
        &json!({"rateLimitsByLimitId":{"codex":{"planType":"plus","primary":{"usedPercent":10,"windowDurationMins":299,"resetsAt":1786200000},"secondary":{"usedPercent":28,"windowDurationMins":10079}}}}),
    );
    assert_eq!(plan.as_deref(), Some("plus"));
    assert_eq!(
        windows
            .iter()
            .map(|value| (value.id.as_str(), value.used_percent))
            .collect::<Vec<_>>(),
        [("session", 10.0), ("weekly", 28.0)]
    );
    assert_eq!(windows[0].resets_at, Some(1_786_200_000_000));
    let parsed = parse_codex_usage(include_str!("fixtures/codex/status-standard.txt"), NOW);
    assert_eq!(
        parsed
            .iter()
            .map(|value| (value.id.as_str(), value.used_percent))
            .collect::<Vec<_>>(),
        [("session", 7.0), ("weekly", 12.0)]
    );
}

#[test]
fn antigravity_cli_and_oauth_variants() {
    let (windows, plan) = parse_antigravity_usage("Models & Quota GEMINI MODELS Weekly Limit Remaining 75.00% Resets in 4d 2h Five Hour Limit Remaining 90.00% Resets in 2h 30m CLAUDE AND GPT MODELS Weekly Limit Remaining 60.00% Resets in 3d Five Hour Limit Remaining 100.00% Quota available G (Google AI Pro)", NOW);
    assert_eq!(plan.as_deref(), Some("Google AI Pro"));
    assert_eq!(
        windows
            .iter()
            .map(|value| (value.id.as_str(), value.used_percent))
            .collect::<Vec<_>>(),
        [
            ("gemini-session", 10.0),
            ("gemini-weekly", 25.0),
            ("claude-gpt-session", 0.0),
            ("claude-gpt-weekly", 40.0)
        ]
    );
    let quota = normalize_antigravity_quota(&json!({"buckets":[
        {"remainingFraction":0.75,"resetTime":"2026-08-08T13:00:00Z","modelId":"gemini-2.5-pro-preview"},
        {"remainingFraction":0.75,"resetTime":"2026-08-08T13:00:00Z","modelId":"gemini-2.5-pro"},
        {"remainingFraction":0.1,"resetTime":"2026-08-08T14:00:00Z","modelId":"gemini-2.5-flash"}
    ]}));
    assert_eq!(
        quota
            .iter()
            .map(|value| (value.label.as_str(), value.used_percent))
            .collect::<Vec<_>>(),
        [("Pro", 25.0), ("Flash", 90.0)]
    );
}

#[test]
fn antigravity_reads_the_refreshes_in_wording() {
    let (windows, _) =
        parse_antigravity_usage(include_str!("fixtures/antigravity/usage-standard.txt"), NOW);
    assert_eq!(
        windows
            .iter()
            .map(|value| (value.id.as_str(), value.resets_at))
            .collect::<Vec<_>>(),
        [
            ("gemini-session", Some(NOW + 109 * 60_000)),
            ("gemini-weekly", Some(NOW + 5393 * 60_000)),
            ("claude-gpt-session", None),
            ("claude-gpt-weekly", None)
        ]
    );
    assert_eq!(
        windows[0].reset_description.as_deref(),
        Some("Refreshes in 1h 49m")
    );
}

#[test]
fn strips_terminal_control_sequences() {
    assert_eq!(
        strip_terminal_codes("\u{1b}[31mUsage\u{1b}[0m\u{1b}]0;title\u{7}\r42%\0"),
        "Usage\n42%"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn pty_timeout_kills_and_untracks_child() {
    let error = run_pty(PtyOptions {
        executable: Path::new("/bin/sleep"),
        args: &["10"],
        input: "",
        write_delay: Duration::ZERO,
        completion_delay: Duration::ZERO,
        timeout: Duration::from_millis(50),
        cancellation: CancellationToken::new(),
        complete: Arc::new(|_| false),
    })
    .await
    .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Timeout);
    assert_eq!(active_child_count(), 0);

    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
    });
    let error = run_pty(PtyOptions {
        executable: Path::new("/bin/sleep"),
        args: &["10"],
        input: "",
        write_delay: Duration::ZERO,
        completion_delay: Duration::ZERO,
        timeout: Duration::from_secs(5),
        cancellation,
        complete: Arc::new(|_| false),
    })
    .await
    .unwrap_err();
    assert_eq!(error.kind, ProviderErrorKind::Aborted);
    assert_eq!(active_child_count(), 0);
}
