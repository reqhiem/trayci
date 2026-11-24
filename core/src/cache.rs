use crate::{model::*, settings::atomic_json};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, io, path::PathBuf};
use tokio::fs;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheInput {
    schema_version: u8,
    providers: HashMap<String, Value>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CacheOutput<'a> {
    schema_version: u8,
    providers: &'a HashMap<ProviderId, ProviderUsageSnapshot>,
}

pub struct CacheRepository {
    path: PathBuf,
}

impl Default for CacheRepository {
    fn default() -> Self {
        Self::new(crate::settings::config_directory().join("cache.json"))
    }
}

impl CacheRepository {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Cached usage is only stale once it is older than [`crate::service::STALE_AFTER_MS`];
    /// treating a snapshot from two minutes ago as stale is what made every launch re-fetch.
    pub async fn load(&self, now: u64) -> HashMap<ProviderId, ProviderUsageSnapshot> {
        let Ok(bytes) = fs::read(&self.path).await else {
            return HashMap::new();
        };
        let Ok(cache) = serde_json::from_slice::<CacheInput>(&bytes) else {
            return HashMap::new();
        };
        if cache.schema_version != 1 {
            return HashMap::new();
        }
        cache
            .providers
            .into_iter()
            .filter_map(|(id, value)| {
                serde_json::from_value::<ProviderUsageSnapshot>(value)
                    .ok()
                    .map(|mut snapshot| {
                        snapshot.status = if now.saturating_sub(snapshot.updated_at)
                            <= crate::service::STALE_AFTER_MS
                        {
                            UsageStatus::Ok
                        } else {
                            UsageStatus::Stale
                        };
                        snapshot.source = Some(UsageSource::Cache);
                        snapshot.error = None;
                        (id, snapshot.normalize())
                    })
            })
            .collect()
    }

    pub async fn save(
        &self,
        providers: &HashMap<ProviderId, ProviderUsageSnapshot>,
    ) -> io::Result<()> {
        let normalized = providers
            .iter()
            .map(|(id, snapshot)| (id.clone(), snapshot.clone().normalize()))
            .collect();
        atomic_json(
            &self.path,
            &CacheOutput {
                schema_version: 1,
                providers: &normalized,
            },
        )
        .await
    }
}
