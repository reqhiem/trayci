import type {
  ProviderId,
  ProviderUsageSnapshot,
  TrayciSettings,
  UsageFetchReason,
  UsageProvider,
  UsageState,
} from "../shared/types";
import { log } from "./logger";
import { CacheRepository } from "./persistence";
import { ProviderError } from "./providers/common";

const STALE_AFTER_MS = 30 * 60_000;
const MAX_STALE_RETENTION_MS = 24 * 60 * 60_000;
const BACKOFF_MS = [30_000, 60_000, 120_000, 300_000, 900_000];

type Listener = (state: UsageState) => void;

function emptySnapshot(
  provider: UsageProvider,
  status: ProviderUsageSnapshot["status"],
  error: string | null,
): ProviderUsageSnapshot {
  return {
    provider: provider.id,
    displayName: provider.displayName,
    status,
    plan: null,
    windows: [],
    updatedAt: Date.now(),
    source: null,
    error,
  };
}

function publicError(error: unknown): string {
  if (!(error instanceof ProviderError)) return "Usage unavailable";
  switch (error.kind) {
    case "not-installed":
      return "CLI not detected";
    case "not-authenticated":
      return "Not signed in";
    case "rate-limited":
      return "Temporarily rate limited";
    case "timeout":
      return "Provider timed out";
    case "parse":
      return "Provider response changed";
    default:
      return "Usage unavailable";
  }
}

export class UsageService {
  private state: UsageState = {
    providers: {},
    isRefreshing: false,
    lastRefreshStartedAt: null,
    lastRefreshCompletedAt: null,
  };
  private readonly listeners = new Set<Listener>();
  private readonly failures = new Map<ProviderId, number>();
  private readonly nextRetry = new Map<ProviderId, number>();
  private active: Promise<UsageState> | null = null;
  private queuedManual = false;
  private controller: AbortController | null = null;
  private timer: NodeJS.Timeout | null = null;
  private stopped = true;

  constructor(
    private readonly providers: UsageProvider[],
    private readonly getSettings: () => TrayciSettings,
    private readonly cache: Pick<
      CacheRepository,
      "load" | "save"
    > = new CacheRepository(),
  ) {}

  async start(): Promise<void> {
    this.stopped = false;
    const cached = await this.cache.load();
    const enabled = this.enabledProviders();
    this.state.providers = Object.fromEntries(
      enabled.flatMap((provider) =>
        cached[provider.id] ? [[provider.id, cached[provider.id]]] : [],
      ),
    );
    this.emit();
    await this.refreshAll("startup");
  }

  async stop(): Promise<void> {
    this.stopped = true;
    if (this.timer) clearTimeout(this.timer);
    this.controller?.abort();
    await this.active?.catch(() => undefined);
    await this.cache.save(this.state.providers);
  }

  getState(): UsageState {
    return structuredClone(this.state);
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  async refreshAll(reason: UsageFetchReason = "manual"): Promise<UsageState> {
    if (this.active) {
      if (reason === "manual") this.queuedManual = true;
      await this.active;
      if (reason === "manual" && this.queuedManual) {
        this.queuedManual = false;
        return this.refreshAll("manual");
      }
      return this.getState();
    }
    this.active = this.performRefresh(reason);
    try {
      return await this.active;
    } finally {
      this.active = null;
      if (this.queuedManual && !this.stopped) {
        this.queuedManual = false;
        void this.refreshAll("manual");
      }
    }
  }

  async refreshProvider(
    providerId: ProviderId,
    reason: UsageFetchReason = "manual",
  ): Promise<ProviderUsageSnapshot> {
    const provider = this.providers.find(
      (candidate) => candidate.id === providerId,
    );
    if (!provider) throw new Error(`Unknown provider: ${providerId}`);
    return this.fetchProvider(
      provider,
      reason,
      Date.now(),
      new AbortController().signal,
    );
  }

  settingsChanged(): void {
    const enabled = new Set(
      this.enabledProviders().map((provider) => provider.id),
    );
    this.state.providers = Object.fromEntries(
      Object.entries(this.state.providers).filter(([id]) =>
        enabled.has(id as ProviderId),
      ),
    );
    this.emit();
    this.schedule();
    void this.refreshAll("manual");
  }

  private enabledProviders(): UsageProvider[] {
    const settings = this.getSettings();
    return this.providers.filter((provider) => {
      if (provider.id !== "claude" && provider.id !== "codex") return true;
      return settings.providers[provider.id].enabled;
    });
  }

  private async performRefresh(reason: UsageFetchReason): Promise<UsageState> {
    const startedAt = Date.now();
    this.controller = new AbortController();
    this.state = {
      ...this.state,
      isRefreshing: true,
      lastRefreshStartedAt: startedAt,
    };
    this.emit();
    const providers = this.enabledProviders().filter(
      (provider) =>
        reason === "manual" ||
        (this.nextRetry.get(provider.id) ?? 0) <= startedAt,
    );
    await Promise.allSettled(
      providers.map((provider) =>
        this.fetchProvider(
          provider,
          reason,
          startedAt,
          this.controller!.signal,
        ),
      ),
    );
    this.state = {
      ...this.state,
      isRefreshing: false,
      lastRefreshCompletedAt: Date.now(),
    };
    this.controller = null;
    await this.cache
      .save(this.state.providers)
      .catch(() => log("warn", "cache_write_failed"));
    this.emit();
    this.schedule();
    return this.getState();
  }

  private async fetchProvider(
    provider: UsageProvider,
    reason: UsageFetchReason,
    now: number,
    signal: AbortSignal,
  ): Promise<ProviderUsageSnapshot> {
    const previous = this.state.providers[provider.id];
    this.state.providers[provider.id] = previous
      ? { ...previous, status: "fetching", error: null }
      : emptySnapshot(provider, "fetching", null);
    this.emit();
    const started = performance.now();
    try {
      const snapshot = await provider.fetchUsage({ signal, reason, now });
      this.failures.delete(provider.id);
      this.nextRetry.delete(provider.id);
      this.state.providers[provider.id] = snapshot;
      log("info", "provider_refresh", {
        provider: provider.id,
        status: "ok",
        durationMs: Math.round(performance.now() - started),
        source: snapshot.source,
      });
      this.emit();
      return snapshot;
    } catch (error) {
      if (error instanceof ProviderError && error.kind === "aborted")
        throw error;
      const message = publicError(error);
      const definitive =
        error instanceof ProviderError &&
        ["not-installed", "not-authenticated"].includes(error.kind);
      if (definitive) {
        this.failures.delete(provider.id);
        this.nextRetry.delete(provider.id);
      } else {
        const count = (this.failures.get(provider.id) ?? 0) + 1;
        this.failures.set(provider.id, count);
        const retryAt =
          error instanceof ProviderError && error.retryAt
            ? error.retryAt
            : now + BACKOFF_MS[Math.min(count - 1, BACKOFF_MS.length - 1)]!;
        this.nextRetry.set(provider.id, retryAt);
      }
      const age = previous ? now - previous.updatedAt : Infinity;
      const snapshot =
        previous &&
        previous.windows.length &&
        !definitive &&
        age <= MAX_STALE_RETENTION_MS
          ? { ...previous, status: "stale" as const, error: message }
          : emptySnapshot(
              provider,
              definitive ? "unavailable" : "error",
              message,
            );
      this.state.providers[provider.id] = snapshot;
      log("warn", "provider_refresh", {
        provider: provider.id,
        status: snapshot.status,
        kind: error instanceof ProviderError ? error.kind : "unknown",
        durationMs: Math.round(performance.now() - started),
      });
      this.emit();
      return snapshot;
    }
  }

  private schedule(): void {
    if (this.timer) clearTimeout(this.timer);
    if (this.stopped) return;
    const now = Date.now();
    const pollAt =
      (this.state.lastRefreshCompletedAt ?? now) +
      this.getSettings().pollIntervalMinutes * 60_000;
    const retryAt = Math.min(...this.nextRetry.values(), Infinity);
    const nextAt = Math.min(pollAt, retryAt);
    this.timer = setTimeout(
      () => {
        const reason = retryAt <= Date.now() ? "retry" : "poll";
        void this.refreshAll(reason);
      },
      Math.max(1_000, nextAt - now),
    );
    this.timer.unref();
  }

  private emit(): void {
    const state = this.getState();
    for (const listener of this.listeners) listener(state);
  }
}

export const staleAfterMs = STALE_AFTER_MS;
export const maxStaleRetentionMs = MAX_STALE_RETENTION_MS;
