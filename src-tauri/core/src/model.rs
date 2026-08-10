use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};
use tokio_util::sync::CancellationToken;

pub type ProviderId = String;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageStatus {
    #[default]
    Idle,
    Fetching,
    Ok,
    Stale,
    Error,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageSource {
    Oauth,
    Rpc,
    Api,
    Cli,
    Cache,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageFetchReason {
    Startup,
    Manual,
    Poll,
    Resume,
    PopoverOpen,
    Retry,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub id: String,
    pub label: String,
    pub used_percent: f64,
    pub duration_minutes: Option<u64>,
    pub resets_at: Option<u64>,
    pub reset_description: Option<String>,
}

impl UsageWindow {
    pub fn normalize(&mut self) {
        self.used_percent = if self.used_percent.is_finite() {
            self.used_percent.clamp(0.0, 100.0)
        } else {
            0.0
        };
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsageSnapshot {
    pub provider: ProviderId,
    pub display_name: String,
    pub status: UsageStatus,
    pub plan: Option<String>,
    pub windows: Vec<UsageWindow>,
    pub updated_at: u64,
    pub source: Option<UsageSource>,
    pub error: Option<String>,
}

impl ProviderUsageSnapshot {
    pub fn normalize(mut self) -> Self {
        self.windows.iter_mut().for_each(UsageWindow::normalize);
        self
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageState {
    pub providers: HashMap<ProviderId, ProviderUsageSnapshot>,
    pub is_refreshing: bool,
    pub last_refresh_started_at: Option<u64>,
    pub last_refresh_completed_at: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderDetectionStatus {
    Available,
    NotInstalled,
    NotAuthenticated,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDetection {
    pub provider: ProviderId,
    pub status: ProviderDetectionStatus,
    pub executable_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct UsageFetchContext {
    pub cancellation: CancellationToken,
    pub reason: UsageFetchReason,
    pub now: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderErrorKind {
    NotInstalled,
    NotAuthenticated,
    RateLimited,
    Timeout,
    Parse,
    Network,
    Aborted,
    Unknown,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ProviderError {
    pub kind: ProviderErrorKind,
    pub message: String,
    pub retry_at: Option<u64>,
}

impl ProviderError {
    pub fn new(kind: ProviderErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            retry_at: None,
        }
    }

    pub fn retry_at(mut self, timestamp_ms: u64) -> Self {
        self.retry_at = Some(timestamp_ms);
        self
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    #[default]
    Detailed,
    Compact,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PercentageDisplay {
    #[default]
    Used,
    Remaining,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationSettings {
    pub quota50: bool,
    pub quota85: bool,
    pub quota90: bool,
    pub reset: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSettings {
    pub enabled: bool,
    pub executable_path: Option<PathBuf>,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            executable_path: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BuiltInProviderSettings {
    pub claude: ProviderSettings,
    pub codex: ProviderSettings,
    pub antigravity: ProviderSettings,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrayciSettings {
    pub schema_version: u8,
    pub start_on_login: bool,
    pub refresh_on_resume: bool,
    pub poll_interval_minutes: u64,
    pub display_mode: DisplayMode,
    pub percentage_display: PercentageDisplay,
    pub notifications: NotificationSettings,
    pub providers: BuiltInProviderSettings,
    #[serde(default)]
    pub window_position: Option<(i32, i32)>,
}

impl Default for TrayciSettings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            start_on_login: false,
            refresh_on_resume: true,
            poll_interval_minutes: 15,
            display_mode: DisplayMode::Detailed,
            percentage_display: PercentageDisplay::Used,
            notifications: NotificationSettings::default(),
            providers: BuiltInProviderSettings::default(),
            window_position: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NotificationSettingsPatch {
    pub quota50: Option<bool>,
    pub quota85: Option<bool>,
    pub quota90: Option<bool>,
    pub reset: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderSettingsPatch {
    pub enabled: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_optional_option")]
    pub executable_path: Option<Option<PathBuf>>,
}

fn deserialize_optional_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuiltInProviderSettingsPatch {
    pub claude: Option<ProviderSettingsPatch>,
    pub codex: Option<ProviderSettingsPatch>,
    pub antigravity: Option<ProviderSettingsPatch>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrayciSettingsPatch {
    pub start_on_login: Option<bool>,
    pub refresh_on_resume: Option<bool>,
    pub poll_interval_minutes: Option<u64>,
    pub display_mode: Option<DisplayMode>,
    pub percentage_display: Option<PercentageDisplay>,
    pub notifications: Option<NotificationSettingsPatch>,
    pub providers: Option<BuiltInProviderSettingsPatch>,
    #[serde(default, deserialize_with = "deserialize_optional_option")]
    pub window_position: Option<Option<(i32, i32)>>,
}
