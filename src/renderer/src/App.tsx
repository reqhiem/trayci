import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type {
  BuiltInProviderId,
  ProviderId,
  ProviderUsageSnapshot,
  TrayciSettings,
  TrayciSettingsPatch,
  UsageState,
  UsageWindow,
} from "../../shared/types";
import { DEFAULT_SETTINGS, FONT_SCALES } from "../../shared/types";
import {
  detailLabel,
  displayPercent,
  formatAge,
  formatResetCountdown,
  providerRank,
  rowLabel,
  severity,
  SOURCE_LABELS,
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

/** A bar that always shows use, colored by how much attention it needs. */
function Meter({
  window,
  title,
}: {
  window: UsageWindow;
  title?: string;
}): React.JSX.Element {
  const used = Math.min(100, Math.max(0, window.usedPercent));
  return (
    <span
      className={`meter meter-${severity(used)}`}
      title={title}
      role="progressbar"
      aria-label={`${detailLabel(window)} used`}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={Math.round(used)}
    >
      <span style={{ width: `${used}%` }} />
    </span>
  );
}

/** The number beside a meter. Red once the window is nearly spent, whichever way it counts. */
function Percent({
  window,
  mode,
}: {
  window: UsageWindow;
  mode: TrayciSettings["percentageDisplay"];
}): React.JSX.Element {
  return (
    <span
      className={
        severity(window.usedPercent) === "critical"
          ? "percent critical"
          : "percent"
      }
    >
      {displayPercent(window, mode)}%
    </span>
  );
}

/**
 * Where each row meter sits: two per line, left then right, so the bars of a column line up. A
 * label too long for half the row is never cut; it takes the whole line instead.
 */
const WIDE_LABEL = 14; // characters; "Claude/GPT wk" still fits half the row at any zoom.
function meterSlots(windows: readonly UsageWindow[]): {
  window: UsageWindow;
  label: string;
  slot: "left" | "right" | "wide";
  line: number;
}[] {
  let line = 0;
  let right = false;
  return windows.map((window) => {
    const label = rowLabel(window);
    if (label.length > WIDE_LABEL) {
      right = false;
      return { window, label, slot: "wide", line: ++line };
    }
    const slot = right ? "right" : "left";
    if (!right) line++;
    right = !right;
    return { window, label, slot, line };
  });
}

function Plan({ plan }: { plan: string | null }): React.JSX.Element | null {
  return plan ? <span className="plan">{plan}</span> : null;
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
  const compact = settings.displayMode === "compact";
  const tightest = tightestWindow(snapshot);
  // statusSummary keeps the age check from #54: an aged reading never shows a healthy countdown.
  const summary = statusSummary(snapshot, now);
  const message = snapshot.windows.length
    ? null
    : snapshot.status === "fetching"
      ? "Loading…"
      : (snapshot.error ?? "Usage unavailable");
  return (
    <button
      className={`provider-row provider-${snapshot.status}${compact ? " provider-row-compact" : ""}`}
      type="button"
      aria-busy={snapshot.status === "fetching"}
      aria-expanded={pinned}
      onClick={onPin}
      onMouseEnter={onReveal}
      onFocus={onReveal}
    >
      <span className="provider-summary">
        <ProviderIcon provider={snapshot.provider} />
        <strong>{snapshot.displayName}</strong>
        <Plan plan={snapshot.plan} />
        <span className="spacer" />
        {summary ? (
          <span className={summary.stale ? "summary stale" : "summary"}>
            {summary.stale || compact
              ? summary.text
              : `resets in ${summary.text}`}
          </span>
        ) : null}
        {compact && tightest ? (
          <>
            <Meter window={tightest} title={detailLabel(tightest)} />
            <Percent window={tightest} mode={settings.percentageDisplay} />
          </>
        ) : null}
        {compact && message ? <span className="summary">{message}</span> : null}
        <span className="chevron" aria-hidden="true" />
      </span>
      {!compact && snapshot.windows.length ? (
        <span className="row-meters">
          {meterSlots(snapshot.windows).map(({ window, label, slot, line }) => (
            <span
              className={`row-meter slot-${slot}`}
              key={window.id}
              style={{ "--line": line } as React.CSSProperties}
            >
              <span className="row-meter-label">{label}</span>
              <Meter window={window} />
              <Percent window={window} mode={settings.percentageDisplay} />
            </span>
          ))}
        </span>
      ) : null}
      {!compact && message ? (
        <span className="provider-message">{message}</span>
      ) : null}
    </button>
  );
}

function ProviderDetail({
  snapshot,
  settings,
  now,
  update,
}: {
  snapshot: ProviderUsageSnapshot;
  settings: TrayciSettings;
  now: number;
  update(patch: TrayciSettingsPatch): void;
}): React.JSX.Element {
  const mode = settings.percentageDisplay;
  return (
    <aside
      className="provider-detail"
      aria-label={`${snapshot.displayName} details`}
    >
      <header data-tauri-drag-region="deep">
        <div className="detail-title">
          <ProviderIcon provider={snapshot.provider} />
          <strong>{snapshot.displayName}</strong>
          <Plan plan={snapshot.plan} />
          <span className="spacer" />
          <button
            className="mode-toggle"
            type="button"
            title={mode === "used" ? "Show remaining" : "Show used"}
            aria-label={
              mode === "used" ? "Used, show remaining" : "Remaining, show used"
            }
            onClick={() =>
              update({
                percentageDisplay: mode === "used" ? "remaining" : "used",
              })
            }
          >
            {mode === "used" ? "Used" : "Remaining"}
            <span aria-hidden="true"> ⇅</span>
          </button>
        </div>
        <span
          className="detail-age"
          title={snapshot.source ? SOURCE_LABELS[snapshot.source] : undefined}
        >
          {formatAge(snapshot.updatedAt, now)}
        </span>
      </header>
      {snapshot.windows.length ? (
        <ul className="detail-windows">
          {snapshot.windows.map((window) => (
            <li key={window.id} title={window.group?.models ?? undefined}>
              <span className="detail-line">
                <span>
                  <strong>{detailLabel(window)}</strong>
                  {window.resetsAt
                    ? ` · resets in ${formatResetCountdown(window.resetsAt, now)}`
                    : null}
                </span>
                <Percent window={window} mode={mode} />
              </span>
              <Meter window={window} />
            </li>
          ))}
        </ul>
      ) : null}
      {snapshot.error ? (
        <div className="inline-error">{snapshot.error}</div>
      ) : null}
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

const PROVIDER_LABELS: readonly (readonly [BuiltInProviderId, string])[] = [
  ["claude", "Claude Code"],
  ["codex", "Codex"],
  ["antigravity", "Antigravity"],
];

/**
 * The X and Y the popover is placed at, as an alternative to dragging it (issue #39).
 *
 * Both coordinates travel together: committing one alone would send the popover to a corner it
 * was never asked to go to. What is typed is kept until the field is left, so a half-typed number
 * is not a position.
 */
function PositionRow({
  value,
  disabled,
  onChange,
}: {
  value: [number, number] | null;
  disabled: boolean;
  onChange(position: [number, number] | null): void;
}): React.JSX.Element {
  const [draft, setDraft] = useState<{ x: string; y: string } | null>(null);
  const shown = draft ?? {
    x: value ? String(value[0]) : "",
    y: value ? String(value[1]) : "",
  };
  // The draft is only dropped once it has become a position. Clearing it on every blur threw away
  // whichever field was filled first, so with no position saved yet, tabbing from X to Y emptied X
  // and neither could ever be set.
  const commit = (): void => {
    const [x, y] = [Number(shown.x), Number(shown.y)];
    if (shown.x === "" || shown.y === "") return;
    if (!Number.isInteger(x) || !Number.isInteger(y)) return;
    setDraft(null);
    onChange([x, y]);
  };
  return (
    <div
      className="setting-row"
      title={disabled ? "Wayland ignores a typed position" : undefined}
    >
      <span>Position</span>
      <span className="position-inputs">
        {(["x", "y"] as const).map((axis) => (
          <label key={axis}>
            {axis.toUpperCase()}
            <input
              type="number"
              step={1}
              disabled={disabled}
              value={shown[axis]}
              onChange={(event) =>
                setDraft({ ...shown, [axis]: event.target.value })
              }
              onBlur={commit}
              onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
              }}
            />
          </label>
        ))}
        <button
          className="row-button"
          type="button"
          title="Back to the tray icon"
          onClick={() => {
            setDraft(null);
            onChange(null);
          }}
        >
          Reset
        </button>
      </span>
    </div>
  );
}

function Settings({
  settings,
  update,
}: {
  settings: TrayciSettings;
  update(patch: TrayciSettingsPatch): void;
}): React.JSX.Element {
  const [canPosition, setCanPosition] = useState(false);
  useEffect(() => {
    void trayci.app
      .positionsAreReal()
      .then(setCanPosition)
      .catch(() => setCanPosition(false));
  }, []);

  const ordered = [...PROVIDER_LABELS].sort(
    ([left], [right]) =>
      providerRank(settings.providerOrder, left) -
      providerRank(settings.providerOrder, right),
  );
  // The whole list is written, not just the pair that swapped: a partial order would leave the
  // providers it omits to the usage sort, which is what the user is overriding.
  const move = (index: number, delta: number): void => {
    const ids = ordered.map(([provider]) => provider);
    const [moved, displaced] = [ids[index], ids[index + delta]];
    if (!moved || !displaced) return;
    ids[index] = displaced;
    ids[index + delta] = moved;
    update({ providerOrder: ids });
  };

  return (
    <div className="settings-view">
      <Group title="General" open>
        <Switch
          checked={settings.startOnLogin}
          onChange={(startOnLogin) => update({ startOnLogin })}
          label="Start on login"
        />
        <Switch
          checked={settings.refreshOnResume}
          onChange={(refreshOnResume) => update({ refreshOnResume })}
          label="Refresh after resume"
        />
        <Choice
          label="Refresh every"
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
            label={`${threshold}% used`}
          />
        ))}
        <Switch
          checked={settings.notifications.reset}
          onChange={(reset) =>
            update({ notifications: { ...settings.notifications, reset } })
          }
          label="Quota resets"
        />
      </Group>
      <Group title="Providers">
        {ordered.map(([provider, label], index) => (
          <div className="setting-row provider-setting" key={provider}>
            <span
              className="choice-group"
              role="group"
              aria-label={`Reorder ${label}`}
            >
              <button
                type="button"
                aria-label={`Move ${label} up`}
                disabled={index === 0}
                onClick={() => move(index, -1)}
              >
                ↑
              </button>
              <button
                type="button"
                aria-label={`Move ${label} down`}
                disabled={index === ordered.length - 1}
                onClick={() => move(index, 1)}
              >
                ↓
              </button>
            </span>
            <label htmlFor={`provider-${provider}`}>{label}</label>
            <input
              id={`provider-${provider}`}
              type="checkbox"
              role="switch"
              checked={settings.providers[provider].enabled}
              onChange={(event) =>
                update({
                  providers: { [provider]: { enabled: event.target.checked } },
                })
              }
            />
          </div>
        ))}
      </Group>
      <Group title="Window">
        <PositionRow
          value={settings.windowPosition}
          disabled={!canPosition}
          onChange={(windowPosition) => update({ windowPosition })}
        />
      </Group>
      <button
        className="quit-button"
        type="button"
        onClick={() => void trayci.app.quit()}
      >
        Quit
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

  // Header drag. Tauri's own drag script moves the window: its mousedown listener on `document`
  // asks the window manager to take over. This one only tells the backend a drag began, so the
  // moves that follow count as the user's and a focus-out during them does not close the popover.
  // Capture phase is required: that script calls stopImmediatePropagation(), so a bubbling listener
  // never runs.
  useEffect(() => {
    let dragging = false;
    const start = (event: MouseEvent): void => {
      const target = event.target as HTMLElement;
      // Mirror what Tauri's own drag script does with a clickable element inside a drag region: it
      // refuses to drag there. Claiming a drag anyway left the flag set for a window move we never
      // made, which the backend would then have stored as a position the user chose.
      if (
        target.closest?.("button, a, input, select, textarea, label, summary")
      )
        return;
      if (!target.closest?.("[data-tauri-drag-region]")) return;
      dragging = true;
      void trayci.app.beginDrag();
    };
    // The window manager holds the pointer while it moves the window, so the release never reaches
    // the webview. The drag is over at the first event that arrives with the button already up.
    const stop = (event: MouseEvent): void => {
      if (!dragging || event.buttons & 1) return;
      dragging = false;
      void trayci.app.endDrag();
    };
    window.addEventListener("mousedown", start, true);
    window.addEventListener("mousemove", stop, true);
    window.addEventListener("mouseup", stop, true);
    return () => {
      window.removeEventListener("mousedown", start, true);
      window.removeEventListener("mousemove", stop, true);
      window.removeEventListener("mouseup", stop, true);
    };
  }, []);

  // The popover is hidden, not unloaded, so a hover survives until the next pointer move. Without
  // this it would reopen showing the detail pane of whatever row the pointer last passed over.
  useEffect(() => {
    const clear = (): void => setHoveredProvider(null);
    window.addEventListener("blur", clear);
    return () => window.removeEventListener("blur", clear);
  }, []);

  // A drag writes windowPosition straight to settings, and the webview is only ever hidden, so
  // without this the Window group would keep showing the position loaded when it was created.
  useEffect(() => {
    if (view !== "settings") return;
    void trayci.settings
      .get()
      .then(setSettings)
      .catch(() => setError("Trayci could not load its state."));
  }, [view]);

  useEffect(() => {
    const close = (): void => {
      if (view === "usage" && pinnedProvider) setPinnedProvider(null);
      else void trayci.app.hidePopover();
    };
    const escape = (event: KeyboardEvent): void => {
      if (event.key === "Escape") close();
    };
    window.addEventListener("keydown", escape);
    const unlisten = trayci.app.onEscape(close);
    return () => {
      window.removeEventListener("keydown", escape);
      unlisten();
    };
  }, [view, pinnedProvider]);

  // The order set in Settings wins. Providers it does not mention rank equal, so they keep the
  // usage ordering behind the ones it does — and an empty order leaves usage in charge of all.
  const order = settings.providerOrder;
  const providers = useMemo(
    () =>
      Object.values(state.providers)
        .filter((snapshot): snapshot is ProviderUsageSnapshot =>
          Boolean(snapshot),
        )
        .sort(
          (a, b) =>
            providerRank(order, a.provider) - providerRank(order, b.provider) ||
            maximum(b) - maximum(a),
        ),
    [state.providers, order],
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
          {view === "usage" ? (
            <>
              <button
                className="icon-button"
                type="button"
                aria-label="Compact view"
                aria-pressed={settings.displayMode === "compact"}
                title={
                  settings.displayMode === "compact"
                    ? "Detailed view"
                    : "Compact view"
                }
                onClick={() =>
                  update({
                    displayMode:
                      settings.displayMode === "compact"
                        ? "detailed"
                        : "compact",
                  })
                }
              >
                <svg viewBox="0 0 24 24" aria-hidden="true">
                  <path
                    d={
                      settings.displayMode === "compact"
                        ? "m7 15 5 5 5-5M7 9l5-5 5 5"
                        : "m7 20 5-5 5 5M7 4l5 5 5-5"
                    }
                  />
                </svg>
              </button>
              <button
                className="icon-button"
                type="button"
                aria-label="Refresh usage"
                title={
                  state.lastRefreshCompletedAt
                    ? `Refresh · ${formatAge(state.lastRefreshCompletedAt, now)}`
                    : "Refresh"
                }
                disabled={state.isRefreshing}
                onClick={() =>
                  void trayci.usage
                    .refreshAll()
                    .catch(() => setError("Refresh failed."))
                }
              >
                <svg
                  className={state.isRefreshing ? "spin" : undefined}
                  viewBox="0 0 24 24"
                  aria-hidden="true"
                >
                  <path d="M21 12a9 9 0 1 1-9-9c2.52 0 4.93 1 6.74 2.74L21 8M21 3v5h-5" />
                </svg>
              </button>
            </>
          ) : null}
          <button
            ref={settingsButton}
            className="icon-button"
            type="button"
            aria-label={view === "usage" ? "Settings" : "Back to usage"}
            title={view === "usage" ? "Settings" : "Back to usage"}
            onClick={() => setView(view === "usage" ? "settings" : "usage")}
          >
            {view === "usage" ? (
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
                <circle cx="12" cy="12" r="3" />
              </svg>
            ) : (
              <svg viewBox="0 0 24 24" aria-hidden="true">
                <path d="M18 6 6 18M6 6l12 12" />
              </svg>
            )}
          </button>
        </header>

        {error ? (
          <div className="banner" role="alert">
            {error}
          </div>
        ) : null}

        {view === "usage" ? (
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
        ) : (
          <Settings settings={settings} update={update} />
        )}
      </section>
      {view === "usage" && selected ? (
        <ProviderDetail
          snapshot={selected}
          settings={settings}
          now={now}
          update={update}
        />
      ) : null}
    </main>
  );
}
