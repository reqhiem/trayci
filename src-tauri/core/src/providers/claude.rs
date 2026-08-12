use super::common::{
    clamp, epoch_ms, home_dir, reset_from_text, resolve_executable, run_pty, PtyOptions,
};
use crate::{model::*, service::UsageProvider};
use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;
use std::{
    env, fs,
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::Duration,
};

type Settings = Arc<dyn Fn() -> TrayciSettings + Send + Sync>;

pub struct ClaudeProvider {
    settings: Settings,
}

impl ClaudeProvider {
    pub fn new(settings: impl Fn() -> TrayciSettings + Send + Sync + 'static) -> Self {
        Self {
            settings: Arc::new(settings),
        }
    }

    fn snapshot(
        &self,
        windows: Vec<UsageWindow>,
        now: u64,
        source: UsageSource,
    ) -> ProviderUsageSnapshot {
        ProviderUsageSnapshot {
            provider: self.id().into(),
            display_name: self.display_name().into(),
            status: UsageStatus::Ok,
            plan: None,
            windows,
            updated_at: now,
            source: Some(source),
            error: None,
        }
    }
}

fn credential_path() -> Option<PathBuf> {
    env::var_os("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(home_dir)
        .map(|path| {
            if env::var_os("CLAUDE_CONFIG_DIR").is_some() {
                path.join(".credentials.json")
            } else {
                path.join(".claude/.credentials.json")
            }
        })
}

fn read_credential() -> Option<(String, Option<u64>)> {
    let value: Value = serde_json::from_slice(&fs::read(credential_path()?).ok()?).ok()?;
    let oauth = value.get("claudeAiOauth")?;
    Some((
        oauth.get("accessToken")?.as_str()?.to_owned(),
        oauth.get("expiresAt").and_then(Value::as_u64),
    ))
}

fn window(
    id: String,
    label: String,
    percent: f64,
    duration: u64,
    reset: Option<&Value>,
) -> UsageWindow {
    UsageWindow {
        id,
        label,
        used_percent: clamp((percent * 100.0).round() / 100.0),
        duration_minutes: Some(duration),
        resets_at: reset.and_then(epoch_ms),
        reset_description: None,
    }
}

pub fn normalize_claude_usage(raw: &Value) -> Vec<UsageWindow> {
    let mut windows = Vec::new();
    if let Some(limits) = raw
        .get("limits")
        .and_then(Value::as_array)
        .filter(|limits| !limits.is_empty())
    {
        for limit in limits {
            let Some(group @ ("weekly" | "session")) = limit.get("group").and_then(Value::as_str)
            else {
                continue;
            };
            let Some(percent) = limit
                .get("percent")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
            else {
                continue;
            };
            let model = limit
                .pointer("/scope/model/display_name")
                .and_then(Value::as_str);
            let weekly = group == "weekly";
            windows.push(window(
                model
                    .map(|name| format!("{}-weekly", slug(name)))
                    .unwrap_or_else(|| if weekly { "weekly" } else { "session" }.into()),
                model.unwrap_or(if weekly { "Weekly" } else { "5h" }).into(),
                percent,
                if weekly { 10_080 } else { 300 },
                limit.get("resets_at"),
            ));
        }
    } else {
        for (key, label) in [
            ("five_hour", "5h"),
            ("seven_day", "Weekly"),
            ("seven_day_fable", "Fable"),
            ("seven_day_sonnet", "Sonnet"),
            ("seven_day_opus", "Opus"),
        ] {
            let Some(value) = raw.get(key) else { continue };
            let Some(percent) = value
                .get("utilization")
                .and_then(Value::as_f64)
                .filter(|value| value.is_finite())
            else {
                continue;
            };
            let session = key == "five_hour";
            windows.push(window(
                if session {
                    "session".into()
                } else if key == "seven_day" {
                    "weekly".into()
                } else {
                    format!("{}-weekly", key.trim_start_matches("seven_day_"))
                },
                label.into(),
                percent,
                if session { 300 } else { 10_080 },
                value.get("resets_at"),
            ));
        }
    }
    windows.sort_by_key(|value| {
        if value.id == "session" {
            0
        } else if value.id == "weekly" {
            1
        } else {
            2
        }
    });
    windows
}

fn slug(value: &str) -> String {
    static NON_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new("[^a-z0-9]+").unwrap());
    NON_WORD
        .replace_all(&value.to_lowercase(), "-")
        .trim_matches('-')
        .into()
}

pub fn parse_claude_usage(output: &str, now: u64) -> Vec<UsageWindow> {
    let normalized = output.split_whitespace().collect::<Vec<_>>().join(" ");
    [
        ("session", "5h", 300, r"(?i)(?:current\s+session|session|5h)[\s\S]{0,180}?(\d+(?:\.\d+)?)%\s*(used|consumed|left|remaining|available)([\s\S]{0,80})"),
        ("weekly", "Weekly", 10_080, r"(?i)(?:current\s+week|weekly\s+limits?|7-day|weekly)[\s\S]{0,180}?(\d+(?:\.\d+)?)%\s*(used|consumed|left|remaining|available)([\s\S]{0,80})"),
        ("fable-weekly", "Fable", 10_080, r"(?i)fable[\s\S]{0,180}?(\d+(?:\.\d+)?)%\s*(used|consumed|left|remaining|available)([\s\S]{0,80})"),
    ].into_iter().filter_map(|(id, label, duration, pattern)| {
        let captures = Regex::new(pattern).unwrap().captures(&normalized)?;
        let mut percent = captures[1].parse::<f64>().ok()?;
        if Regex::new("(?i)left|remaining|available").unwrap().is_match(&captures[2]) { percent = 100.0 - percent; }
        let tail = captures.get(3).map_or("", |value| value.as_str());
        let reset = Regex::new(r"(?i)resets?\s+(?:in\s+)?[^|·,]{1,40}").unwrap().find(tail).map(|value| value.as_str().trim().to_owned());
        Some(UsageWindow { id: id.into(), label: label.into(), used_percent: clamp(percent), duration_minutes: Some(duration), resets_at: reset.as_deref().and_then(|text| reset_from_text(text, now)), reset_description: reset })
    }).collect()
}

#[async_trait]
impl UsageProvider for ClaudeProvider {
    fn id(&self) -> &'static str {
        "claude"
    }
    fn display_name(&self) -> &'static str {
        "Claude"
    }

    async fn detect(&self) -> ProviderDetection {
        let path = resolve_executable(
            "claude",
            (self.settings)()
                .providers
                .claude
                .executable_path
                .as_deref(),
        )
        .await;
        ProviderDetection {
            provider: self.id().into(),
            status: if path.is_none() {
                ProviderDetectionStatus::NotInstalled
            } else if read_credential().is_some() {
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
            ProviderError::new(ProviderErrorKind::NotInstalled, "Claude CLI not detected")
        })?;
        if let Some((token, _)) = read_credential()
            .filter(|(_, expires)| expires.map_or(true, |expires| expires > context.now))
        {
            let request = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap()
                .get("https://api.anthropic.com/api/oauth/usage")
                .bearer_auth(token)
                .header("anthropic-beta", "oauth-2025-04-20")
                .header("user-agent", concat!("trayci/", env!("CARGO_PKG_VERSION")));
            let response = tokio::select! { _ = context.cancellation.cancelled() => return Err(ProviderError::new(ProviderErrorKind::Aborted, "Cancelled")), result = request.send() => result };
            if let Ok(response) = response {
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    let retry = response
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok());
                    return Err(match retry {
                        Some(seconds) => ProviderError::new(
                            ProviderErrorKind::RateLimited,
                            "Claude usage is rate limited",
                        )
                        .retry_at(context.now + seconds * 1000),
                        None => ProviderError::new(
                            ProviderErrorKind::RateLimited,
                            "Claude usage is rate limited",
                        ),
                    });
                }
                if response.status().is_success() {
                    if let Ok(raw) = response.json::<Value>().await {
                        let windows = normalize_claude_usage(&raw);
                        if !windows.is_empty() {
                            return Ok(self.snapshot(windows, context.now, UsageSource::Oauth));
                        }
                    }
                }
            }
        }
        let output = run_pty(PtyOptions {
            executable: &executable,
            args: &[],
            input: "/usage",
            write_delay: Duration::from_secs(1),
            completion_delay: Duration::from_millis(600),
            timeout: Duration::from_secs(25),
            cancellation: context.cancellation.clone(),
            complete: Arc::new(|value| {
                Regex::new(r"(?i)(?:current\s+session|5h|weekly)[\s\S]*?\d+(?:\.\d+)?%")
                    .unwrap()
                    .is_match(value)
            }),
        })
        .await?;
        let windows = parse_claude_usage(&output, context.now);
        if windows.is_empty() {
            Err(ProviderError::new(
                ProviderErrorKind::Parse,
                "Could not parse Claude usage",
            ))
        } else {
            Ok(self.snapshot(windows, context.now, UsageSource::Cli))
        }
    }
}
