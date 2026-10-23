//! Drives the provider against a stand-in `agy`, in its own test binary so the spawned children
//! cannot be mistaken for zombies by the PTY reaping test.
#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt};
use tokio_util::sync::CancellationToken;
use trayci_core::{
    providers::AntigravityProvider, TrayciSettings, UsageFetchContext, UsageFetchReason,
    UsageProvider, UsageSource,
};

#[tokio::test]
async fn reads_usage_from_print_mode() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/antigravity/usage-print.json"
    );
    let log = directory.path().join("calls");
    let agy = directory.path().join("agy");
    fs::write(
        &agy,
        format!(
            "#!/bin/sh\necho \"$AGY_CLI_DISABLE_AUTO_UPDATE $*\" >> '{}'\nif [ \"$1\" = --version ]; then echo 'agy 1.4.2'; else cat '{fixture}'; fi\n",
            log.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&agy, fs::Permissions::from_mode(0o755)).unwrap();
    let mut settings = TrayciSettings::default();
    settings.providers.antigravity.executable_path = Some(agy);
    let provider = AntigravityProvider::new(move || settings.clone());

    let snapshot = provider
        .fetch_usage(&UsageFetchContext {
            cancellation: CancellationToken::new(),
            reason: UsageFetchReason::Manual,
            now: 0,
        })
        .await
        .unwrap();

    assert_eq!(snapshot.source, Some(UsageSource::Cli));
    assert_eq!(snapshot.plan, None);
    assert_eq!(snapshot.windows.len(), 5);
    assert_eq!(snapshot.windows[4].label, "Gemini 4 Argon");
    assert_eq!(
        fs::read_to_string(log).unwrap(),
        "true --version\ntrue -p /usage --output-format json --print-timeout 20s\n"
    );
}
