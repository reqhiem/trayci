use super::common::{
    bounded_cwd, clamp, epoch_ms, home_dir, reset_from_text, resolve_executable, run_pty,
    title_case, PtyOptions,
};
use crate::{model::*, service::UsageProvider};
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::{env, fs, path::Path, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
};

type Settings = Arc<dyn Fn() -> TrayciSettings + Send + Sync>;
pub struct CodexProvider {
    settings: Settings,
}

impl CodexProvider {
    pub fn new(settings: impl Fn() -> TrayciSettings + Send + Sync + 'static) -> Self {
        Self {
            settings: Arc::new(settings),
        }
    }
    fn snapshot(
        &self,
        windows: Vec<UsageWindow>,
        plan: Option<String>,
        now: u64,
        source: UsageSource,
    ) -> ProviderUsageSnapshot {
        ProviderUsageSnapshot {
            provider: self.id().into(),
            display_name: self.display_name().into(),
            status: UsageStatus::Ok,
            plan,
            windows,
            updated_at: now,
            source: Some(source),
            error: None,
        }
    }
}

fn has_auth() -> bool {
    let home = env::var_os("CODEX_HOME")
        .map(Into::into)
        .or_else(|| home_dir().map(|path| path.join(".codex")));
    let auth = home
        .and_then(|path| fs::read(path.join("auth.json")).ok())
        .and_then(|data| serde_json::from_slice::<Value>(&data).ok());
    auth.is_some_and(|value| {
        value
            .pointer("/tokens/access_token")
            .and_then(Value::as_str)
            .is_some()
            || value.get("auth_mode").and_then(Value::as_str).is_some()
    }) || env::var_os("OPENAI_API_KEY").is_some()
}

pub fn normalize_codex_rate_limits(raw: &Value) -> (Vec<UsageWindow>, Option<String>) {
    let keyed = raw.get("rateLimitsByLimitId").and_then(Value::as_object);
    let snapshots: Vec<(String, &Value)> = if keyed.is_some_and(|value| !value.is_empty()) {
        keyed
            .unwrap()
            .iter()
            .map(|(id, value)| (id.clone(), value))
            .collect()
    } else {
        raw.get("rateLimits")
            .map(|value| {
                vec![(
                    value
                        .get("limitId")
                        .and_then(Value::as_str)
                        .unwrap_or("codex")
                        .into(),
                    value,
                )]
            })
            .unwrap_or_default()
    };
    let mut windows = Vec::new();
    let mut plan = None;
    for (limit_id, snapshot) in &snapshots {
        if plan.is_none() {
            plan = snapshot
                .get("planType")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        for (slot, value) in [
            ("primary", snapshot.get("primary")),
            ("secondary", snapshot.get("secondary")),
        ] {
            let Some(value) = value else { continue };
            let Some(percent) = value.get("usedPercent").and_then(Value::as_f64) else {
                continue;
            };
            let raw_duration = value.get("windowDurationMins").and_then(Value::as_f64);
            let duration = raw_duration.map(|duration| {
                if (duration - 300.0).abs() <= 2.0 {
                    300
                } else if (duration - 10_080.0).abs() <= 2.0 {
                    10_080
                } else {
                    duration as u64
                }
            });
            let prefix = if snapshots.len() > 1 {
                format!("{limit_id}-")
            } else {
                String::new()
            };
            let fallback = snapshot
                .get("limitName")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| title_case(limit_id));
            windows.push(UsageWindow {
                id: format!(
                    "{prefix}{}",
                    match duration {
                        Some(300) => "session",
                        Some(10_080) => "weekly",
                        _ => slot,
                    }
                ),
                label: match duration {
                    Some(300) => "5h".into(),
                    Some(10_080) => "Weekly".into(),
                    _ => fallback,
                },
                used_percent: clamp(percent),
                duration_minutes: duration,
                resets_at: value.get("resetsAt").and_then(epoch_ms),
                reset_description: None,
            });
        }
    }
    (windows, plan)
}

pub fn parse_codex_usage(output: &str, now: u64) -> Vec<UsageWindow> {
    let normalized = output.split_whitespace().collect::<Vec<_>>().join(" ");
    [
        (
            "session",
            "5h",
            300,
            r"(?i)(?:5h|session)(?:\s+limit)?[^%]{0,100}?(\d+(?:\.\d+)?)%([^|·]{0,80})",
        ),
        (
            "weekly",
            "Weekly",
            10_080,
            r"(?i)(?:weekly|week|7-day)(?:\s+limit)?[^%]{0,100}?(\d+(?:\.\d+)?)%([^|·]{0,80})",
        ),
    ]
    .into_iter()
    .filter_map(|(id, label, duration, pattern)| {
        let captures = Regex::new(pattern).unwrap().captures(&normalized)?;
        let tail = captures.get(2).map_or("", |value| value.as_str());
        let raw = captures[1].parse::<f64>().ok()?;
        let percent = if Regex::new("(?i)left|remaining|available")
            .unwrap()
            .is_match(tail)
        {
            100.0 - raw
        } else {
            raw
        };
        let reset = Regex::new(r"(?i)resets?\s+(?:in\s+)?[^|·,]{1,40}")
            .unwrap()
            .find(tail)
            .map(|value| value.as_str().trim().to_owned());
        Some(UsageWindow {
            id: id.into(),
            label: label.into(),
            used_percent: clamp(percent),
            duration_minutes: Some(duration),
            resets_at: reset.as_deref().and_then(|text| reset_from_text(text, now)),
            reset_description: reset,
        })
    })
    .collect()
}

async fn rpc(
    executable: &Path,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<Value, ProviderError> {
    let mut child = Command::new(executable)
        .args(["app-server", "--stdio"])
        .current_dir(bounded_cwd()?)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| {
            ProviderError::new(
                ProviderErrorKind::Unknown,
                format!("Codex app-server failed to start: {error}"),
            )
        })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| ProviderError::new(ProviderErrorKind::Unknown, "Codex stdin unavailable"))?;
    let mut lines = BufReader::new(child.stdout.take().ok_or_else(|| {
        ProviderError::new(ProviderErrorKind::Unknown, "Codex stdout unavailable")
    })?)
    .lines();
    stdin.write_all(format!("{}\n", json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"trayci","version":"0.3.0"}}})).as_bytes()).await.map_err(network)?;
    let initialized = read_rpc(&mut lines, 1, Duration::from_secs(30), cancellation).await?;
    if initialized.get("error").is_some() {
        return Err(ProviderError::new(
            ProviderErrorKind::Unknown,
            "Codex RPC initialization failed",
        ));
    }
    for message in [
        json!({"jsonrpc":"2.0","method":"initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"account/rateLimits/read","params":null}),
    ] {
        stdin
            .write_all(format!("{message}\n").as_bytes())
            .await
            .map_err(network)?;
    }
    let response = read_rpc(&mut lines, 2, Duration::from_secs(10), cancellation).await?;
    let _ = child.kill().await;
    response
        .get("result")
        .cloned()
        .filter(|_| response.get("error").is_none())
        .ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::Unknown,
                "Codex rate limits are unavailable",
            )
        })
}

async fn read_rpc(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    id: u64,
    timeout: Duration,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<Value, ProviderError> {
    let read = async {
        while let Some(line) = lines.next_line().await.map_err(network)? {
            if let Ok(value) = serde_json::from_str::<Value>(&line) {
                if value.get("id").and_then(Value::as_u64) == Some(id) {
                    return Ok(value);
                }
            }
        }
        Err(ProviderError::new(
            ProviderErrorKind::Unknown,
            "Codex app-server exited early",
        ))
    };
    tokio::select! {
        _ = cancellation.cancelled() => Err(ProviderError::new(ProviderErrorKind::Aborted, "Cancelled")),
        result = tokio::time::timeout(timeout, read) => result.unwrap_or_else(|_| Err(ProviderError::new(ProviderErrorKind::Timeout, "Codex RPC timed out"))),
    }
}

fn network(error: std::io::Error) -> ProviderError {
    ProviderError::new(ProviderErrorKind::Network, format!("Codex RPC: {error}"))
}

#[async_trait]
impl UsageProvider for CodexProvider {
    fn id(&self) -> &'static str {
        "codex"
    }
    fn display_name(&self) -> &'static str {
        "Codex"
    }
    async fn detect(&self) -> ProviderDetection {
        let path = resolve_executable(
            "codex",
            (self.settings)().providers.codex.executable_path.as_deref(),
        )
        .await;
        ProviderDetection {
            provider: self.id().into(),
            status: if path.is_none() {
                ProviderDetectionStatus::NotInstalled
            } else if has_auth() {
                ProviderDetectionStatus::Available
            } else {
                ProviderDetectionStatus::NotAuthenticated
            },
            executable_path: path,
        }
    }
    async fn fetch_usage(
        &self,
        context: &UsageFetchContext,
    ) -> Result<ProviderUsageSnapshot, ProviderError> {
        let detection = self.detect().await;
        let executable = detection.executable_path.ok_or_else(|| {
            ProviderError::new(ProviderErrorKind::NotInstalled, "Codex CLI not detected")
        })?;
        if detection.status == ProviderDetectionStatus::NotAuthenticated {
            return Err(ProviderError::new(
                ProviderErrorKind::NotAuthenticated,
                "Codex is not signed in",
            ));
        }
        if let Ok(raw) = rpc(&executable, &context.cancellation).await {
            let (windows, plan) = normalize_codex_rate_limits(&raw);
            if !windows.is_empty() {
                return Ok(self.snapshot(windows, plan, context.now, UsageSource::Rpc));
            }
        } else if context.cancellation.is_cancelled() {
            return Err(ProviderError::new(ProviderErrorKind::Aborted, "Cancelled"));
        }
        let output = run_pty(PtyOptions {
            executable: &executable,
            args: &[],
            input: "/status",
            write_delay: Duration::from_secs(1),
            completion_delay: Duration::from_millis(600),
            timeout: Duration::from_secs(15),
            cancellation: context.cancellation.clone(),
            complete: Arc::new(|value| {
                Regex::new(r"(?i)(?:5h|weekly|7-day)[\s\S]*?\d+(?:\.\d+)?%")
                    .unwrap()
                    .is_match(value)
            }),
        })
        .await?;
        let windows = parse_codex_usage(&output, context.now);
        if windows.is_empty() {
            Err(ProviderError::new(
                ProviderErrorKind::Parse,
                "Could not parse Codex usage",
            ))
        } else {
            Ok(self.snapshot(windows, None, context.now, UsageSource::Cli))
        }
    }
}
