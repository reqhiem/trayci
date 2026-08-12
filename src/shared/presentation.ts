import type { ProviderUsageSnapshot, UsageWindow } from "./types";

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

export function formatAge(
  updatedAt: number,
  now: number,
  never = "now",
): string {
  const minutes = Math.max(0, Math.floor((now - updatedAt) / 60_000));
  return minutes ? `Updated ${minutes}m ago` : `Updated ${never}`;
}

/**
 * The line a provider row shows beside its name: how old a stale reading is, or when the tightest
 * window resets. A row with no windows says nothing here and shows its error instead.
 */
export function statusSummary(
  snapshot: ProviderUsageSnapshot,
  now: number,
): { text: string; stale: boolean } | null {
  const tightest = tightestWindow(snapshot);
  if (snapshot.status === "stale")
    return { text: formatAge(snapshot.updatedAt, now), stale: true };
  if (tightest?.resetsAt)
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
  if (!minutes) return "now";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}
