use crate::{cache::CacheRepository, model::*};
use async_trait::async_trait;
use futures::future::join_all;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{watch, Mutex},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

pub const STALE_AFTER_MS: u64 = 30 * 60_000;
pub const MAX_STALE_RETENTION_MS: u64 = 24 * 60 * 60_000;
pub const BACKOFF_MS: [u64; 5] = [30_000, 60_000, 120_000, 300_000, 900_000];

#[async_trait]
pub trait UsageProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    async fn detect(&self) -> ProviderDetection;
    async fn fetch_usage(
        &self,
        context: &UsageFetchContext,
    ) -> Result<ProviderUsageSnapshot, ProviderError>;
}

type Listener = Arc<dyn Fn(UsageState) + Send + Sync>;

#[derive(Default)]
struct RefreshCoord {
    active: bool,
    queued_manual: bool,
    generation: u64,
}

struct Inner {
    state: UsageState,
    failures: HashMap<ProviderId, usize>,
    next_retry: HashMap<ProviderId, u64>,
    stopped: bool,
    cancellation: Option<CancellationToken>,
}

pub struct UsageService {
    providers: Vec<Box<dyn UsageProvider>>,
    settings: Arc<dyn Fn() -> TrayciSettings + Send + Sync>,
    cache: CacheRepository,
    inner: Mutex<Inner>,
    refresh: Mutex<RefreshCoord>,
    refreshed: watch::Sender<u64>,
    listeners: StdMutex<HashMap<u64, Listener>>,
    next_listener: std::sync::atomic::AtomicU64,
    polling: StdMutex<Option<JoinHandle<()>>>,
}

impl UsageService {
    pub fn new(
        providers: Vec<Box<dyn UsageProvider>>,
        settings: impl Fn() -> TrayciSettings + Send + Sync + 'static,
    ) -> Self {
        Self::with_cache(providers, settings, CacheRepository::default())
    }

    pub fn with_cache(
        providers: Vec<Box<dyn UsageProvider>>,
        settings: impl Fn() -> TrayciSettings + Send + Sync + 'static,
        cache: CacheRepository,
    ) -> Self {
        Self {
            providers,
            settings: Arc::new(settings),
            cache,
            inner: Mutex::new(Inner {
                state: UsageState::default(),
                failures: HashMap::new(),
                next_retry: HashMap::new(),
                stopped: true,
                cancellation: None,
            }),
            refresh: Mutex::new(RefreshCoord::default()),
            refreshed: watch::channel(0).0,
            listeners: StdMutex::new(HashMap::new()),
            next_listener: std::sync::atomic::AtomicU64::new(1),
            polling: StdMutex::new(None),
        }
    }

    pub async fn start(self: &Arc<Self>) {
        let cached = self.cache.load().await;
        {
            let enabled = self.enabled_provider_ids();
            let mut inner = self.inner.lock().await;
            inner.stopped = false;
            inner.state.providers = cached
                .into_iter()
                .filter(|(id, _)| enabled.iter().any(|enabled| enabled == id))
                .collect();
        }
        self.emit().await;
        self.refresh_all(UsageFetchReason::Startup).await;
        let service = Arc::clone(self);
        *self.polling.lock().expect("polling lock") = Some(tokio::spawn(async move {
            service.poll_loop().await;
        }));
    }

    pub async fn stop(&self) {
        if let Some(task) = self.polling.lock().expect("polling lock").take() {
            task.abort();
        }
        let state = {
            let mut inner = self.inner.lock().await;
            inner.stopped = true;
            if let Some(token) = inner.cancellation.take() {
                token.cancel();
            }
            inner.state.clone()
        };
        crate::providers::abort_all_children();
        let _ = self.cache.save(&state.providers).await;
    }

    pub async fn get_state(&self) -> UsageState {
        self.inner.lock().await.state.clone()
    }

    pub fn subscribe(
        self: &Arc<Self>,
        listener: impl Fn(UsageState) + Send + Sync + 'static,
    ) -> Subscription {
        let id = self
            .next_listener
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.listeners
            .lock()
            .expect("listeners lock")
            .insert(id, Arc::new(listener));
        Subscription {
            service: Arc::downgrade(self),
            id,
        }
    }

    pub async fn refresh_all(&self, reason: UsageFetchReason) -> UsageState {
        let waiting = {
            let mut refresh = self.refresh.lock().await;
            if refresh.active {
                if reason == UsageFetchReason::Manual {
                    refresh.queued_manual = true;
                }
                Some((refresh.generation, self.refreshed.subscribe()))
            } else {
                refresh.active = true;
                None
            }
        };
        if let Some((generation, mut refreshed)) = waiting {
            loop {
                if *refreshed.borrow_and_update() != generation {
                    return self.get_state().await;
                }
                let _ = refreshed.changed().await;
            }
        }

        let mut cycle_reason = reason;
        loop {
            self.perform_refresh(cycle_reason).await;
            let mut refresh = self.refresh.lock().await;
            if refresh.queued_manual && !self.inner.lock().await.stopped {
                refresh.queued_manual = false;
                cycle_reason = UsageFetchReason::Manual;
            } else {
                refresh.active = false;
                refresh.generation += 1;
                self.refreshed.send_replace(refresh.generation);
                return self.get_state().await;
            }
        }
    }

    pub async fn refresh_provider(
        &self,
        provider_id: &str,
        reason: UsageFetchReason,
    ) -> Result<ProviderUsageSnapshot, String> {
        let provider = self
            .providers
            .iter()
            .find(|provider| provider.id() == provider_id)
            .ok_or_else(|| format!("Unknown provider: {provider_id}"))?;
        Ok(self
            .fetch_provider(
                provider.as_ref(),
                reason,
                now_ms(),
                CancellationToken::new(),
            )
            .await)
    }

    pub async fn settings_changed(self: &Arc<Self>) {
        let enabled = self.enabled_provider_ids();
        {
            let mut inner = self.inner.lock().await;
            inner.state.providers.retain(|id, _| enabled.contains(id));
        }
        self.emit().await;
        self.refresh_all(UsageFetchReason::Manual).await;
    }

    async fn perform_refresh(&self, reason: UsageFetchReason) {
        let started_at = now_ms();
        let cancellation = CancellationToken::new();
        {
            let mut inner = self.inner.lock().await;
            inner.cancellation = Some(cancellation.clone());
            inner.state.is_refreshing = true;
            inner.state.last_refresh_started_at = Some(started_at);
        }
        self.emit().await;

        let enabled = self.enabled_provider_ids();
        let due = {
            let inner = self.inner.lock().await;
            self.providers
                .iter()
                .filter(|provider| enabled.iter().any(|id| id == provider.id()))
                .filter(|provider| {
                    reason == UsageFetchReason::Manual
                        || inner.next_retry.get(provider.id()).copied().unwrap_or(0) <= started_at
                })
                .collect::<Vec<_>>()
        };
        join_all(due.into_iter().map(|provider| {
            self.fetch_provider(provider.as_ref(), reason, started_at, cancellation.clone())
        }))
        .await;

        let providers = {
            let mut inner = self.inner.lock().await;
            inner.state.is_refreshing = false;
            inner.state.last_refresh_completed_at = Some(now_ms());
            inner.cancellation = None;
            inner.state.providers.clone()
        };
        let _ = self.cache.save(&providers).await;
        self.emit().await;
    }

    async fn fetch_provider(
        &self,
        provider: &dyn UsageProvider,
        reason: UsageFetchReason,
        now: u64,
        cancellation: CancellationToken,
    ) -> ProviderUsageSnapshot {
        let previous = {
            let mut inner = self.inner.lock().await;
            let previous = inner.state.providers.get(provider.id()).cloned();
            inner.state.providers.insert(
                provider.id().into(),
                previous.clone().map_or_else(
                    || empty_snapshot(provider, UsageStatus::Fetching, None, now),
                    |mut snapshot| {
                        snapshot.status = UsageStatus::Fetching;
                        snapshot.error = None;
                        snapshot
                    },
                ),
            );
            previous
        };
        self.emit().await;

        let result = provider
            .fetch_usage(&UsageFetchContext {
                cancellation,
                reason,
                now,
            })
            .await;
        let snapshot = match result {
            Ok(snapshot) => {
                let mut inner = self.inner.lock().await;
                inner.failures.remove(provider.id());
                inner.next_retry.remove(provider.id());
                snapshot.normalize()
            }
            Err(error) if error.kind == ProviderErrorKind::Aborted => {
                previous.unwrap_or_else(|| {
                    empty_snapshot(
                        provider,
                        UsageStatus::Error,
                        Some("Usage unavailable".into()),
                        now,
                    )
                })
            }
            Err(error) => {
                let definitive = matches!(
                    error.kind,
                    ProviderErrorKind::NotInstalled | ProviderErrorKind::NotAuthenticated
                );
                let mut inner = self.inner.lock().await;
                if definitive {
                    inner.failures.remove(provider.id());
                    inner.next_retry.remove(provider.id());
                } else {
                    let count = inner.failures.entry(provider.id().into()).or_default();
                    *count += 1;
                    let retry_at = error.retry_at.unwrap_or_else(|| {
                        now + BACKOFF_MS[(*count - 1).min(BACKOFF_MS.len() - 1)]
                    });
                    inner.next_retry.insert(provider.id().into(), retry_at);
                }
                match previous.filter(|snapshot| {
                    !definitive
                        && !snapshot.windows.is_empty()
                        && now.saturating_sub(snapshot.updated_at) <= MAX_STALE_RETENTION_MS
                }) {
                    Some(mut snapshot) => {
                        snapshot.status = UsageStatus::Stale;
                        snapshot.error = Some(public_error(error.kind).into());
                        snapshot
                    }
                    None => empty_snapshot(
                        provider,
                        if definitive {
                            UsageStatus::Unavailable
                        } else {
                            UsageStatus::Error
                        },
                        Some(public_error(error.kind).into()),
                        now,
                    ),
                }
            }
        };
        self.inner
            .lock()
            .await
            .state
            .providers
            .insert(provider.id().into(), snapshot.clone());
        self.emit().await;
        snapshot
    }

    async fn poll_loop(self: Arc<Self>) {
        let mut last_tick = now_ms();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if self.inner.lock().await.stopped {
                return;
            }
            let now = now_ms();
            let interval = (self.settings)().poll_interval_minutes * 60_000;
            let resume = (self.settings)().refresh_on_resume
                && now.saturating_sub(last_tick) > interval + 30_000;
            last_tick = now;
            let (poll_due, retry_due) = {
                let inner = self.inner.lock().await;
                (
                    inner
                        .state
                        .last_refresh_completed_at
                        .map_or(true, |completed| now >= completed + interval),
                    inner.next_retry.values().any(|retry| *retry <= now),
                )
            };
            if resume || retry_due || poll_due {
                self.refresh_all(if resume {
                    UsageFetchReason::Resume
                } else if retry_due {
                    UsageFetchReason::Retry
                } else {
                    UsageFetchReason::Poll
                })
                .await;
            }
        }
    }

    fn enabled_provider_ids(&self) -> Vec<String> {
        let settings = (self.settings)();
        [
            ("claude", settings.providers.claude.enabled),
            ("codex", settings.providers.codex.enabled),
            ("antigravity", settings.providers.antigravity.enabled),
        ]
        .into_iter()
        .filter(|(_, enabled)| *enabled)
        .map(|(id, _)| id.into())
        .collect()
    }

    async fn emit(&self) {
        let state = self.get_state().await;
        let listeners = self
            .listeners
            .lock()
            .expect("listeners lock")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for listener in listeners {
            listener(state.clone());
        }
    }
}

pub struct Subscription {
    service: std::sync::Weak<UsageService>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(service) = self.service.upgrade() {
            service
                .listeners
                .lock()
                .expect("listeners lock")
                .remove(&self.id);
        }
    }
}

fn empty_snapshot(
    provider: &dyn UsageProvider,
    status: UsageStatus,
    error: Option<String>,
    now: u64,
) -> ProviderUsageSnapshot {
    ProviderUsageSnapshot {
        provider: provider.id().into(),
        display_name: provider.display_name().into(),
        status,
        plan: None,
        windows: Vec::new(),
        updated_at: now,
        source: None,
        error,
    }
}

fn public_error(kind: ProviderErrorKind) -> &'static str {
    match kind {
        ProviderErrorKind::NotInstalled => "CLI not detected",
        ProviderErrorKind::NotAuthenticated => "Not signed in",
        ProviderErrorKind::RateLimited => "Temporarily rate limited",
        ProviderErrorKind::Timeout => "Provider timed out",
        ProviderErrorKind::Parse => "Provider response changed",
        _ => "Usage unavailable",
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
