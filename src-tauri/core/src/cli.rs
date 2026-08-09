use crate::{model::*, service::UsageProvider};

pub fn cli_arguments<I, S>(arguments: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    arguments
        .into_iter()
        .map(Into::into)
        .skip(1)
        .filter(|argument| argument != "." && !argument.ends_with("out/main/index.js"))
        .collect()
}

pub fn is_cli_mode(args: &[String]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.as_str(), "usage" | "doctor" | "--version"))
}

pub async fn run_cli(
    args: &[String],
    providers: &[Box<dyn UsageProvider>],
    version: &str,
) -> (i32, String) {
    if args.iter().any(|arg| arg == "--version") {
        return (0, format!("{version}\n"));
    }
    if args.iter().any(|arg| arg == "doctor") {
        let detections =
            futures::future::join_all(providers.iter().map(|provider| provider.detect())).await;
        let mut output = detections
            .iter()
            .map(|detection| {
                format!(
                    "{}: {}{}",
                    detection.provider,
                    detection_status(detection.status),
                    detection
                        .executable_path
                        .as_ref()
                        .map(|path| format!(" ({})", path.display()))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        output.push_str(&format!(
            "\nactive provider probes: {}\n",
            crate::providers::active_child_count()
        ));
        let code = i32::from(
            !detections
                .iter()
                .all(|result| result.status == ProviderDetectionStatus::Available),
        );
        return (code, output);
    }

    let selected = providers.iter().filter(|provider| {
        !args.iter().any(|arg| is_provider_id(arg)) || args.iter().any(|arg| arg == provider.id())
    });
    let cancellation = tokio_util::sync::CancellationToken::new();
    let now = now_ms();
    let fetches = futures::future::join_all(selected.map(|provider| async {
        provider
            .fetch_usage(&UsageFetchContext {
                cancellation: cancellation.clone(),
                reason: UsageFetchReason::Manual,
                now,
            })
            .await
            .unwrap_or_else(|error| error_snapshot(provider.as_ref(), error, now))
    }));
    tokio::pin!(fetches);
    let snapshots = tokio::select! {
        snapshots = &mut fetches => snapshots,
        _ = shutdown_signal() => {
            cancellation.cancel();
            fetches.await
        }
    };
    crate::providers::abort_all_children();
    let ok = snapshots
        .iter()
        .all(|snapshot| snapshot.status == UsageStatus::Ok);
    let output = if args.iter().any(|arg| arg == "--json") {
        serde_json::to_string_pretty(&serde_json::json!({
            "providers": snapshots.iter().map(|snapshot| (&snapshot.provider, snapshot)).collect::<std::collections::HashMap<_, _>>()
        }))
        .expect("snapshots serialize")
    } else {
        snapshots
            .iter()
            .map(|snapshot| human(snapshot, now))
            .collect::<Vec<_>>()
            .join("\n\n")
    };
    (i32::from(!ok), format!("{output}\n"))
}

pub fn format_reset(resets_at: Option<u64>, now: u64) -> String {
    let Some(resets_at) = resets_at else {
        return String::new();
    };
    let minutes = resets_at.saturating_sub(now) / 60_000;
    match minutes {
        0 => "resets now".into(),
        1..=59 => format!("resets in {minutes}m"),
        60..=1439 => format!("resets in {}h {}m", minutes / 60, minutes % 60),
        _ => format!("resets in {}d {}h", minutes / 1440, minutes % 1440 / 60),
    }
}

fn human(snapshot: &ProviderUsageSnapshot, now: u64) -> String {
    if snapshot.status != UsageStatus::Ok {
        return format!(
            "{}\n  {}",
            snapshot.display_name,
            snapshot.error.as_deref().unwrap_or("Usage unavailable")
        );
    }
    std::iter::once(snapshot.display_name.clone())
        .chain(snapshot.windows.iter().map(|window| {
            format!(
                "  {:10} {:.0}%   {}",
                window.label,
                window.used_percent,
                format_reset(window.resets_at, now)
            )
            .trim_end()
            .to_owned()
        }))
        .collect::<Vec<_>>()
        .join("\n")
}

fn error_snapshot(
    provider: &dyn UsageProvider,
    error: ProviderError,
    now: u64,
) -> ProviderUsageSnapshot {
    ProviderUsageSnapshot {
        provider: provider.id().into(),
        display_name: provider.display_name().into(),
        status: UsageStatus::Error,
        plan: None,
        windows: Vec::new(),
        updated_at: now,
        source: None,
        error: Some(
            match error.kind {
                ProviderErrorKind::NotInstalled => "Provider not installed",
                ProviderErrorKind::NotAuthenticated => "Provider authentication required",
                ProviderErrorKind::RateLimited => "Provider rate limited",
                ProviderErrorKind::Timeout => "Provider timed out",
                ProviderErrorKind::Aborted => "Request cancelled",
                ProviderErrorKind::Parse
                | ProviderErrorKind::Network
                | ProviderErrorKind::Unknown => "Usage unavailable",
            }
            .into(),
        ),
    }
}

fn detection_status(status: ProviderDetectionStatus) -> &'static str {
    match status {
        ProviderDetectionStatus::Available => "available",
        ProviderDetectionStatus::NotInstalled => "not-installed",
        ProviderDetectionStatus::NotAuthenticated => "not-authenticated",
        ProviderDetectionStatus::Unknown => "unknown",
    }
}

pub fn is_provider_id(value: &str) -> bool {
    matches!(value, "claude" | "codex" | "antigravity")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(unix)]
async fn shutdown_signal() {
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("install SIGTERM handler");
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {},
        _ = terminate.recv() => {},
    }
}

#[cfg(not(unix))]
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
