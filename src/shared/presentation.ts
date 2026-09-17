import type { ProviderUsageSnapshot, UsageWindow } from "./types";

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

/** Minutes as the coarsest unit that still reads at a glance: `45m`, `2h 15m`, `4d 12h`. */
function formatDuration(minutes: number): string {
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}

export function formatAge(
  updatedAt: number,
  now: number,
  never = "now",
): string {
  const minutes = Math.max(0, Math.floor((now - updatedAt) / 60_000));
  return minutes
    ? `Updated ${formatDuration(minutes)} ago`
    : `Updated ${never}`;
}

/** Mirrors STALE_AFTER_MS in src-tauri/core/src/service.rs. */
export const STALE_AFTER_MS = 30 * 60_000;

/**
 * The line a provider row shows beside its name: how old a stale reading is, or when the tightest
 * window resets. A row with no windows says nothing here and shows its error instead.
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
      text: `Resets in ${formatResetCountdown(tightest.resetsAt, now)}`,
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
