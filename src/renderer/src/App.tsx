import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type {
  ProviderId,
  ProviderUsageSnapshot,
  TrayciSettings,
  TrayciSettingsPatch,
  UsageState,
  UsageWindow,
} from "../../shared/types";
import { DEFAULT_SETTINGS } from "../../shared/types";
import {
  formatResetCountdown,
  tightestWindow,
} from "../../shared/presentation";

const EMPTY_STATE: UsageState = {
  providers: {},
  isRefreshing: false,
  lastRefreshStartedAt: null,
  lastRefreshCompletedAt: null,
};

function maximum(snapshot: ProviderUsageSnapshot): number {
  return Math.max(0, ...snapshot.windows.map((window) => window.usedPercent));
}

function displayPercent(
  window: UsageWindow,
  mode: TrayciSettings["percentageDisplay"],
): number {
  const used = Math.min(100, Math.max(0, window.usedPercent));
  return Math.round(mode === "used" ? used : 100 - used);
}

function denseLabel(window: UsageWindow): string {
  const cadence = window.durationMinutes === 10_080 ? "wk" : "5h";
  if (window.id.startsWith("gemini-")) return `Gem ${cadence}`;
  if (window.id.startsWith("claude-gpt-")) return `C/G ${cadence}`;
  return window.id === "weekly" ? "wk" : window.label;
}

function ProviderIcon({
  provider,
}: {
  provider: ProviderId;
}): React.JSX.Element {
  if (provider === "claude") {
    return (
      <span className="provider-icon claude-icon" aria-hidden="true">
        <svg viewBox="0 0 24 24">
          <path d="M12 3v18M3 12h18M5.6 5.6l12.8 12.8M18.4 5.6 5.6 18.4M8.4 3.8l7.2 16.4M20.2 8.4 3.8 15.6M15.6 3.8 8.4 20.2M20.2 15.6 3.8 8.4" />
        </svg>
      </span>
    );
  }
  if (provider === "antigravity") {
    return (
      <span className="provider-icon antigravity-icon" aria-hidden="true">
        <svg viewBox="0 0 24 24">
          <path d="M12 2.8c.8 5.1 4.1 8.4 9.2 9.2-5.1.8-8.4 4.1-9.2 9.2-.8-5.1-4.1-8.4-9.2-9.2 5.1-.8 8.4-4.1 9.2-9.2Z" />
          <path d="M18.2 3.2c.2 1.5 1.1 2.4 2.6 2.6-1.5.2-2.4 1.1-2.6 2.6-.2-1.5-1.1-2.4-2.6-2.6 1.5-.2 2.4-1.1 2.6-2.6Z" />
        </svg>
      </span>
    );
  }
  return (
    <span className="provider-icon codex-icon" aria-hidden="true">
      <svg viewBox="0 0 24 24">
        <path d="M12 3.1a4.4 4.4 0 0 1 4 2.5 4.4 4.4 0 0 1 3.8 6.6 4.4 4.4 0 0 1-3.7 6.3 4.4 4.4 0 0 1-7.9-.1 4.4 4.4 0 0 1-3.9-6.3 4.4 4.4 0 0 1 3.8-6.5A4.4 4.4 0 0 1 12 3.1Z" />
        <path d="m8.1 5.6 7.8 4.5v8.4M4.3 12.1l7.7-4.5 7.8 4.6M8.2 18.4V9.5l7.7-3.9M16.1 18.5 8.2 14l-3.9 2.2M12 20.9v-9l-7.7-4.4" />
      </svg>
    </span>
  );
}

function Metric({
  window,
  settings,
  now,
  dense = false,
}: {
  window: UsageWindow;
  settings: TrayciSettings;
  now: number;
  dense?: boolean;
}): React.JSX.Element {
  const used = Math.min(100, Math.max(0, window.usedPercent));
  const value = displayPercent(window, settings.percentageDisplay);
  return (
    <div className={dense ? "metric metric-dense" : "metric"}>
      <div className="metric-copy">
        <strong>{dense ? denseLabel(window) : window.label}</strong>
      </div>
      <div className="metric-line">
        <div
          className="progress"
          role="progressbar"
          aria-label={`${window.label} usage`}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(used)}
        >
          <span style={{ width: `${used}%` }} />
        </div>
        <strong>{value}%</strong>
      </div>
      {!dense ? (
        <div className="metric-meta">
          <span>
            {value}% {settings.percentageDisplay}
          </span>
          <span>
            {window.resetsAt
              ? `Resets in ${formatResetCountdown(window.resetsAt, now)}`
              : "—"}
          </span>
        </div>
      ) : null}
    </div>
  );
}

function ProviderRow({
  snapshot,
  settings,
  now,
  selected,
  onSelect,
}: {
  snapshot: ProviderUsageSnapshot;
  settings: TrayciSettings;
  now: number;
  selected: boolean;
  onSelect(): void;
}): React.JSX.Element {
  const tightest = tightestWindow(snapshot);
  const ageMinutes = Math.max(
    0,
    Math.floor((now - snapshot.updatedAt) / 60_000),
  );
  const updated = ageMinutes ? `Updated ${ageMinutes}m ago` : "Updated now";
  return (
    <button
      className={`provider-row provider-${snapshot.status}`}
      type="button"
      aria-pressed={selected}
      aria-busy={snapshot.status === "fetching"}
      onClick={onSelect}
    >
      <div className="provider-summary">
        <ProviderIcon provider={snapshot.provider} />
        <strong>{snapshot.displayName}</strong>
        {settings.displayMode === "compact" && tightest ? (
          <span className="compact-value">
            {tightest.resetsAt
              ? formatResetCountdown(tightest.resetsAt, now)
              : tightest.label}
            <strong>
              {displayPercent(tightest, settings.percentageDisplay)}%
            </strong>
          </span>
        ) : snapshot.status === "stale" ? (
          <span className="status">{updated}</span>
        ) : tightest?.resetsAt ? (
          <span className="reset-summary">
            Resets in {formatResetCountdown(tightest.resetsAt, now)}
          </span>
        ) : snapshot.windows.length ? (
          <span className="status">{updated}</span>
        ) : null}
        <span className="chevron" aria-hidden="true">
          ›
        </span>
      </div>
      {settings.displayMode === "detailed" && snapshot.windows.length ? (
        <div
          className={`inline-metrics${snapshot.windows.length > 2 ? " inline-metrics-grid" : ""}`}
        >
          {snapshot.windows.map((window) => (
            <Metric
              key={window.id}
              window={window}
              settings={settings}
              now={now}
              dense
            />
          ))}
        </div>
      ) : null}
      {!snapshot.windows.length ? (
        <span className="provider-message">
          {snapshot.status === "fetching"
            ? "Loading usage…"
            : (snapshot.error ?? "Usage unavailable")}
        </span>
      ) : null}
    </button>
  );
}

function ProviderDetail({
  snapshot,
  settings,
  now,
}: {
  snapshot: ProviderUsageSnapshot;
  settings: TrayciSettings;
  now: number;
}): React.JSX.Element {
  const ageMinutes = Math.max(
    0,
    Math.floor((now - snapshot.updatedAt) / 60_000),
  );
  return (
    <aside
      className="provider-detail"
      aria-label={`${snapshot.displayName} details`}
    >
      <header>
        <div className="detail-title">
          <ProviderIcon provider={snapshot.provider} />
          <strong>{snapshot.displayName}</strong>
          {snapshot.plan ? <span className="plan">{snapshot.plan}</span> : null}
        </div>
        <span>
          {ageMinutes ? `Updated ${ageMinutes}m ago` : "Updated just now"}
        </span>
      </header>
      <div className="detail-metrics">
        {snapshot.windows.map((window) => (
          <Metric
            key={window.id}
            window={window}
            settings={settings}
            now={now}
          />
        ))}
        {snapshot.error ? (
          <div className="inline-error">{snapshot.error}</div>
        ) : null}
      </div>
    </aside>
  );
}

function Switch({
  checked,
  onChange,
  label,
}: {
  checked: boolean;
  onChange(value: boolean): void;
  label: string;
}): React.JSX.Element {
  return (
    <label className="setting-row">
      <span>{label}</span>
      <input
        type="checkbox"
        role="switch"
        checked={checked}
        onChange={(event) => onChange(event.target.checked)}
      />
    </label>
  );
}

function Settings({
  settings,
  update,
}: {
  settings: TrayciSettings;
  update(patch: TrayciSettingsPatch): void;
}): React.JSX.Element {
  return (
    <div className="settings-view">
      <section className="settings-group">
        <h2>General</h2>
        <Switch
          checked={settings.startOnLogin}
          onChange={(startOnLogin) => update({ startOnLogin })}
          label="Start Trayci on login"
        />
        <Switch
          checked={settings.refreshOnResume}
          onChange={(refreshOnResume) => update({ refreshOnResume })}
          label="Refresh after resume"
        />
        <label className="setting-row">
          <span>Refresh interval</span>
          <select
            value={settings.pollIntervalMinutes}
            onChange={(event) =>
              update({ pollIntervalMinutes: Number(event.target.value) })
            }
          >
            {[5, 15, 30, 60].map((minutes) => (
              <option key={minutes} value={minutes}>
                {minutes} minutes
              </option>
            ))}
          </select>
        </label>
      </section>
      <section className="settings-group">
        <h2>Providers</h2>
        <Switch
          checked={settings.providers.claude.enabled}
          onChange={(enabled) => update({ providers: { claude: { enabled } } })}
          label="Claude Code"
        />
        <Switch
          checked={settings.providers.codex.enabled}
          onChange={(enabled) => update({ providers: { codex: { enabled } } })}
          label="Codex"
        />
        <Switch
          checked={settings.providers.antigravity.enabled}
          onChange={(enabled) =>
            update({ providers: { antigravity: { enabled } } })
          }
          label="Antigravity"
        />
      </section>
      <section className="settings-group">
        <h2>Display</h2>
        <label className="setting-row">
          <span>Percentage</span>
          <select
            value={settings.percentageDisplay}
            onChange={(event) =>
              update({
                percentageDisplay: event.target.value as "used" | "remaining",
              })
            }
          >
            <option value="used">Used</option>
            <option value="remaining">Remaining</option>
          </select>
        </label>
      </section>
      <button
        className="quit-button"
        type="button"
        onClick={() => void window.trayci.app.quit()}
      >
        Quit Trayci
      </button>
    </div>
  );
}

export default function App(): React.JSX.Element {
  const [state, setState] = useState(EMPTY_STATE);
  const [settings, setSettings] = useState(DEFAULT_SETTINGS);
  const [view, setView] = useState<"usage" | "settings">("usage");
  const [selectedProvider, setSelectedProvider] = useState<ProviderId | null>(
    null,
  );
  const [now, setNow] = useState(Date.now());
  const [error, setError] = useState<string | null>(null);
  const root = useRef<HTMLElement>(null);

  useEffect(() => {
    void Promise.all([
      window.trayci.usage.getState(),
      window.trayci.settings.get(),
    ])
      .then(([nextState, nextSettings]) => {
        setState(nextState);
        setSettings(nextSettings);
      })
      .catch(() => setError("Trayci could not load its state."));
    const unsubscribe = window.trayci.usage.subscribe(setState);
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    const escape = (event: KeyboardEvent): void => {
      if (event.key === "Escape") void window.trayci.app.hidePopover();
    };
    window.addEventListener("keydown", escape);
    return () => {
      unsubscribe();
      window.clearInterval(timer);
      window.removeEventListener("keydown", escape);
    };
  }, []);

  const providers = useMemo(
    () =>
      Object.values(state.providers)
        .filter((snapshot): snapshot is ProviderUsageSnapshot =>
          Boolean(snapshot),
        )
        .sort((a, b) => maximum(b) - maximum(a)),
    [state.providers],
  );
  const selected =
    view === "usage"
      ? (providers.find((snapshot) => snapshot.provider === selectedProvider) ??
        null)
      : null;
  const targetWidth = selected ? 680 : 360;

  useLayoutEffect(() => {
    const element = root.current;
    if (!element) return;
    let frame = 0;
    const resize = (): void => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(
        () =>
          void window.trayci.app.resizePopover(
            targetWidth,
            element.scrollHeight,
          ),
      );
    };
    const observer = new ResizeObserver(resize);
    observer.observe(element);
    resize();
    return () => {
      observer.disconnect();
      window.cancelAnimationFrame(frame);
    };
  }, [targetWidth, view]);

  const update = (patch: TrayciSettingsPatch): void => {
    setError(null);
    const previous = settings;
    setSettings({
      ...settings,
      ...patch,
      schemaVersion: 1,
      providers: {
        claude: { ...settings.providers.claude, ...patch.providers?.claude },
        codex: { ...settings.providers.codex, ...patch.providers?.codex },
        antigravity: {
          ...settings.providers.antigravity,
          ...patch.providers?.antigravity,
        },
      },
    });
    void window.trayci.settings
      .update(patch)
      .then(setSettings)
      .catch(() => {
        setSettings(previous);
        setError("Setting could not be saved.");
      });
  };

  const setMode = (displayMode: TrayciSettings["displayMode"]): void => {
    if (displayMode === "compact") setSelectedProvider(null);
    update({ displayMode });
  };

  return (
    <main ref={root} className={selected ? "has-detail" : ""}>
      <section className="master">
        <header className="topbar">
          {view === "settings" ? (
            <button
              className="back-button"
              type="button"
              aria-label="Back to usage"
              onClick={() => setView("usage")}
            >
              ‹
            </button>
          ) : null}
          <h1>{view === "usage" ? "Usage" : "Settings"}</h1>
          <div className="header-actions">
            {view === "usage" ? <span>all agents</span> : null}
            {view === "usage" ? (
              <button
                className="icon-button"
                type="button"
                aria-label="Refresh usage"
                disabled={state.isRefreshing}
                onClick={() =>
                  void window.trayci.usage
                    .refreshAll()
                    .catch(() => setError("Refresh failed."))
                }
              >
                <span
                  className={state.isRefreshing ? "spin" : ""}
                  aria-hidden="true"
                >
                  ↻
                </span>
              </button>
            ) : null}
            <button
              className="icon-button settings-button"
              type="button"
              aria-label={view === "usage" ? "Settings" : "Back to usage"}
              onClick={() => setView(view === "usage" ? "settings" : "usage")}
            >
              {view === "usage" ? "···" : "×"}
            </button>
          </div>
        </header>

        {error ? (
          <div className="banner" role="alert">
            {error}
          </div>
        ) : null}

        {view === "usage" ? (
          <>
            <div className="density" aria-label="Information density">
              {(["detailed", "compact"] as const).map((mode) => (
                <button
                  key={mode}
                  type="button"
                  aria-pressed={settings.displayMode === mode}
                  onClick={() => setMode(mode)}
                >
                  {mode === "detailed" ? "Detailed" : "Compact"}
                </button>
              ))}
            </div>
            <div className="provider-list" aria-live="polite">
              {providers.length ? (
                providers.map((snapshot) => (
                  <ProviderRow
                    key={snapshot.provider}
                    snapshot={snapshot}
                    settings={settings}
                    now={now}
                    selected={selectedProvider === snapshot.provider}
                    onSelect={() =>
                      setSelectedProvider(
                        selectedProvider === snapshot.provider
                          ? null
                          : snapshot.provider,
                      )
                    }
                  />
                ))
              ) : (
                <div className="empty">No providers enabled.</div>
              )}
            </div>
          </>
        ) : (
          <Settings settings={settings} update={update} />
        )}
      </section>
      {view === "usage" && selected ? (
        <ProviderDetail snapshot={selected} settings={settings} now={now} />
      ) : null}
    </main>
  );
}
