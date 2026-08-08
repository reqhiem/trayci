import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { normalizeClaudeUsage, parseClaudeUsage } from "../../src/main/providers/claude";
import { normalizeCodexRateLimits, parseCodexUsage } from "../../src/main/providers/codex";
import {
  abortAllChildren,
  activeChildCount,
  stripTerminalCodes,
  trackChild
} from "../../src/main/providers/common";
import { vi } from "vitest";

const now = Date.parse("2026-08-08T12:00:00Z");

describe("Claude usage", () => {
  it("normalizes direct OAuth windows", () => {
    const windows = normalizeClaudeUsage({
      five_hour: { utilization: 0.28, resets_at: "2026-08-08T14:00:00Z" },
      seven_day: { utilization: 41, resets_at: null },
      seven_day_fable: { utilization: 0.2, resets_at: null },
      extra_usage: { utilization: 99 }
    });
    expect(windows.map(({ id, usedPercent }) => [id, usedPercent])).toEqual([
      ["session", 28],
      ["weekly", 41],
      ["fable-weekly", 20]
    ]);
    expect(windows[0]?.resetsAt).toBe(Date.parse("2026-08-08T14:00:00Z"));
  });

  it("parses PTY variants and remaining percentages", async () => {
    const output = await readFile(join(process.cwd(), "tests/fixtures/claude/usage-standard.txt"), "utf8");
    const windows = parseClaudeUsage(output, now);
    expect(windows.map(({ id, usedPercent }) => [id, usedPercent])).toEqual([
      ["session", 62],
      ["weekly", 31],
      ["fable-weekly", 20]
    ]);
    expect(windows[0]?.resetsAt).toBe(now + 135 * 60_000);
  });
});

describe("Codex usage", () => {
  it("prefers keyed rate limits, classifies duration and converts seconds", () => {
    const normalized = normalizeCodexRateLimits({
      rateLimits: { primary: { usedPercent: 99, windowDurationMins: 300 } },
      rateLimitsByLimitId: {
        codex: {
          planType: "plus",
          primary: { usedPercent: 10, windowDurationMins: 299, resetsAt: 1_786_200_000 },
          secondary: { usedPercent: 28, windowDurationMins: 10_079, resetsAt: 1_786_500_000 }
        }
      }
    });
    expect(normalized.plan).toBe("plus");
    expect(normalized.windows.map(({ id, label, usedPercent }) => [id, label, usedPercent])).toEqual([
      ["session", "5h", 10],
      ["weekly", "Weekly", 28]
    ]);
    expect(normalized.windows[0]?.resetsAt).toBe(1_786_200_000_000);
  });

  it("parses PTY session and weekly windows", async () => {
    const output = await readFile(join(process.cwd(), "tests/fixtures/codex/status-standard.txt"), "utf8");
    expect(parseCodexUsage(output, now).map(({ id, usedPercent }) => [id, usedPercent])).toEqual([
      ["session", 7],
      ["weekly", 12]
    ]);
  });
});

it("removes ANSI, OSC and control sequences", () => {
  expect(stripTerminalCodes("\u001b[31mUsage\u001b[0m\u001b]0;title\u0007\r42%\u0000")).toBe("Usage\n42%");
});

it("cleans every tracked provider child", () => {
  const kill = vi.fn();
  trackChild({ kill });
  expect(activeChildCount()).toBe(1);
  abortAllChildren();
  expect(kill).toHaveBeenCalledOnce();
  expect(activeChildCount()).toBe(0);
});
