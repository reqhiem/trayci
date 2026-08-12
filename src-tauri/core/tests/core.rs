use async_trait::async_trait;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::Semaphore;
use trayci_core::{
    cli::run_cli,
    service::{refresh_reason, should_fetch, STALE_AFTER_MS},
    BuiltInProviderSettingsPatch, CacheRepository, NotificationSettings, ProviderDetection,
    ProviderDetectionStatus, ProviderError, ProviderErrorKind, ProviderSettingsPatch,
    ProviderUsageSnapshot, QuotaNotifier, SettingsRepository, Theme, TrayciSettings,
    TrayciSettingsPatch, UsageFetchContext, UsageFetchReason, UsageProvider, UsageService,
    UsageSource, UsageState, UsageStatus, UsageWindow,
};

fn snapshot(percent: f64, status: UsageStatus) -> ProviderUsageSnapshot {
    ProviderUsageSnapshot {
        provider: "claude".into(),
        display_name: "Claude".into(),
        status,
        plan: None,
        windows: vec![UsageWindow {
            id: "session".into(),
            label: "Session".into(),
            used_percent: percent,
            duration_minutes: Some(300),
            resets_at: Some(2_000),
            reset_description: None,
        }],
        updated_at: 1_000,
        source: Some(UsageSource::Oauth),
        error: None,
    }
}

#[test]
fn schema_v1_settings_match_the_typescript_contract() {
    let json = serde_json::to_value(TrayciSettings::default()).unwrap();

    assert_eq!(json["schemaVersion"], 1);
    assert_eq!(json["displayMode"], "detailed");
    assert_eq!(json["percentageDisplay"], "used");
    assert!(json["providers"]["antigravity"]["executablePath"].is_null());
    assert!(serde_json::from_value::<TrayciSettings>(json).is_ok());
}

#[tokio::test]
async fn settings_validate_and_persist_privately() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    let mut repository = SettingsRepository::new(&path);
    repository.load().await;
    let settings = repository
        .update(TrayciSettingsPatch {
            poll_interval_minutes: Some(5),
            providers: Some(BuiltInProviderSettingsPatch {
                claude: Some(ProviderSettingsPatch {
                    executable_path: Some(Some("/usr/bin/claude".into())),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(settings.schema_version, 1);
    #[cfg(unix)]
    assert_eq!(
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o600
    );

    assert!(repository
        .update(TrayciSettingsPatch {
            poll_interval_minutes: Some(4),
            ..Default::default()
        })
        .await
        .is_err());

    let patch: ProviderSettingsPatch = serde_json::from_str(r#"{"executablePath":null}"#).unwrap();
    assert_eq!(patch.executable_path, Some(None));
}

#[tokio::test]
async fn settings_written_before_theme_and_font_scale_keep_their_values() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.json");
    let mut before = serde_json::to_value(TrayciSettings::default()).unwrap();
    before["startOnLogin"] = true.into();
    before["pollIntervalMinutes"] = 30.into();
    before.as_object_mut().unwrap().remove("theme");
    before.as_object_mut().unwrap().remove("fontScale");
    tokio::fs::write(&path, serde_json::to_vec(&before).unwrap())
        .await
        .unwrap();

    let settings = SettingsRepository::new(&path).load().await;
    assert!(settings.start_on_login, "an old config must not be reset");
    assert_eq!(settings.poll_interval_minutes, 30);
    assert_eq!(settings.theme, Theme::Dark);
    assert_eq!(settings.font_scale, 1.0);

    let mut repository = SettingsRepository::new(&path);
    repository.load().await;
    assert!(repository
        .update(TrayciSettingsPatch {
            font_scale: Some(3.0),
            ..Default::default()
        })
        .await
        .is_err());
}

#[tokio::test]
async fn cache_loads_valid_entries_and_ages_them_into_stale() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.json");
    let repository = CacheRepository::new(&path);
    let mut providers = HashMap::new();
    providers.insert("claude".into(), snapshot(130.0, UsageStatus::Ok));
    repository.save(&providers).await.unwrap();

    let fresh = repository.load(1_000 + STALE_AFTER_MS).await;
    assert_eq!(fresh["claude"].status, UsageStatus::Ok);
    assert_eq!(fresh["claude"].source, Some(UsageSource::Cache));
    assert_eq!(fresh["claude"].windows[0].used_percent, 100.0);

    let stale = repository.load(1_001 + STALE_AFTER_MS).await;
    assert_eq!(stale["claude"].status, UsageStatus::Stale);
}

#[test]
fn notifications_emit_only_the_highest_new_threshold() {
    let mut notifier = QuotaNotifier::default();
    let settings = NotificationSettings {
        quota50: true,
        quota85: true,
        quota90: true,
        reset: true,
    };
    let mut state = UsageState::default();
    state
        .providers
        .insert("claude".into(), snapshot(96.0, UsageStatus::Ok));
    let requests = notifier.update(&state, &settings, 1_500);
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].title, "Claude quota at 90%");
    assert!(notifier.update(&state, &settings, 1_500).is_empty());
}

struct FailsAfterSuccess;

#[async_trait]
impl UsageProvider for FailsAfterSuccess {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn display_name(&self) -> &'static str {
        "Claude"
    }

    async fn detect(&self) -> ProviderDetection {
        ProviderDetection {
            provider: self.id().into(),
            status: ProviderDetectionStatus::Available,
            executable_path: None,
        }
    }

    async fn fetch_usage(
        &self,
        context: &UsageFetchContext,
    ) -> Result<ProviderUsageSnapshot, ProviderError> {
        if context.reason == UsageFetchReason::Startup {
            let mut value = snapshot(42.0, UsageStatus::Ok);
            value.updated_at = context.now;
            Ok(value)
        } else {
            Err(ProviderError::new(
                ProviderErrorKind::Network,
                "secret upstream detail",
            ))
        }
    }
}

#[tokio::test]
async fn service_preserves_recent_success_as_stale_with_public_error() {
    let directory = tempfile::tempdir().unwrap();
    let service = Arc::new(UsageService::with_cache(
        vec![Box::new(FailsAfterSuccess)],
        TrayciSettings::default,
        CacheRepository::new(directory.path().join("cache.json")),
    ));
    let fresh = service
        .refresh_provider("claude", UsageFetchReason::Startup)
        .await
        .unwrap();
    assert_eq!(fresh.status, UsageStatus::Ok);

    let stale = service
        .refresh_provider("claude", UsageFetchReason::Manual)
        .await
        .unwrap();
    assert_eq!(stale.status, UsageStatus::Stale);
    assert_eq!(stale.error.as_deref(), Some("Usage unavailable"));
}

#[tokio::test]
async fn cli_json_uses_the_typescript_contract_without_internal_errors() {
    let providers: Vec<Box<dyn UsageProvider>> = vec![Box::new(FailsAfterSuccess)];
    let (code, output) = run_cli(&["usage".into(), "--json".into()], &providers, "test").await;
    let value: serde_json::Value = serde_json::from_str(&output).unwrap();

    assert_eq!(code, 1);
    assert_eq!(value["providers"]["claude"]["displayName"], "Claude");
    assert!(value["providers"]["claude"]["updatedAt"].is_number());
    assert_eq!(value["providers"]["claude"]["error"], "Usage unavailable");
    assert!(!output.contains("secret upstream detail"));
}

/// A provider under the test's control: it counts its calls, answers from a script, and only
/// answers once the test hands it a permit, so nothing here depends on a real account or a clock.
struct Scripted {
    id: &'static str,
    calls: Arc<AtomicUsize>,
    gate: Arc<Semaphore>,
    answer: Box<dyn Fn(usize) -> Result<ProviderUsageSnapshot, ProviderError> + Send + Sync>,
}

impl Scripted {
    fn new(
        id: &'static str,
        answer: impl Fn(usize) -> Result<ProviderUsageSnapshot, ProviderError> + Send + Sync + 'static,
    ) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                id,
                calls: Arc::clone(&calls),
                gate: Arc::new(Semaphore::new(Semaphore::MAX_PERMITS)),
                answer: Box::new(answer),
            },
            calls,
        )
    }

    fn gated(mut self, gate: Arc<Semaphore>) -> Self {
        self.gate = gate;
        self
    }
}

#[async_trait]
impl UsageProvider for Scripted {
    fn id(&self) -> &'static str {
        self.id
    }

    fn display_name(&self) -> &'static str {
        "Scripted"
    }

    async fn detect(&self) -> ProviderDetection {
        ProviderDetection {
            provider: self.id().into(),
            status: ProviderDetectionStatus::Available,
            executable_path: None,
        }
    }

    async fn fetch_usage(
        &self,
        context: &UsageFetchContext,
    ) -> Result<ProviderUsageSnapshot, ProviderError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        self.gate.acquire().await.unwrap().forget();
        (self.answer)(call).map(|mut value| {
            value.provider = self.id().into();
            value.updated_at = context.now;
            value
        })
    }
}

fn service(providers: Vec<Box<dyn UsageProvider>>, cache: &std::path::Path) -> Arc<UsageService> {
    Arc::new(UsageService::with_cache(
        providers,
        TrayciSettings::default,
        CacheRepository::new(cache),
    ))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

async fn write_cache(path: &std::path::Path, age_ms: u64) {
    let mut cached = snapshot(42.0, UsageStatus::Ok);
    cached.updated_at = now_ms() - age_ms;
    CacheRepository::new(path)
        .save(&HashMap::from([("claude".to_string(), cached)]))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_warm_cache_starts_without_a_single_provider_request() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.json");
    write_cache(&path, 60_000).await;
    let (provider, calls) = Scripted::new("claude", |_| Ok(snapshot(1.0, UsageStatus::Ok)));
    let service = service(vec![Box::new(provider)], &path);

    service.start().await;
    let state = service.get_state().await;
    service.stop().await;

    assert_eq!(calls.load(Ordering::SeqCst), 0, "the cache was still fresh");
    assert_eq!(state.providers["claude"].status, UsageStatus::Ok);
    assert_eq!(state.providers["claude"].source, Some(UsageSource::Cache));
}

#[tokio::test]
async fn a_cache_older_than_the_poll_interval_is_refreshed_on_startup() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.json");
    write_cache(&path, 2 * 60 * 60_000).await;
    let (provider, calls) = Scripted::new("claude", |_| Ok(snapshot(7.0, UsageStatus::Ok)));
    let service = service(vec![Box::new(provider)], &path);

    service.start().await;
    let state = service.get_state().await;
    service.stop().await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(state.providers["claude"].source, Some(UsageSource::Oauth));
    assert_eq!(state.providers["claude"].windows[0].used_percent, 7.0);
}

#[tokio::test]
async fn a_retry_recovers_its_own_provider_without_calling_the_healthy_one() {
    let directory = tempfile::tempdir().unwrap();
    let (failing, failing_calls) = Scripted::new("claude", |call| {
        if call == 0 {
            Err(ProviderError::new(ProviderErrorKind::Network, "offline").retry_at(0))
        } else {
            Ok(snapshot(12.0, UsageStatus::Ok))
        }
    });
    let (healthy, healthy_calls) = Scripted::new("codex", |_| Ok(snapshot(3.0, UsageStatus::Ok)));
    let service = service(
        vec![Box::new(failing), Box::new(healthy)],
        &directory.path().join("cache.json"),
    );

    let first = service.refresh_all(UsageFetchReason::Startup).await;
    assert_eq!(first.providers["claude"].status, UsageStatus::Error);
    assert_eq!(
        first.providers["claude"].error.as_deref(),
        Some("Usage unavailable")
    );
    assert_eq!(healthy_calls.load(Ordering::SeqCst), 1);

    let recovered = service.refresh_all(UsageFetchReason::Retry).await;
    assert_eq!(recovered.providers["claude"].status, UsageStatus::Ok);
    assert_eq!(failing_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        healthy_calls.load(Ordering::SeqCst),
        1,
        "a retry cycle must not drag a healthy provider onto the network"
    );

    service.refresh_all(UsageFetchReason::Retry).await;
    assert_eq!(
        failing_calls.load(Ordering::SeqCst),
        2,
        "nothing is owed a retry"
    );
}

#[tokio::test]
async fn concurrent_manual_refreshes_collapse_into_one_extra_cycle() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.json");
    write_cache(&path, 60_000).await;
    let gate = Arc::new(Semaphore::new(0));
    let (provider, calls) = Scripted::new("claude", |_| Ok(snapshot(5.0, UsageStatus::Ok)));
    let service = service(vec![Box::new(provider.gated(Arc::clone(&gate)))], &path);
    service.start().await;

    // The test runtime is single threaded, so `join!` parks the first refresh on the gate and lets
    // the other two queue behind it before the last future opens the gate.
    tokio::join!(
        service.refresh_all(UsageFetchReason::Manual),
        service.refresh_all(UsageFetchReason::Manual),
        service.refresh_all(UsageFetchReason::Manual),
        async { gate.add_permits(8) },
    );
    service.stop().await;

    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "two refreshes waiting on a third collapse into a single follow-up cycle"
    );
}

#[test]
fn a_resume_only_refreshes_when_the_setting_allows_it() {
    let gap = 5 * 60_000;
    assert_eq!(
        refresh_reason(gap, true, false, false),
        Some(UsageFetchReason::Resume)
    );
    assert_eq!(refresh_reason(gap, false, false, false), None);
    assert_eq!(
        refresh_reason(gap, false, false, true),
        Some(UsageFetchReason::Poll),
        "a disabled resume still polls on its own schedule"
    );
    assert_eq!(
        refresh_reason(1_000, true, true, true),
        Some(UsageFetchReason::Retry)
    );
    assert_eq!(refresh_reason(1_000, true, false, false), None);
}

#[test]
fn only_a_manual_refresh_overrides_a_backoff_we_invented() {
    let interval = 15 * 60_000;
    let ours = Some((9_000, false));
    let theirs = Some((9_000, true));

    assert!(should_fetch(
        UsageFetchReason::Manual,
        ours,
        None,
        1_000,
        interval
    ));
    assert!(!should_fetch(
        UsageFetchReason::Manual,
        theirs,
        None,
        1_000,
        interval
    ));
    assert!(!should_fetch(
        UsageFetchReason::Poll,
        ours,
        None,
        1_000,
        interval
    ));
    assert!(should_fetch(
        UsageFetchReason::Retry,
        theirs,
        None,
        9_000,
        interval
    ));

    let fresh = Some(1_000);
    assert!(!should_fetch(
        UsageFetchReason::Poll,
        None,
        fresh,
        1_000 + interval - 1,
        interval
    ));
    assert!(should_fetch(
        UsageFetchReason::Poll,
        None,
        fresh,
        1_000 + interval,
        interval
    ));
    assert!(should_fetch(
        UsageFetchReason::Startup,
        None,
        None,
        1_000,
        interval
    ));
}

#[tokio::test]
async fn doctor_keeps_the_electron_cli_contract() {
    let providers: Vec<Box<dyn UsageProvider>> = vec![Box::new(FailsAfterSuccess)];
    let (code, output) = run_cli(&["doctor".into()], &providers, "test").await;

    assert_eq!(code, 0);
    assert_eq!(output, "claude: available\nactive provider probes: 0\n");
}
