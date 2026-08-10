import { describe, expect, it } from "vitest";
import {
  formatResetCountdown,
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
});
