import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type {
  ProviderId,
  ProviderUsageSnapshot,
  TrayciSettings,
  TrayciSettingsPatch,
  UsageState,
  UsageWindow,
} from "../../shared/types";
import { DEFAULT_SETTINGS, FONT_SCALES } from "../../shared/types";
import {
  formatAge,
  formatResetCountdown,
  statusSummary,
  tightestWindow,
} from "../../shared/presentation";
import { trayci } from "./bridge";

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
  const progressClass =
    used < 50
      ? "progress-low"
      : used < 85
        ? "progress-medium"
        : used < 90
          ? "progress-high"
          : "progress-critical";

  return (
    <div className={dense ? "metric metric-dense" : "metric"}>
      <div className="metric-copy">
        <strong>{dense ? denseLabel(window) : window.label}</strong>
      </div>
      <div className="metric-line">
        <div
          className={`progress ${progressClass}`}
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
  pinned,
  onPin,
  onReveal,
}: {
  snapshot: ProviderUsageSnapshot;
  settings: TrayciSettings;
  now: number;
  pinned: boolean;
  onPin(): void;
  onReveal(): void;
}): React.JSX.Element {
  const tightest = tightestWindow(snapshot);
  const summary = statusSummary(snapshot, now);
  return (
    <button
      className={`provider-row provider-${snapshot.status}`}
      type="button"
      aria-busy={snapshot.status === "fetching"}
      aria-expanded={pinned}
      onClick={onPin}
      onMouseEnter={onReveal}
      onFocus={onReveal}
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
        ) : summary ? (
          <span className={summary.stale ? "status" : "reset-summary"}>
            {summary.text}
          </span>
        ) : null}
        <span className="chevron" aria-hidden="true" />
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
  pinned,
}: {
  snapshot: ProviderUsageSnapshot;
  settings: TrayciSettings;
  now: number;
  pinned: boolean;
}): React.JSX.Element {
  return (
    <aside
      className="provider-detail"
      aria-label={`${snapshot.displayName} details`}
    >
      <header data-tauri-drag-region="deep">
        <div className="detail-title">
          <ProviderIcon provider={snapshot.provider} />
          <strong>{snapshot.displayName}</strong>
          {snapshot.plan ? <span className="plan">{snapshot.plan}</span> : null}
        </div>
        <span>
          {formatAge(snapshot.updatedAt, now, "just now")}
          {pinned ? " · pinned" : ""}
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

function Choice<Value extends string | number>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: Value;
  options: readonly (readonly [Value, string])[];
  onChange(value: Value): void;
}): React.JSX.Element {
  return (
    <div className="setting-row">
      <span>{label}</span>
      <div className="choice-group" role="group" aria-label={label}>
        {options.map(([option, text]) => (
          <button
            key={option}
            type="button"
            aria-pressed={value === option}
            onClick={() => onChange(option)}
          >
            {text}
          </button>
        ))}
      </div>
    </div>
  );
}

function Group({
  title,
  open,
  children,
}: {
  title: string;
  open?: boolean;
  children: React.ReactNode;
}): React.JSX.Element {
  return (
    <details className="settings-group" open={open}>
      <summary>
        <h2>{title}</h2>
      </summary>
      {children}
    </details>
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
      <Group title="General" open>
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
        <Choice
          label="Refresh interval"
          value={settings.pollIntervalMinutes}
          options={[5, 15, 30, 60].map(
            (minutes) => [minutes, `${minutes}m`] as const,
          )}
          onChange={(pollIntervalMinutes) => update({ pollIntervalMinutes })}
        />
      </Group>
      <Group title="Appearance" open>
        <Choice
          label="Theme"
          value={settings.theme}
          options={[
            ["system", "Auto"],
            ["light", "Light"],
            ["dark", "Dark"],
          ]}
          onChange={(theme) => update({ theme })}
        />
        <Choice
          label="Text size"
          value={settings.fontScale}
          options={FONT_SCALES}
          onChange={(fontScale) => update({ fontScale })}
        />
        <Choice
          label="Percentage"
          value={settings.percentageDisplay}
          options={[
            ["used", "Used"],
            ["remaining", "Remaining"],
          ]}
          onChange={(percentageDisplay) => update({ percentageDisplay })}
        />
      </Group>
      <Group title="Notifications">
        {([50, 85, 90] as const).map((threshold) => (
          <Switch
            key={threshold}
            checked={settings.notifications[`quota${threshold}`]}
            onChange={(enabled) =>
              update({
                notifications: {
                  ...settings.notifications,
                  [`quota${threshold}`]: enabled,
                },
              })
            }
            label={`Quota reaches ${threshold}%`}
          />
        ))}
        <Switch
          checked={settings.notifications.reset}
          onChange={(reset) =>
            update({ notifications: { ...settings.notifications, reset } })
          }
          label="Quota reset completes"
        />
      </Group>
      <Group title="Providers">
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
      </Group>
      <Group title="Window">
        <div className="setting-row">
          <span>Drag the title bar to move the popup (X11 sessions only)</span>
          <button
            className="row-button"
            type="button"
            onClick={() => update({ windowPosition: null })}
          >
            Reset position
          </button>
        </div>
      </Group>
      <button
        className="quit-button"
        type="button"
        onClick={() => void trayci.app.quit()}
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
  const [pinnedProvider, setPinnedProvider] = useState<ProviderId | null>(null);
  const [hoveredProvider, setHoveredProvider] = useState<ProviderId | null>(
    null,
  );
  const [now, setNow] = useState(Date.now());
  const [error, setError] = useState<string | null>(null);
  const master = useRef<HTMLElement>(null);
  const backButton = useRef<HTMLButtonElement>(null);
  const settingsButton = useRef<HTMLButtonElement>(null);
  const skipViewFocus = useRef(true);

  useEffect(() => {
    void Promise.all([trayci.usage.getState(), trayci.settings.get()])
      .then(([nextState, nextSettings]) => {
        setState(nextState);
        setSettings(nextSettings);
      })
      .catch(() => setError("Trayci could not load its state."));
    const unsubscribe = trayci.usage.subscribe(setState);
    const timer = window.setInterval(() => setNow(Date.now()), 60_000);
    return () => {
      unsubscribe();
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    if (skipViewFocus.current) {
      skipViewFocus.current = false;
      return;
    }
    (view === "settings" ? backButton : settingsButton).current?.focus();
  }, [view]);

  useEffect(() => {
    const system = window.matchMedia("(prefers-color-scheme: light)");
    const apply = (): void => {
      document.documentElement.dataset.theme =
        settings.theme === "system"
          ? system.matches
            ? "light"
            : "dark"
          : settings.theme;
    };
    apply();
    system.addEventListener("change", apply);
    return () => system.removeEventListener("change", apply);
  }, [settings.theme]);

  // Tauri moves the window itself; the backend only needs to know the move was ours to keep.
  // Capture phase is required: Tauri's own mousedown listener on `document` calls
  // stopImmediatePropagation() before starting the drag, so a bubbling listener never runs.
  useEffect(() => {
    const start = (event: MouseEvent): void => {
      if ((event.target as HTMLElement).closest?.("[data-tauri-drag-region]"))
        void trayci.app.beginDrag();
    };
    window.addEventListener("mousedown", start, true);
    return () => window.removeEventListener("mousedown", start, true);
  }, []);

  useEffect(() => {
    const escape = (event: KeyboardEvent): void => {
      if (event.key !== "Escape") return;
      if (view === "usage" && pinnedProvider) setPinnedProvider(null);
      else void trayci.app.hidePopover();
    };
    window.addEventListener("keydown", escape);
    return () => window.removeEventListener("keydown", escape);
  }, [view, pinnedProvider]);

  const providers = useMemo(
    () =>
      Object.values(state.providers)
        .filter((snapshot): snapshot is ProviderUsageSnapshot =>
          Boolean(snapshot),
        )
        .sort((a, b) => maximum(b) - maximum(a)),
    [state.providers],
  );
  const revealed = pinnedProvider ?? hoveredProvider;
  const selected =
    view === "usage"
      ? (providers.find((snapshot) => snapshot.provider === revealed) ?? null)
      : null;
  const targetWidth = selected ? 680 : 360;

  // Measured on the master column alone: the detail pane scrolls instead of resizing the window,
  // so revealing it on hover never shifts the rows under the pointer.
  useLayoutEffect(() => {
    const element = master.current;
    if (!element) return;
    let frame = 0;
    const resize = (): void => {
      window.cancelAnimationFrame(frame);
      frame = window.requestAnimationFrame(
        () => void trayci.app.resizePopover(targetWidth, element.scrollHeight),
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
      notifications: {
        ...settings.notifications,
        ...patch.notifications,
      },
      providers: {
        claude: { ...settings.providers.claude, ...patch.providers?.claude },
        codex: { ...settings.providers.codex, ...patch.providers?.codex },
        antigravity: {
          ...settings.providers.antigravity,
          ...patch.providers?.antigravity,
        },
      },
    });
    void trayci.settings
      .update(patch)
      .then(setSettings)
      .catch(() => {
        setSettings(previous);
        setError("Setting could not be saved.");
      });
  };

  return (
    // Hover is only dropped when the pointer leaves the whole popover: the detail pane arriving
    // under the pointer used to count as leaving the row, which closed and reopened it forever.
    <main
      className={selected ? "has-detail" : ""}
      onMouseLeave={() => setHoveredProvider(null)}
    >
      <section className="master" ref={master}>
        <header className="topbar" data-tauri-drag-region="deep">
          {view === "settings" ? (
            <button
              ref={backButton}
              className="back-button"
              type="button"
              aria-label="Back to usage"
              onClick={() => setView("usage")}
            />
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
                  void trayci.usage
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
              ref={settingsButton}
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
                  onClick={() => update({ displayMode: mode })}
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
                    pinned={pinnedProvider === snapshot.provider}
                    onReveal={() => setHoveredProvider(snapshot.provider)}
                    onPin={() =>
                      setPinnedProvider(
                        pinnedProvider === snapshot.provider
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
        <ProviderDetail
          snapshot={selected}
          settings={settings}
          now={now}
          pinned={pinnedProvider === selected.provider}
        />
      ) : null}
    </main>
  );
}
