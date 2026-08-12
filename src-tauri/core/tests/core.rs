use async_trait::async_trait;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{collections::HashMap, sync::Arc};
use trayci_core::{
    cli::run_cli, BuiltInProviderSettingsPatch, CacheRepository, NotificationSettings,
    ProviderDetection, ProviderDetectionStatus, ProviderError, ProviderErrorKind,
    ProviderSettingsPatch, ProviderUsageSnapshot, QuotaNotifier, SettingsRepository,
    Theme, TrayciSettings, TrayciSettingsPatch, UsageFetchContext, UsageFetchReason, UsageProvider,
    UsageService, UsageSource, UsageState, UsageStatus, UsageWindow,
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
async fn cache_loads_valid_entries_as_normalized_stale_data() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cache.json");
    let repository = CacheRepository::new(&path);
    let mut providers = HashMap::new();
    providers.insert("claude".into(), snapshot(130.0, UsageStatus::Ok));
    repository.save(&providers).await.unwrap();

    let loaded = repository.load().await;
    let claude = &loaded["claude"];
    assert_eq!(claude.status, UsageStatus::Stale);
    assert_eq!(claude.source, Some(UsageSource::Cache));
    assert_eq!(claude.windows[0].used_percent, 100.0);
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

#[tokio::test]
async fn doctor_keeps_the_electron_cli_contract() {
    let providers: Vec<Box<dyn UsageProvider>> = vec![Box::new(FailsAfterSuccess)];
    let (code, output) = run_cli(&["doctor".into()], &providers, "test").await;

    assert_eq!(code, 0);
    assert_eq!(output, "claude: available\nactive provider probes: 0\n");
}
