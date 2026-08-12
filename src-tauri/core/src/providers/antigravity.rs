use super::{
    common::{
        clamp, epoch_ms, home_dir, reset_from_text, reset_phrase, resolve_executable, run_pty,
        PtyOptions,
    },
    google_oauth,
};
use crate::{model::*, service::UsageProvider};
use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    env, fs,
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::Duration,
};

const CODE_ASSIST: &str = "https://cloudcode-pa.googleapis.com/v1internal";
type Settings = Arc<dyn Fn() -> TrayciSettings + Send + Sync>;
pub struct AntigravityProvider {
    settings: Settings,
}
impl AntigravityProvider {
    pub fn new(settings: impl Fn() -> TrayciSettings + Send + Sync + 'static) -> Self {
        Self {
            settings: Arc::new(settings),
        }
    }
}

#[derive(Clone)]
struct Credentials {
    access: String,
    refresh: String,
    expiry: u64,
}
fn credentials_path() -> Option<PathBuf> {
    env::var_os("GEMINI_CLI_HOME")
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|path| path.join(".gemini")))
        .map(|path| path.join("oauth_creds.json"))
}
fn credentials() -> Option<Credentials> {
    let value: Value = serde_json::from_slice(&fs::read(credentials_path()?).ok()?).ok()?;
    Some(Credentials {
        access: value.get("access_token")?.as_str()?.into(),
        refresh: value.get("refresh_token")?.as_str()?.into(),
        expiry: value.get("expiry_date")?.as_u64()?,
    })
}

fn label(model: &str) -> (String, bool) {
    let known = HashMap::from([
        ("gemini-3.1-pro", "3.1 Pro"),
        ("gemini-3.1-flash", "3.1 Flash"),
        ("gemini-3.1-flash-lite", "3.1 Flash Lite"),
        ("gemini-3.0-pro", "3.0 Pro"),
        ("gemini-3.0-flash", "3.0 Flash"),
        ("gemini-2.5-pro", "Pro"),
        ("gemini-2.5-flash", "Flash"),
        ("gemini-2.5-flash-lite", "Flash Lite"),
        ("gemini-2.0-pro", "2.0 Pro"),
        ("gemini-2.0-flash", "2.0 Flash"),
        ("gemini-1.5-pro", "1.5 Pro"),
        ("gemini-1.5-flash", "1.5 Flash"),
    ]);
    known.get(model).map_or_else(
        || {
            (
                model
                    .trim_start_matches("gemini-")
                    .split('-')
                    .map(|part| {
                        let mut chars = part.chars();
                        chars.next().map_or_else(String::new, |first| {
                            first.to_uppercase().collect::<String>() + chars.as_str()
                        })
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                false,
            )
        },
        |value| ((*value).into(), true),
    )
}

pub fn normalize_antigravity_quota(value: &Value) -> Vec<UsageWindow> {
    let candidates = value
        .as_array()
        .or_else(|| value.get("buckets").and_then(Value::as_array))
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut windows: Vec<(UsageWindow, bool)> = Vec::new();
    let mut seen = HashMap::new();
    for bucket in candidates {
        let reset_val = bucket
            .get("resetTime")
            .or_else(|| bucket.get("reset_time"))
            .or_else(|| bucket.get("resetsAt"))
            .or_else(|| bucket.get("reset"));
        let (Some(remaining), Some(model)) = (
            bucket
                .get("remainingFraction")
                .and_then(Value::as_f64)
                .filter(|v| v.is_finite()),
            bucket.get("modelId").and_then(Value::as_str),
        ) else {
            continue;
        };
        let used = clamp((1.0 - remaining) * 100.0).round();
        let resets = reset_val.and_then(epoch_ms);
        let reset_desc = reset_val.and_then(|v| v.as_str()).map(str::to_owned);
        let (name, known) = label(model);
        let item = (
            UsageWindow {
                id: format!("model-{model}"),
                label: name,
                used_percent: used,
                duration_minutes: Some(60),
                resets_at: resets,
                reset_description: reset_desc,
            },
            known,
        );
        let key = (used as i64, resets);
        if let Some(index) = seen.get(&key).copied() {
            let previous: &(UsageWindow, bool) = &windows[index];
            if (known && !previous.1)
                || (known == previous.1 && item.0.label.len() < previous.0.label.len())
            {
                windows[index] = item;
            }
        } else {
            seen.insert(key, windows.len());
            windows.push(item);
        }
    }
    windows.into_iter().map(|(window, _)| window).collect()
}

fn quota_group(
    output: &str,
    heading: &Regex,
    next: Option<&Regex>,
    id: &str,
    name: &str,
    now: u64,
) -> Vec<UsageWindow> {
    let Some(start) = heading.find(output).map(|value| value.start()) else {
        return vec![];
    };
    let remainder = &output[start..];
    let end = next
        .and_then(|pattern| pattern.find(&remainder[1..]).map(|value| value.start() + 1))
        .unwrap_or(remainder.len());
    let section = remainder[..end]
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    [
        ("session", "5h", 300, "Five Hour"),
        ("weekly", "Weekly", 10_080, "Weekly"),
    ]
    .into_iter()
    .filter_map(|(window_id, window_name, duration, heading)| {
        // Group 2 is everything up to the next window, where the reset copy lives:
        // "84% remaining · Refreshes in 1h 49m".
        let pattern = format!(
            r"(?i){heading} Limit Remaining[^%]{{0,200}}?(\d+(?:\.\d+)?)%(.*?)(?:Limit Remaining|$)"
        );
        let captures = Regex::new(&pattern).unwrap().captures(&section)?;
        let reset = reset_phrase(captures.get(2).map_or("", |value| value.as_str()));
        Some(UsageWindow {
            id: format!("{id}-{window_id}"),
            label: format!("{name} {window_name}"),
            used_percent: clamp(100.0 - captures[1].parse::<f64>().ok()?),
            duration_minutes: Some(duration),
            resets_at: reset.as_deref().and_then(|text| reset_from_text(text, now)),
            reset_description: reset,
        })
    })
    .collect()
}

pub fn parse_antigravity_usage(output: &str, now: u64) -> (Vec<UsageWindow>, Option<String>) {
    static GEMINI: LazyLock<Regex> = LazyLock::new(|| Regex::new("(?i)GEMINI MODELS").unwrap());
    static CLAUDE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new("(?i)CLAUDE AND GPT MODELS").unwrap());
    let windows = quota_group(output, &GEMINI, Some(&CLAUDE), "gemini", "Gemini", now)
        .into_iter()
        .chain(quota_group(
            output,
            &CLAUDE,
            None,
            "claude-gpt",
            "Claude/GPT",
            now,
        ))
        .collect();
    let plan = Regex::new(r"(?i)\((Google AI [^)]+)\)")
        .unwrap()
        .captures(output)
        .map(|captures| captures[1].to_owned());
    (windows, plan)
}

async fn refresh_token(
    refresh: &str,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<String, ProviderError> {
    let (client_id, client_secret) = google_oauth::extract(None).await.ok_or_else(|| {
        ProviderError::new(
            ProviderErrorKind::NotAuthenticated,
            "Gemini CLI OAuth metadata not found",
        )
    })?;
    let request = client().post("https://oauth2.googleapis.com/token").form(&[
        ("client_id", client_id.as_str()),
        ("client_secret", client_secret.as_str()),
        ("refresh_token", refresh),
        ("grant_type", "refresh_token"),
    ]);
    let response = send(request, cancellation).await?;
    if !response.status().is_success() {
        return Err(ProviderError::new(
            ProviderErrorKind::NotAuthenticated,
            "Google OAuth token refresh failed",
        ));
    }
    response
        .json::<Value>()
        .await
        .ok()
        .and_then(|value| {
            value
                .get("access_token")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::Parse,
                "Google OAuth token response changed",
            )
        })
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}
async fn send(
    request: reqwest::RequestBuilder,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<reqwest::Response, ProviderError> {
    tokio::select! { _ = cancellation.cancelled() => Err(ProviderError::new(ProviderErrorKind::Aborted, "Cancelled")), response = request.send() => response.map_err(|_| ProviderError::new(ProviderErrorKind::Network, "Google quota request failed")) }
}
async fn api(
    path: &str,
    token: &str,
    body: Value,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<reqwest::Response, ProviderError> {
    send(
        client()
            .post(format!("{CODE_ASSIST}:{path}"))
            .bearer_auth(token)
            .header("user-agent", concat!("trayci/", env!("CARGO_PKG_VERSION")))
            .json(&body),
        cancellation,
    )
    .await
}
async fn quota(
    token: &str,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<reqwest::Response, ProviderError> {
    let project_response = api(
        "loadCodeAssist",
        token,
        json!({"metadata":{"ideType":"GEMINI_CLI","pluginType":"GEMINI"}}),
        cancellation,
    )
    .await?;
    if !project_response.status().is_success() {
        return Ok(project_response);
    }
    let project = project_response
        .json::<Value>()
        .await
        .ok()
        .and_then(|value| {
            value
                .get("cloudaicompanionProject")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .ok_or_else(|| {
            ProviderError::new(
                ProviderErrorKind::Parse,
                "Google Code Assist project not found",
            )
        })?;
    api(
        "retrieveUserQuota",
        token,
        json!({"project":project}),
        cancellation,
    )
    .await
}

#[async_trait]
impl UsageProvider for AntigravityProvider {
    fn id(&self) -> &'static str {
        "antigravity"
    }
    fn display_name(&self) -> &'static str {
        "Antigravity"
    }
    async fn detect(&self) -> ProviderDetection {
        let path = resolve_executable(
            "agy",
            (self.settings)()
                .providers
                .antigravity
                .executable_path
                .as_deref(),
        )
        .await;
        ProviderDetection {
            provider: self.id().into(),
            status: if path.is_some() || credentials().is_some() {
                ProviderDetectionStatus::Available
            } else {
                ProviderDetectionStatus::NotInstalled
            },
            executable_path: path,
        }
    }
    async fn fetch_usage(
        &self,
        context: &UsageFetchContext,
    ) -> Result<ProviderUsageSnapshot, ProviderError> {
        let detection = self.detect().await;
        let mut cli_error = None;
        if let Some(executable) = detection.executable_path {
            match run_pty(PtyOptions {
                executable: &executable,
                args: &[],
                input: "/usage",
                write_delay: Duration::from_millis(2500),
                completion_delay: Duration::from_millis(1200),
                timeout: Duration::from_secs(20),
                cancellation: context.cancellation.clone(),
                complete: Arc::new(|value| {
                    Regex::new(r"(?i)Limit Remaining[\s\S]{0,200}?\d+(?:\.\d+)?%")
                        .unwrap()
                        .find_iter(value)
                        .count()
                        >= 4
                }),
            })
            .await
            {
                Ok(output) => {
                    let (windows, plan) = parse_antigravity_usage(&output, context.now);
                    if windows.len() == 4 {
                        return Ok(snapshot(windows, plan, context.now, UsageSource::Cli));
                    }
                    cli_error = Some(ProviderError::new(
                        ProviderErrorKind::Parse,
                        "Could not parse Antigravity usage",
                    ));
                }
                Err(error) if error.kind == ProviderErrorKind::Aborted => return Err(error),
                Err(error) => cli_error = Some(error),
            }
        }
        let credentials = match credentials() {
            Some(value) => value,
            None => {
                return Err(cli_error.unwrap_or_else(|| {
                    ProviderError::new(
                        ProviderErrorKind::NotInstalled,
                        "Antigravity CLI not detected",
                    )
                }))
            }
        };
        let original = credentials.access.clone();
        let mut access = if credentials.expiry > context.now {
            original.clone()
        } else {
            refresh_token(&credentials.refresh, &context.cancellation).await?
        };
        let mut response = quota(&access, &context.cancellation).await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED && access == original {
            access = refresh_token(&credentials.refresh, &context.cancellation).await?;
            response = quota(&access, &context.cancellation).await?;
        }
        if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            let retry = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            return Err(match retry {
                Some(seconds) => ProviderError::new(
                    ProviderErrorKind::RateLimited,
                    "Antigravity usage is rate limited",
                )
                .retry_at(context.now + seconds * 1000),
                None => ProviderError::new(
                    ProviderErrorKind::RateLimited,
                    "Antigravity usage is rate limited",
                ),
            });
        }
        if !response.status().is_success() {
            return Err(ProviderError::new(
                if matches!(
                    response.status(),
                    reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
                ) {
                    ProviderErrorKind::NotAuthenticated
                } else {
                    ProviderErrorKind::Network
                },
                "Antigravity quota request failed",
            ));
        }
        let windows =
            normalize_antigravity_quota(&response.json::<Value>().await.map_err(|_| {
                ProviderError::new(
                    ProviderErrorKind::Parse,
                    "Antigravity quota response changed",
                )
            })?);
        if windows.is_empty() {
            Err(ProviderError::new(
                ProviderErrorKind::Parse,
                "Antigravity returned no quota windows",
            ))
        } else {
            Ok(snapshot(windows, None, context.now, UsageSource::Oauth))
        }
    }
}

fn snapshot(
    windows: Vec<UsageWindow>,
    plan: Option<String>,
    now: u64,
    source: UsageSource,
) -> ProviderUsageSnapshot {
    ProviderUsageSnapshot {
        provider: "antigravity".into(),
        display_name: "Antigravity".into(),
        status: UsageStatus::Ok,
        plan,
        windows,
        updated_at: now,
        source: Some(source),
        error: None,
    }
}
