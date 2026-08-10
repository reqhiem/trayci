use crate::model::{
    NotificationSettingsPatch, ProviderSettings, ProviderSettingsPatch, TrayciSettings,
    TrayciSettingsPatch,
};
use std::{io, path::PathBuf};
use tokio::fs;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("invalid settings: {0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

pub fn config_directory() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(std::env::temp_dir)
        .join("trayci")
}

pub struct SettingsRepository {
    path: PathBuf,
    settings: TrayciSettings,
}

impl Default for SettingsRepository {
    fn default() -> Self {
        Self::new(config_directory().join("config.json"))
    }
}

impl SettingsRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            settings: TrayciSettings::default(),
        }
    }

    pub async fn load(&mut self) -> TrayciSettings {
        self.settings = match fs::read(&self.path).await {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .ok()
                .filter(|settings: &TrayciSettings| validate(settings).is_ok())
                .unwrap_or_default(),
            Err(_) => TrayciSettings::default(),
        };
        self.get()
    }

    pub fn get(&self) -> TrayciSettings {
        self.settings.clone()
    }

    pub async fn update(
        &mut self,
        patch: TrayciSettingsPatch,
    ) -> Result<TrayciSettings, SettingsError> {
        let mut next = self.settings.clone();
        merge(&mut next, patch)?;
        atomic_json(&self.path, &next).await?;
        self.settings = next;
        Ok(self.get())
    }
}

pub fn merge(
    settings: &mut TrayciSettings,
    patch: TrayciSettingsPatch,
) -> Result<(), SettingsError> {
    if let Some(value) = patch.start_on_login {
        settings.start_on_login = value;
    }
    if let Some(value) = patch.refresh_on_resume {
        settings.refresh_on_resume = value;
    }
    if let Some(value) = patch.poll_interval_minutes {
        settings.poll_interval_minutes = value;
    }
    if let Some(value) = patch.display_mode {
        settings.display_mode = value;
    }
    if let Some(value) = patch.percentage_display {
        settings.percentage_display = value;
    }
    if let Some(value) = patch.notifications {
        merge_notifications(&mut settings.notifications, value);
    }
    if let Some(value) = patch.providers {
        if let Some(patch) = value.claude {
            merge_provider(&mut settings.providers.claude, patch)?;
        }
        if let Some(patch) = value.codex {
            merge_provider(&mut settings.providers.codex, patch)?;
        }
        if let Some(patch) = value.antigravity {
            merge_provider(&mut settings.providers.antigravity, patch)?;
        }
    }
    if let Some(value) = patch.window_position {
        settings.window_position = value;
    }
    settings.schema_version = 1;
    validate(settings)
}

fn merge_notifications(
    target: &mut crate::model::NotificationSettings,
    patch: NotificationSettingsPatch,
) {
    if let Some(value) = patch.quota50 {
        target.quota50 = value;
    }
    if let Some(value) = patch.quota85 {
        target.quota85 = value;
    }
    if let Some(value) = patch.quota90 {
        target.quota90 = value;
    }
    if let Some(value) = patch.reset {
        target.reset = value;
    }
}

fn merge_provider(
    target: &mut ProviderSettings,
    patch: ProviderSettingsPatch,
) -> Result<(), SettingsError> {
    if let Some(value) = patch.enabled {
        target.enabled = value;
    }
    if let Some(value) = patch.executable_path {
        if value.as_ref().is_some_and(|path| !path.is_absolute()) {
            return Err(SettingsError::Invalid(
                "provider executablePath must be absolute or null".into(),
            ));
        }
        target.executable_path = value;
    }
    Ok(())
}

fn validate(settings: &TrayciSettings) -> Result<(), SettingsError> {
    if settings.schema_version != 1 {
        return Err(SettingsError::Invalid("unsupported schemaVersion".into()));
    }
    if !(5..=1440).contains(&settings.poll_interval_minutes) {
        return Err(SettingsError::Invalid(
            "pollIntervalMinutes must be between 5 and 1440".into(),
        ));
    }
    for provider in [
        &settings.providers.claude,
        &settings.providers.codex,
        &settings.providers.antigravity,
    ] {
        if provider
            .executable_path
            .as_ref()
            .is_some_and(|path| !path.is_absolute())
        {
            return Err(SettingsError::Invalid(
                "provider executablePath must be absolute or null".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) async fn atomic_json<T: serde::Serialize>(path: &PathBuf, value: &T) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    fs::create_dir_all(parent).await?;
    #[cfg(unix)]
    fs::set_permissions(parent, std::os::unix::fs::PermissionsExt::from_mode(0o700)).await?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_extension(format!("{}.{nonce}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;

    #[cfg(unix)]
    {
        let mut options = fs::OpenOptions::new();
        options.create(true).truncate(true).write(true).mode(0o600);
        let mut file = options.open(&temporary).await?;
        use tokio::io::AsyncWriteExt;
        file.write_all(&bytes).await?;
        file.write_all(b"\n").await?;
        file.sync_all().await?;
    }
    #[cfg(not(unix))]
    fs::write(&temporary, [bytes, b"\n".to_vec()].concat()).await?;

    fs::rename(temporary, path).await
}
