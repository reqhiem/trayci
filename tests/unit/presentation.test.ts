import { describe, expect, it } from "vitest";
import {
  formatAge,
  formatResetCountdown,
  providerRank,
  statusSummary,
  tightestWindow,
} from "../../src/shared/presentation";
import type { ProviderUsageSnapshot } from "../../src/shared/types";

const snapshot: ProviderUsageSnapshot = {
  provider: "claude",
  displayName: "Claude",
  status: "ok",
  plan: null,
  windows: [
    {
      id: "session",
      label: "5h",
      usedPercent: 20,
      durationMinutes: 300,
      resetsAt: null,
      resetDescription: null,
    },
    {
      id: "weekly",
      label: "Weekly",
      usedPercent: 70,
      durationMinutes: 10_080,
      resetsAt: null,
      resetDescription: null,
    },
  ],
  updatedAt: 0,
  source: "oauth",
  error: null,
};

describe("presentation helpers", () => {
  it("selects the most constrained window", () => {
    expect(tightestWindow(snapshot)?.id).toBe("weekly");
    expect(tightestWindow({ ...snapshot, windows: [] })).toBeNull();
  });

  it("formats one shared countdown", () => {
    expect(formatResetCountdown(0, 0)).toBe("now");
    expect(formatResetCountdown(12 * 60_000, 0)).toBe("12m");
    expect(formatResetCountdown((26 * 60 + 5) * 60_000, 0)).toBe("1d 2h");
  });

  it("reports how old a stale reading is instead of when it resets", () => {
    const stale = { ...snapshot, status: "stale" as const, updatedAt: 0 };
    expect(statusSummary(stale, 7 * 60_000)).toEqual({
      text: "Updated 7m ago",
      stale: true,
    });
    expect(formatAge(0, 0, "just now")).toBe("Updated just now");
  });

  it("prefers the tightest reset while a reading is current", () => {
    const fresh = {
      ...snapshot,
      windows: snapshot.windows.map((window) => ({
        ...window,
        resetsAt: 90 * 60_000,
      })),
    };
    expect(statusSummary(fresh, 0)).toEqual({
      text: "Resets in 1h 30m",
      stale: false,
    });
    expect(statusSummary({ ...snapshot, windows: [] }, 0)).toBeNull();
  });
});

describe("providerRank", () => {
  // The sort the provider list runs: the configured order first, usage breaking the rest.
  const sorted = (order: string[], ids: [string, number][]): string[] =>
    ids
      .sort(
        ([left, usedLeft], [right, usedRight]) =>
          providerRank(order, left) - providerRank(order, right) ||
          usedRight - usedLeft,
      )
      .map(([id]) => id);

  it("leaves the usage ordering alone when no order is set", () => {
    const usage: [string, number][] = [
      ["claude", 10],
      ["codex", 90],
      ["antigravity", 50],
    ];
    expect(sorted([], usage)).toEqual(["codex", "antigravity", "claude"]);
  });

  it("puts the configured order ahead of usage", () => {
    const usage: [string, number][] = [
      ["claude", 10],
      ["codex", 90],
      ["antigravity", 50],
    ];
    expect(sorted(["claude", "antigravity", "codex"], usage)).toEqual([
      "claude",
      "antigravity",
      "codex",
    ]);
  });

  it("appends providers the order does not mention, still by usage", () => {
    expect(providerRank(["claude"], "plugin:new")).toBe(1);
    const usage: [string, number][] = [
      ["claude", 10],
      ["codex", 5],
      ["plugin:new", 80],
    ];
    expect(sorted(["codex"], usage)).toEqual(["codex", "plugin:new", "claude"]);
  });
});
