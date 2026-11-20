import type { ProviderUsageSnapshot, UsageSource, UsageWindow } from "./types";

/**
 * Where a provider sits in the order the user arranged, for sorting. Ids the order does not
 * mention rank equal and last, so they keep whatever order the caller sorts by next — and an
 * empty order ranks everything equal, which leaves the usage ordering untouched.
 */
export function providerRank(order: readonly string[], id: string): number {
  const index = order.indexOf(id);
  return index < 0 ? order.length : index;
}

export function tightestWindow(
  snapshot: ProviderUsageSnapshot,
): UsageWindow | null {
  return snapshot.windows.reduce<UsageWindow | null>(
    (current, candidate) =>
      !current || candidate.usedPercent > current.usedPercent
        ? candidate
        : current,
    null,
  );
}

/** Minutes as the coarsest unit that still reads at a glance: `45m`, `3h 54m`, `5d 1h`. */
function formatDuration(minutes: number): string {
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

export function formatAge(updatedAt: number, now: number): string {
  const minutes = Math.max(0, Math.floor((now - updatedAt) / 60_000));
  return minutes
    ? `Updated ${formatDuration(minutes)} ago`
    : "Updated just now";
}

/** Where a reading came from, so a scraped or cached number is not mistaken for the API's. */
export const SOURCE_LABELS: Record<UsageSource, string> = {
  oauth: "via API",
  api: "via API",
  rpc: "via RPC",
  cli: "via CLI",
  cache: "from cache",
};

/** Mirrors STALE_AFTER_MS in src-tauri/core/src/service.rs. */
export const STALE_AFTER_MS = 30 * 60_000;

/**
 * What a provider row shows beside its name: how old a stale reading is ("Updated 2h ago"), or the
 * bare countdown to the tightest window's reset ("3h 54m"), which the detailed row reads as
 * "resets in 3h 54m". A row with no windows says nothing here and shows its error instead.
 *
 * The status field only records how old a snapshot was when it was loaded, so age is re-checked
 * here: an aged reading must not render a healthy countdown just because it loaded as `ok`.
 */
export function statusSummary(
  snapshot: ProviderUsageSnapshot,
  now: number,
): { text: string; stale: boolean } | null {
  const tightest = tightestWindow(snapshot);
  const aged = now - snapshot.updatedAt > STALE_AFTER_MS;
  if (snapshot.status === "stale")
    return { text: formatAge(snapshot.updatedAt, now), stale: true };
  if (tightest?.resetsAt && !aged)
    return {
      text: formatResetCountdown(tightest.resetsAt, now),
      stale: false,
    };
  if (snapshot.windows.length)
    return { text: formatAge(snapshot.updatedAt, now), stale: true };
  return null;
}

export function formatResetCountdown(resetsAt: number, now: number): string {
  const minutes = Math.max(0, Math.floor((resetsAt - now) / 60_000));
  return minutes ? formatDuration(minutes) : "now";
}

/**
 * A pool's plain name: agy's name minus a trailing "models", with " and " as a slash. Mirrors
 * `pool_name` in src-tauri/core/src/providers/antigravity.rs, so the TUI and the popover agree:
 * "Gemini Models" → "Gemini", "Claude and GPT models" → "Claude/GPT".
 */
export function poolName(name: string): string {
  return name
    .trim()
    .replace(/\s+models$/i, "")
    .replace(/ and /g, "/");
}

/**
 * The window's length. An ungrouped window is keyed on its label, because Claude's per-model
 * weekly windows ("Fable") share the weekly duration and must keep their name. A pooled window is
 * keyed on its duration; any other length is its label without the pool's name in front.
 */
function cadence(window: UsageWindow, weekly: string): string {
  if (!window.group) return window.label === "Weekly" ? weekly : window.label;
  if (window.durationMinutes === 300) return "5h";
  if (window.durationMinutes === 10_080) return weekly;
  const pool = poolName(window.group.name);
  return window.label.startsWith(`${pool} `)
    ? window.label.slice(pool.length + 1)
    : "";
}

/** A meter in a provider row: "Gemini 5h", "Claude/GPT wk", "Gemini 4 Argon wk", "5h", "Fable". */
export function rowLabel(window: UsageWindow): string {
  const length = cadence(window, "wk");
  if (!window.group) return length;
  return [poolName(window.group.name), length].filter(Boolean).join(" ");
}

/** An entry in the detail pane: "Gemini · Weekly", "Claude/GPT · 5h", "Weekly", "Fable". */
export function detailLabel(window: UsageWindow): string {
  const length = cadence(window, "Weekly");
  if (!window.group) return length;
  return [poolName(window.group.name), length].filter(Boolean).join(" · ");
}

/** The number a meter shows: an integer, used or remaining per the setting. */
export function displayPercent(
  window: UsageWindow,
  mode: "used" | "remaining",
): number {
  const used = Math.min(100, Math.max(0, window.usedPercent));
  return Math.round(mode === "used" ? used : 100 - used);
}

/** How much attention a meter needs, at the notification thresholds (50/85/90). Always by use. */
export function severity(
  usedPercent: number,
): "low" | "medium" | "high" | "critical" {
  if (usedPercent >= 90) return "critical";
  if (usedPercent >= 85) return "high";
  if (usedPercent >= 50) return "medium";
  return "low";
}
