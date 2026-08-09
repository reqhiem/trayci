import { expect, it, vi } from "vitest";
import type { ProviderUsageSnapshot, UsageState } from "../../src/shared/types";
import { DEFAULT_SETTINGS } from "../../src/shared/types";
import { QuotaNotifier } from "../../src/main/notifications";

function state(
  usedPercent: number,
  resetsAt: number,
  status: ProviderUsageSnapshot["status"] = "ok",
): UsageState {
  const snapshot: ProviderUsageSnapshot = {
    provider: "codex",
    displayName: "Codex",
    status,
    plan: null,
    windows: [
      {
        id: "weekly",
        label: "Weekly",
        usedPercent,
        durationMinutes: 10_080,
        resetsAt,
        resetDescription: null,
      },
    ],
    updatedAt: 0,
    source: "rpc",
    error: null,
  };
  return {
    providers: { codex: snapshot },
    isRefreshing: false,
    lastRefreshStartedAt: null,
    lastRefreshCompletedAt: null,
  };
}

it("notifies once per threshold and again after a reset", () => {
  const notify = vi.fn();
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.notifications = {
    quota80: true,
    quota90: true,
    quota95: true,
    reset: true,
  };
  const notifier = new QuotaNotifier(() => settings, notify);

  notifier.update(state(91, 100), 0);
  notifier.update(state(96, 100), 50);
  notifier.update(state(96, 100), 50);
  notifier.update(state(2, 200), 100);
  notifier.update(state(81, 200), 101);

  expect(notify.mock.calls.map(([title]) => title)).toEqual([
    "Codex quota at 90%",
    "Codex quota at 95%",
    "Codex quota reset",
    "Codex quota at 80%",
  ]);
});

it("uses the cached snapshot to avoid duplicates after restart", () => {
  const notify = vi.fn();
  const settings = structuredClone(DEFAULT_SETTINGS);
  settings.notifications.quota80 = true;
  const notifier = new QuotaNotifier(() => settings, notify);

  notifier.update(state(85, 100, "stale"), 0);
  notifier.update(state(86, 100), 1);

  expect(notify).not.toHaveBeenCalled();
});
