export type BuiltInProviderId = "claude" | "codex" | "antigravity";
export type ProviderId = BuiltInProviderId | `plugin:${string}`;
export type ProviderDetectionStatus =
  "available" | "not-installed" | "not-authenticated" | "unknown";
export type UsageStatus =
  "idle" | "fetching" | "ok" | "stale" | "error" | "unavailable";
export type UsageSource = "oauth" | "rpc" | "api" | "cli" | "cache";
export type UsageFetchReason =
  "startup" | "manual" | "poll" | "resume" | "popover-open" | "retry";

export type UsageWindow = {
  id: string;
  label: string;
  usedPercent: number;
  durationMinutes: number | null;
  resetsAt: number | null;
  resetDescription: string | null;
};

export type ProviderUsageSnapshot = {
  provider: ProviderId;
  displayName: string;
  status: UsageStatus;
  plan: string | null;
  windows: UsageWindow[];
  updatedAt: number;
  source: UsageSource | null;
  error: string | null;
};

export type UsageState = {
  providers: Partial<Record<ProviderId, ProviderUsageSnapshot>>;
  isRefreshing: boolean;
  lastRefreshStartedAt: number | null;
  lastRefreshCompletedAt: number | null;
};

export type ProviderDetection = {
  provider: ProviderId;
  status: ProviderDetectionStatus;
  executablePath: string | null;
};

export type UsageFetchContext = {
  signal: AbortSignal;
  reason: UsageFetchReason;
  now: number;
};

export interface UsageProvider {
  readonly id: ProviderId;
  readonly displayName: string;
  detect(signal?: AbortSignal): Promise<ProviderDetection>;
  fetchUsage(context: UsageFetchContext): Promise<ProviderUsageSnapshot>;
}

export type TrayciSettings = {
  schemaVersion: 1;
  startOnLogin: boolean;
  refreshOnResume: boolean;
  pollIntervalMinutes: number;
  displayMode: "detailed" | "compact";
  percentageDisplay: "used" | "remaining";
  providers: Record<
    BuiltInProviderId,
    { enabled: boolean; executablePath: string | null }
  >;
};

export type TrayciSettingsPatch = Partial<
  Omit<TrayciSettings, "schemaVersion" | "providers">
> & {
  providers?: Partial<{
    [Provider in BuiltInProviderId]: Partial<
      TrayciSettings["providers"][Provider]
    >;
  }>;
};

export const DEFAULT_SETTINGS: TrayciSettings = {
  schemaVersion: 1,
  startOnLogin: false,
  refreshOnResume: true,
  pollIntervalMinutes: 15,
  displayMode: "detailed",
  percentageDisplay: "used",
  providers: {
    claude: { enabled: true, executablePath: null },
    codex: { enabled: true, executablePath: null },
    antigravity: { enabled: true, executablePath: null },
  },
};

export type TrayciApi = {
  usage: {
    getState(): Promise<UsageState>;
    refreshAll(): Promise<UsageState>;
    subscribe(callback: (state: UsageState) => void): () => void;
  };
  settings: {
    get(): Promise<TrayciSettings>;
    update(patch: TrayciSettingsPatch): Promise<TrayciSettings>;
  };
  app: {
    hidePopover(): Promise<void>;
    resizePopover(width: number, height: number): Promise<void>;
    quit(): Promise<void>;
  };
};
