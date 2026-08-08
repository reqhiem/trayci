import { useEffect, useMemo, useState } from "react";
import type {
  ProviderUsageSnapshot,
  TrayciSettings,
  TrayciSettingsPatch,
  UsageState,
  UsageWindow
} from "../../shared/types";
import { DEFAULT_SETTINGS } from "../../shared/types";
import { formatResetCountdown, tightestWindow } from "../../shared/presentation";

const EMPTY_STATE: UsageState = {
  providers: {},
  isRefreshing: false,
  lastRefreshStartedAt: null,
  lastRefreshCompletedAt: null
};

function maximum(snapshot: ProviderUsageSnapshot): number {
  return Math.max(0, ...snapshot.windows.map((window) => window.usedPercent));
}

function Metric({
  window,
  percentageDisplay,
  now
}: {
  window: UsageWindow;
  percentageDisplay: TrayciSettings["percentageDisplay"];
  now: number;
}): React.JSX.Element {
  const used = Math.min(100, Math.max(0, window.usedPercent));
  const displayed = percentageDisplay === "used" ? used : 100 - used;
  return (
    <div className="metric">
      <div className="metric-copy">
        <span>{window.label}</span>
        <span className="metric-value">
          {Math.round(displayed)}% {percentageDisplay === "used" ? "used" : "left"}
          {window.resetsAt ? <small> · {formatResetCountdown(window.resetsAt, now)}</small> : null}
        </span>
      </div>
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
    </div>
  );
}

function ProviderCard({
  snapshot,
  settings,
  now
}: {
  snapshot: ProviderUsageSnapshot;
  settings: TrayciSettings;
  now: number;
}): React.JSX.Element {
  const windows = settings.displayMode === "compact"
    ? [tightestWindow(snapshot)].filter((window): window is UsageWindow => Boolean(window))
    : snapshot.windows;
  const ageMinutes = Math.max(0, Math.floor((now - snapshot.updatedAt) / 60_000));
  return (
    <section className={`provider provider-${snapshot.status}`} aria-labelledby={`provider-${snapshot.provider}`}>
      <header>
        <div>
          <h2 id={`provider-${snapshot.provider}`}>{snapshot.displayName}</h2>
          {snapshot.plan ? <span className="plan">{snapshot.plan}</span> : null}
        </div>
        {snapshot.status === "fetching" ? <span className="status pulse">Refreshing</span> : null}
        {snapshot.status === "stale" ? <span className="status">Updated {ageMinutes}m ago</span> : null}
      </header>
      {windows.length ? windows.map((window) => (
        <Metric key={window.id} window={window} percentageDisplay={settings.percentageDisplay} now={now} />
      )) : (
        <div className="provider-message">
          {snapshot.status === "fetching" ? "Checking usage…" : snapshot.error ?? "Usage unavailable"}
        </div>
      )}
      {snapshot.error && windows.length ? <div className="inline-error">{snapshot.error}</div> : null}
    </section>
  );
}

function Switch({ checked, onChange, label }: { checked: boolean; onChange(value: boolean): void; label: string }): React.JSX.Element {
  return (
    <label className="setting-row">
      <span>{label}</span>
      <input type="checkbox" role="switch" checked={checked} onChange={(event) => onChange(event.target.checked)} />
    </label>
  );
}

function Settings({
  settings,
  update
}: {
  settings: TrayciSettings;
  update(patch: TrayciSettingsPatch): void;
}): React.JSX.Element {
  return (
    <div className="settings-view">
      <section className="settings-group">
        <h2>General</h2>
        <Switch checked={settings.startOnLogin} onChange={(startOnLogin) => update({ startOnLogin })} label="Start Trayci on login" />
        <Switch checked={settings.refreshOnResume} onChange={(refreshOnResume) => update({ refreshOnResume })} label="Refresh after resume" />
        <label className="setting-row">
          <span>Refresh interval</span>
          <select value={settings.pollIntervalMinutes} onChange={(event) => update({ pollIntervalMinutes: Number(event.target.value) })}>
            {[5, 15, 30, 60].map((minutes) => <option key={minutes} value={minutes}>{minutes} minutes</option>)}
          </select>
        </label>
      </section>
      <section className="settings-group">
        <h2>Providers</h2>
        <Switch checked={settings.providers.claude.enabled} onChange={(enabled) => update({ providers: { claude: { enabled } } })} label="Claude Code" />
        <Switch checked={settings.providers.codex.enabled} onChange={(enabled) => update({ providers: { codex: { enabled } } })} label="Codex" />
      </section>
      <section className="settings-group">
        <h2>Display</h2>
        <label className="setting-row">
          <span>Percentage</span>
          <select value={settings.percentageDisplay} onChange={(event) => update({ percentageDisplay: event.target.value as "used" | "remaining" })}>
            <option value="used">Used</option>
            <option value="remaining">Remaining</option>
          </select>
        </label>
        <label className="setting-row">
          <span>Default view</span>
          <select value={settings.displayMode} onChange={(event) => update({ displayMode: event.target.value as "detailed" | "compact" })}>
            <option value="detailed">Detailed</option>
            <option value="compact">Compact</option>
          </select>
        </label>
      </section>
    </div>
  );
}

export default function App(): React.JSX.Element {
  const [state, setState] = useState(EMPTY_STATE);
  const [settings, setSettings] = useState(DEFAULT_SETTINGS);
  const [view, setView] = useState<"usage" | "settings">("usage");
  const [now, setNow] = useState(Date.now());
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void Promise.all([window.trayci.usage.getState(), window.trayci.settings.get()])
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
    () => Object.values(state.providers)
      .filter((snapshot): snapshot is ProviderUsageSnapshot => Boolean(snapshot))
      .sort((a, b) => maximum(b) - maximum(a)),
    [state.providers]
  );

  const update = (patch: TrayciSettingsPatch): void => {
    setError(null);
    const previous = settings;
    setSettings({
      ...settings,
      ...patch,
      schemaVersion: 1,
      providers: {
        claude: { ...settings.providers.claude, ...patch.providers?.claude },
        codex: { ...settings.providers.codex, ...patch.providers?.codex }
      }
    });
    void window.trayci.settings.update(patch).then(setSettings).catch(() => {
      setSettings(previous);
      setError("Setting could not be saved.");
    });
  };

  return (
    <main>
      <header className="topbar">
        <div>
          <span className="eyebrow">AI coding usage</span>
          <h1>Trayci</h1>
        </div>
        {view === "usage" ? (
          <button
            className="icon-button"
            type="button"
            aria-label="Refresh usage"
            title="Refresh usage"
            disabled={state.isRefreshing}
            onClick={() => void window.trayci.usage.refreshAll().catch(() => setError("Refresh failed."))}
          >
            <span aria-hidden="true" className={state.isRefreshing ? "spin" : ""}>↻</span>
          </button>
        ) : null}
      </header>

      <nav className="tabs" aria-label="Trayci view">
        <button type="button" aria-current={view === "usage"} onClick={() => setView("usage")}>Usage</button>
        <button type="button" aria-current={view === "settings"} onClick={() => setView("settings")}>Settings</button>
      </nav>

      {error ? <div className="banner" role="alert">{error}</div> : null}

      {view === "usage" ? (
        <div className="usage-view">
          <div className="density" aria-label="Information density">
            {(["detailed", "compact"] as const).map((mode) => (
              <button key={mode} type="button" aria-pressed={settings.displayMode === mode} onClick={() => update({ displayMode: mode })}>
                {mode === "detailed" ? "Detailed" : "Compact"}
              </button>
            ))}
          </div>
          <div className="provider-list" aria-live="polite">
            {providers.length ? providers.map((snapshot) => (
              <ProviderCard key={snapshot.provider} snapshot={snapshot} settings={settings} now={now} />
            )) : <div className="empty">No providers enabled.</div>}
          </div>
        </div>
      ) : <Settings settings={settings} update={update} />}

      <footer>
        <span>{state.lastRefreshCompletedAt ? `Updated ${Math.max(0, Math.floor((now - state.lastRefreshCompletedAt) / 60_000))}m ago` : "Local only"}</span>
        <button type="button" onClick={() => void window.trayci.app.quit()}>Quit Trayci</button>
      </footer>
    </main>
  );
}
