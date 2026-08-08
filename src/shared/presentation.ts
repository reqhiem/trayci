import type { ProviderUsageSnapshot, UsageWindow } from "./types";

type Rectangle = { x: number; y: number; width: number; height: number };

export function calculatePopoverPosition(
  tray: Rectangle,
  workArea: Rectangle,
  size: { width: number; height: number },
): { x: number; y: number } {
  const trayCenterX = tray.x + tray.width / 2;
  const trayCenterY = tray.y + tray.height / 2;
  const x = Math.round(
    Math.min(
      workArea.x + workArea.width - size.width,
      Math.max(workArea.x, trayCenterX - size.width / 2),
    ),
  );
  const above = trayCenterY > workArea.y + workArea.height / 2;
  const preferredY = above
    ? tray.y - size.height - 8
    : tray.y + tray.height + 8;
  const y = Math.round(
    Math.min(
      workArea.y + workArea.height - size.height,
      Math.max(workArea.y, preferredY),
    ),
  );
  return { x, y };
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

export function formatResetCountdown(resetsAt: number, now: number): string {
  const minutes = Math.max(0, Math.floor((resetsAt - now) / 60_000));
  if (!minutes) return "now";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ${minutes % 60}m`;
  return `${Math.floor(hours / 24)}d ${hours % 24}h`;
}
