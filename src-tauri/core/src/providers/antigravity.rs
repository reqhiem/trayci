use super::{
    common::{
        clamp, epoch_ms, home_dir, rate_limited, reset_from_text, reset_phrase, resolve_executable,
        run_process, run_pty, PtyOptions,
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
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, LazyLock,
    },
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
                group: None,
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

/// `agy` answers `/usage` in print mode as a command from this version on. Older builds send the
/// text to the model as a prompt, so every poll would start a conversation and spend quota.
const MIN_PRINT_USAGE: (u64, u64, u64) = (1, 1, 11);
/// A due update check makes `agy` start its background updater; a quota read must not update the
/// CLI. Only `true` disables it.
const PROBE_ENV: &[(&str, &str)] = &[("AGY_CLI_DISABLE_AUTO_UPDATE", "true")];
/// Latched once `agy` answers the print-mode read with a model turn. The evidence is the same event
/// that spends quota, so the read is not retried for the rest of the process's life.
static PRINT_USAGE_UNSUPPORTED: AtomicBool = AtomicBool::new(false);

pub fn parse_agy_version(output: &str) -> Option<(u64, u64, u64)> {
    static VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)\.(\d+)\.(\d+)").unwrap());
    let captures = VERSION.captures(output)?;
    Some((
        captures[1].parse().ok()?,
        captures[2].parse().ok()?,
        captures[3].parse().ok()?,
    ))
}

/// "Gemini Models" reads "Gemini" and "Claude and GPT models" reads "Claude/GPT"; a pool named
/// after one model ("Gemini 4 Argon") keeps its name.
fn pool_name(group: &str) -> String {
    let trimmed = group.trim();
    let stem = trimmed
        .len()
        .checked_sub(" models".len())
        .filter(|&at| trimmed.is_char_boundary(at) && trimmed[at..].eq_ignore_ascii_case(" models"))
        .map_or(trimmed, |at| &trimmed[..at]);
    stem.replace(" and ", "/")
}

/// The window's own name: the two lengths agy meters, or its label without the pool prefix and
/// the "Limit Remaining" suffix, so an unknown window is named rather than forced into one of them.
fn window_name(duration: Option<u64>, bucket: &str, group: &str) -> String {
    match duration {
        Some(300) => "5h".into(),
        Some(10_080) => "Weekly".into(),
        _ => {
            let name = bucket
                .get(..group.len())
                .filter(|prefix| prefix.eq_ignore_ascii_case(group))
                .map_or(bucket, |_| &bucket[group.len()..])
                .trim();
            let name = ["Limit Remaining", "Limit"]
                .iter()
                .find_map(|suffix| name.strip_suffix(suffix))
                .unwrap_or(name)
                .trim();
            if name.is_empty() {
                bucket.trim().into()
            } else {
                name.into()
            }
        }
    }
}

fn usage_envelope(stdout: &str) -> impl Iterator<Item = Value> + '_ {
    stdout
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('{'))
        .filter_map(|line| serde_json::from_str(line).ok())
}

/// Reads what `agy -p /usage --output-format json` prints: an envelope whose `command.data.groups`
/// lists every quota pool and its buckets. Every group is kept, so a new pool appears without a
/// code change. `None` means the reply is not a successful usage reading, which is not the same
/// as an empty one.
pub fn parse_antigravity_json(stdout: &str) -> Option<Vec<UsageWindow>> {
    usage_envelope(stdout).find_map(|value| {
        if value.get("status").and_then(Value::as_str) != Some("SUCCESS") {
            return None;
        }
        let command = value.get("command")?;
        // `/usage` and `/quota` both answer as `usage`; the name proves this is a quota reply.
        if command.get("name").and_then(Value::as_str) != Some("usage") {
            return None;
        }
        let mut windows = Vec::new();
        for group in command.pointer("/data/groups")?.as_array()? {
            let Some(name) = group.get("name").and_then(Value::as_str) else {
                continue;
            };
            let models = group
                .get("description")
                .and_then(Value::as_str)
                .and_then(|text| text.strip_prefix("Models within this group:"))
                .map(|text| text.trim().to_owned())
                .filter(|text| !text.is_empty());
            // A disabled bucket is one the tier does not meter; drawing it as 0% used would report
            // headroom the account does not have.
            let mut buckets = group
                .get("buckets")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .filter(|bucket| bucket.get("disabled").and_then(Value::as_bool) != Some(true))
                .filter_map(|bucket| {
                    let duration = match bucket.get("window").and_then(Value::as_str) {
                        Some("5h") => Some(300),
                        Some("weekly") => Some(10_080),
                        _ => None,
                    };
                    let remaining = bucket
                        .get("remaining_fraction")
                        .and_then(Value::as_f64)
                        .filter(|value| value.is_finite())?;
                    Some((
                        bucket.get("id").and_then(Value::as_str)?.to_owned(),
                        bucket.get("name").and_then(Value::as_str).unwrap_or(name),
                        duration,
                        clamp(((1.0 - remaining) * 100.0).round()),
                        bucket.get("reset_time").and_then(epoch_ms),
                    ))
                })
                .collect::<Vec<_>>();
            // Shortest window first, the order the other providers use.
            buckets.sort_by_key(|bucket| bucket.2.unwrap_or(u64::MAX));
            let pool = pool_name(name);
            let single = buckets.len() == 1;
            for (id, bucket, duration, used, resets) in buckets {
                windows.push(UsageWindow {
                    id,
                    label: if single {
                        pool.clone()
                    } else {
                        format!("{pool} {}", window_name(duration, bucket, name))
                    },
                    used_percent: used,
                    duration_minutes: duration,
                    resets_at: resets,
                    reset_description: None,
                    group: Some(UsageGroup {
                        name: name.to_owned(),
                        models: models.clone(),
                    }),
                });
            }
        }
        (!windows.is_empty()).then_some(windows)
    })
}

/// True when the reply shows `agy` ran a model turn instead of answering a command. A real command
/// reply has no conversation and no turns; a prompt has both.
pub fn ran_model_turn(stdout: &str) -> bool {
    usage_envelope(stdout).any(|value| {
        value
            .get("num_turns")
            .and_then(Value::as_u64)
            .is_some_and(|turns| turns > 0)
            || value
                .get("conversation_id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
    })
}

/// Reads quota from `agy` in print mode, which spends nothing and needs no terminal. Any failure is
/// returned so the caller can fall back to the interactive panel.
async fn print_usage(
    executable: &Path,
    cancellation: &tokio_util::sync::CancellationToken,
) -> Result<Vec<UsageWindow>, ProviderError> {
    let unsupported = || {
        ProviderError::new(
            ProviderErrorKind::Parse,
            "This agy cannot report usage in print mode",
        )
    };
    if PRINT_USAGE_UNSUPPORTED.load(Ordering::Relaxed) {
        return Err(unsupported());
    }
    // Rechecked on every read: the CLI can be replaced while Trayci runs, and an unsupported read
    // costs quota.
    let version = run_process(
        executable,
        &["--version"],
        PROBE_ENV,
        Duration::from_secs(5),
        cancellation,
    )
    .await?;
    let version = parse_agy_version(&String::from_utf8_lossy(&version.stdout))
        .or_else(|| parse_agy_version(&String::from_utf8_lossy(&version.stderr)));
    if version.map_or(true, |version| version < MIN_PRINT_USAGE) {
        return Err(unsupported());
    }
    let output = run_process(
        executable,
        // Never add --disable-slash-commands: it turns `/usage` into a billed prompt.
        &[
            "-p",
            "/usage",
            "--output-format",
            "json",
            "--print-timeout",
            "20s",
        ],
        PROBE_ENV,
        Duration::from_secs(30),
        cancellation,
    )
    .await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if let Some(windows) = parse_antigravity_json(&stdout) {
        return Ok(windows);
    }
    if ran_model_turn(&stdout) {
        PRINT_USAGE_UNSUPPORTED.store(true, Ordering::Relaxed);
        return Err(unsupported());
    }
    Err(ProviderError::new(
        ProviderErrorKind::Parse,
        "Could not parse Antigravity usage",
    ))
}

fn quota_group(
    output: &str,
    heading: &Regex,
    next: Option<&Regex>,
    id: &str,
    name: &str,
    group: &str,
    now: u64,
) -> Vec<UsageWindow> {
    let Some(start) = heading.find(output).map(|value| value.start()) else {
        return vec![];
    };
    let remainder = &output[start..];
    let end = next
        .and_then(|pattern| pattern.find(&remainder[1..]).map(|value| value.start() + 1))
        .unwrap_or(remainder.len());
    let models = remainder[..end].lines().find_map(|line| {
        line.trim()
            .strip_prefix("Models within this group:")
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
    });
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
            group: Some(UsageGroup {
                name: group.into(),
                models: models.clone(),
            }),
        })
    })
    .collect()
}

pub fn parse_antigravity_usage(output: &str, now: u64) -> (Vec<UsageWindow>, Option<String>) {
    static GEMINI: LazyLock<Regex> = LazyLock::new(|| Regex::new("(?i)GEMINI MODELS").unwrap());
    static CLAUDE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new("(?i)CLAUDE AND GPT MODELS").unwrap());
    let windows = quota_group(
        output,
        &GEMINI,
        Some(&CLAUDE),
        "gemini",
        "Gemini",
        "Gemini Models",
        now,
    )
    .into_iter()
    .chain(quota_group(
        output,
        &CLAUDE,
        None,
        "claude-gpt",
        "Claude/GPT",
        "Claude and GPT models",
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
            match print_usage(&executable, &context.cancellation).await {
                // The JSON reply carries no plan name; only the interactive panel prints it.
                Ok(windows) => return Ok(snapshot(windows, None, context.now, UsageSource::Cli)),
                Err(error) if error.kind == ProviderErrorKind::Aborted => return Err(error),
                Err(_) => {}
            }
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
            return Err(rate_limited(
                response.headers(),
                context.now,
                "Antigravity usage is rate limited",
            ));
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
