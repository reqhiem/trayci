import { describe, expect, it } from "vitest";
import {
  calculatePopoverPosition,
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

  it("keeps the popover inside the work area", () => {
    expect(
      calculatePopoverPosition(
        { x: 1900, y: 0, width: 20, height: 24 },
        { x: 0, y: 0, width: 1920, height: 1080 },
        { width: 390, height: 560 },
      ),
    ).toEqual({ x: 1530, y: 32 });
    expect(
      calculatePopoverPosition(
        { x: 0, y: 1050, width: 20, height: 30 },
        { x: 0, y: 0, width: 1920, height: 1080 },
        { width: 390, height: 560 },
      ),
    ).toEqual({ x: 0, y: 482 });
  });
});
