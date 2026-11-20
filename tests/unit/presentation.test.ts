import { describe, expect, it } from "vitest";
import {
  detailLabel,
  displayPercent,
  formatAge,
  formatResetCountdown,
  poolName,
  providerRank,
  rowLabel,
  severity,
  statusSummary,
  tightestWindow,
} from "../../src/shared/presentation";
import type {
  ProviderUsageSnapshot,
  UsageWindow,
} from "../../src/shared/types";

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
    expect(formatAge(0, 0)).toBe("Updated just now");
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
      text: "1h 30m",
      stale: false,
    });
    expect(statusSummary({ ...snapshot, windows: [] }, 0)).toBeNull();
  });

  it("ages out a reading that loaded as ok but has not been refreshed since", () => {
    // The bug in #54: a snapshot the cache marked `ok` days ago kept rendering a healthy
    // countdown, because only the load-time status field was ever consulted.
    const twoDays = 2 * 24 * 60 * 60_000;
    const aged = {
      ...snapshot,
      windows: snapshot.windows.map((window) => ({
        ...window,
        resetsAt: twoDays + 90 * 60_000,
      })),
    };
    expect(statusSummary(aged, twoDays)).toEqual({
      text: "Updated 2d 0h ago",
      stale: true,
    });
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

// The vocabulary table in issue #66, shared with the TUI (#67) and the Figma library (#68).
describe("vocabulary", () => {
  const gemini = { name: "Gemini Models", models: "Gemini Flash, Gemini Pro" };
  const thirdParty = { name: "Claude and GPT models", models: null };
  const argon = { name: "Gemini 4 Argon", models: null };
  const meter = (
    id: string,
    durationMinutes: number | null,
    group: UsageWindow["group"],
    label: string,
  ): UsageWindow => ({
    id,
    label,
    usedPercent: 0,
    durationMinutes,
    resetsAt: null,
    resetDescription: null,
    group,
  });
  // Labels as antigravity.rs builds them: the pool's name and window, or the pool alone.
  const antigravity = [
    meter("gemini-5h", 300, gemini, "Gemini 5h"),
    meter("gemini-weekly", 10_080, gemini, "Gemini Weekly"),
    meter("3p-5h", 300, thirdParty, "Claude/GPT 5h"),
    meter("3p-weekly", 10_080, thirdParty, "Claude/GPT Weekly"),
    meter("gemini-argon-wtu-weekly", 10_080, argon, "Gemini 4 Argon"),
  ];
  // Claude's per-model limit shares the weekly length but keeps its own name.
  const claude = [
    meter("session", 300, undefined, "5h"),
    meter("weekly", 10_080, undefined, "Weekly"),
    meter("fable-weekly", 10_080, undefined, "Fable"),
  ];

  it("names pools in plain words", () => {
    expect(poolName("Gemini Models")).toBe("Gemini");
    expect(poolName("Claude and GPT models")).toBe("Claude/GPT");
    expect(poolName("Gemini 4 Argon")).toBe("Gemini 4 Argon");
    expect(poolName("Nova models")).toBe("Nova");
  });

  it("labels row meters by pool and cadence, or by cadence alone", () => {
    expect(antigravity.map(rowLabel)).toEqual([
      "Gemini 5h",
      "Gemini wk",
      "Claude/GPT 5h",
      "Claude/GPT wk",
      "Gemini 4 Argon wk",
    ]);
    expect(claude.map(rowLabel)).toEqual(["5h", "wk", "Fable"]);
  });

  it("labels detail entries with the cadence spelled out", () => {
    expect(antigravity.map(detailLabel)).toEqual([
      "Gemini · 5h",
      "Gemini · Weekly",
      "Claude/GPT · 5h",
      "Claude/GPT · Weekly",
      "Gemini 4 Argon · Weekly",
    ]);
    expect(claude.map(detailLabel)).toEqual(["5h", "Weekly", "Fable"]);
  });

  it("keeps the name of a pooled window of any other length", () => {
    const hourly = meter(
      "nova-1h",
      60,
      { name: "Nova models", models: null },
      "Nova 1h",
    );
    const lone = meter(
      "nova",
      null,
      { name: "Nova models", models: null },
      "Nova",
    );
    expect([rowLabel(hourly), detailLabel(hourly)]).toEqual([
      "Nova 1h",
      "Nova · 1h",
    ]);
    expect([rowLabel(lone), detailLabel(lone)]).toEqual(["Nova", "Nova"]);
  });

  it("counts down to a reset", () => {
    const at = (minutes: number): string =>
      formatResetCountdown(minutes * 60_000, 0);
    expect([
      at(45),
      at(3 * 60 + 54),
      at((5 * 24 + 1) * 60 + 30),
      at(0),
    ]).toEqual(["45m", "3h 54m", "5d 1h", "now"]);
  });

  it("says how old a reading is", () => {
    expect(formatAge(0, 2 * 60_000 + 59_000)).toBe("Updated 2m ago");
    expect(formatAge(0, 30_000)).toBe("Updated just now");
  });

  it("shows an integer percent, used or remaining", () => {
    const window = { ...claude[0]!, usedPercent: 17.4 };
    expect(displayPercent(window, "used")).toBe(17);
    expect(displayPercent(window, "remaining")).toBe(83);
    expect(displayPercent({ ...window, usedPercent: 140 }, "remaining")).toBe(
      0,
    );
  });

  it("colors by use at the notification thresholds", () => {
    expect([0, 49, 50, 84, 85, 89, 90, 100].map(severity)).toEqual([
      "low",
      "low",
      "medium",
      "medium",
      "high",
      "high",
      "critical",
      "critical",
    ]);
  });
});
