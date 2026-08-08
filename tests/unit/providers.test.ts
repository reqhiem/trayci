import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { describe, expect, it } from "vitest";
import {
  normalizeClaudeUsage,
  parseClaudeUsage,
} from "../../src/main/providers/claude";
import {
  normalizeCodexRateLimits,
  parseCodexUsage,
} from "../../src/main/providers/codex";
import {
  abortAllChildren,
  activeChildCount,
  stripTerminalCodes,
  trackChild,
} from "../../src/main/providers/common";
import { vi } from "vitest";
import {
  AntigravityProvider,
  normalizeAntigravityQuota,
  parseAntigravityUsage,
} from "../../src/main/providers/antigravity";
import { DEFAULT_SETTINGS } from "../../src/shared/types";

const now = Date.parse("2026-08-08T12:00:00Z");

describe("Claude usage", () => {
  it("normalizes direct OAuth windows", () => {
    const windows = normalizeClaudeUsage({
      five_hour: { utilization: 0.28, resets_at: "2026-08-08T14:00:00Z" },
      seven_day: { utilization: 41, resets_at: null },
      seven_day_fable: { utilization: 0.2, resets_at: null },
      extra_usage: { utilization: 99 },
    });
    expect(windows.map(({ id, usedPercent }) => [id, usedPercent])).toEqual([
      ["session", 28],
      ["weekly", 41],
      ["fable-weekly", 20],
    ]);
    expect(windows[0]?.resetsAt).toBe(Date.parse("2026-08-08T14:00:00Z"));
  });

  it("parses PTY variants and remaining percentages", async () => {
    const output = await readFile(
      join(process.cwd(), "tests/fixtures/claude/usage-standard.txt"),
      "utf8",
    );
    const windows = parseClaudeUsage(output, now);
    expect(windows.map(({ id, usedPercent }) => [id, usedPercent])).toEqual([
      ["session", 62],
      ["weekly", 31],
      ["fable-weekly", 20],
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
          primary: {
            usedPercent: 10,
            windowDurationMins: 299,
            resetsAt: 1_786_200_000,
          },
          secondary: {
            usedPercent: 28,
            windowDurationMins: 10_079,
            resetsAt: 1_786_500_000,
          },
        },
      },
    });
    expect(normalized.plan).toBe("plus");
    expect(
      normalized.windows.map(({ id, label, usedPercent }) => [
        id,
        label,
        usedPercent,
      ]),
    ).toEqual([
      ["session", "5h", 10],
      ["weekly", "Weekly", 28],
    ]);
    expect(normalized.windows[0]?.resetsAt).toBe(1_786_200_000_000);
  });

  it("parses PTY session and weekly windows", async () => {
    const output = await readFile(
      join(process.cwd(), "tests/fixtures/codex/status-standard.txt"),
      "utf8",
    );
    expect(
      parseCodexUsage(output, now).map(({ id, usedPercent }) => [
        id,
        usedPercent,
      ]),
    ).toEqual([
      ["session", 7],
      ["weekly", 12],
    ]);
  });
});

describe("Antigravity usage", () => {
  it("parses the signed-in agy quota panel", () => {
    const parsed = parseAntigravityUsage(
      `Models & Quota
       GEMINI MODELS
       Weekly Limit Remaining 75.00% Resets in 4d 2h
       Five Hour Limit Remaining 90.00% Resets in 2h 30m
       CLAUDE AND GPT MODELS
       Weekly Limit Remaining 60.00% Resets in 3d
       Five Hour Limit Remaining 100.00% Quota available
       G (Google AI Pro)`,
      now,
    );
    expect(parsed.plan).toBe("Google AI Pro");
    expect(
      parsed.windows.map(({ id, usedPercent }) => [id, usedPercent]),
    ).toEqual([
      ["gemini-session", 10],
      ["gemini-weekly", 25],
      ["claude-gpt-session", 0],
      ["claude-gpt-weekly", 40],
    ]);
    expect(parsed.windows[0]?.resetsAt).toBe(now + 150 * 60_000);
  });

  it("normalizes remaining quota and deduplicates equivalent buckets", () => {
    const windows = normalizeAntigravityQuota({
      buckets: [
        {
          remainingFraction: 0.75,
          resetTime: "2026-08-08T13:00:00Z",
          modelId: "gemini-2.5-pro-preview",
        },
        {
          remainingFraction: 0.75,
          resetTime: "2026-08-08T13:00:00Z",
          modelId: "gemini-2.5-pro",
        },
        {
          remainingFraction: 0.1,
          resetTime: "2026-08-08T14:00:00Z",
          modelId: "gemini-2.5-flash",
        },
        { remainingFraction: Number.NaN, resetTime: "later", modelId: "bad" },
      ],
    });
    expect(
      windows.map(({ label, usedPercent }) => [label, usedPercent]),
    ).toEqual([
      ["Pro", 25],
      ["Flash", 90],
    ]);
    expect(windows[0]?.resetsAt).toBe(Date.parse("2026-08-08T13:00:00Z"));
  });

  it("rejects payloads without valid quota buckets", () => {
    expect(
      normalizeAntigravityQuota({ buckets: [{ modelId: "gemini" }] }),
    ).toEqual([]);
  });

  it("fetches Code Assist quota with the existing OAuth session", async () => {
    const home = await mkdtemp(join(tmpdir(), "trayci-antigravity-"));
    const previousHome = process.env.GEMINI_CLI_HOME;
    process.env.GEMINI_CLI_HOME = home;
    await writeFile(
      join(home, "oauth_creds.json"),
      JSON.stringify({
        access_token: "access-token",
        refresh_token: "refresh-token",
        expiry_date: now + 60_000,
      }),
    );
    const fetchMock = vi
      .spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify({ cloudaicompanionProject: "project-123" }),
          { status: 200 },
        ),
      )
      .mockResolvedValueOnce(
        new Response(
          JSON.stringify([
            {
              remainingFraction: 0.4,
              resetTime: "2026-08-08T14:00:00Z",
              modelId: "gemini-3.1-pro",
            },
          ]),
          { status: 200 },
        ),
      );
    try {
      const provider = new AntigravityProvider(
        () => DEFAULT_SETTINGS,
        async () => null,
      );
      expect((await provider.detect()).status).toBe("available");
      const snapshot = await provider.fetchUsage({
        signal: new AbortController().signal,
        reason: "manual",
        now,
      });
      expect(snapshot.windows[0]).toMatchObject({
        label: "3.1 Pro",
        usedPercent: 60,
      });
      expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([
        "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist",
        "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota",
      ]);
    } finally {
      fetchMock.mockRestore();
      abortAllChildren();
      if (previousHome === undefined) delete process.env.GEMINI_CLI_HOME;
      else process.env.GEMINI_CLI_HOME = previousHome;
      await rm(home, { recursive: true, force: true });
    }
  });
});

it("removes ANSI, OSC and control sequences", () => {
  expect(
    stripTerminalCodes(
      "\u001b[31mUsage\u001b[0m\u001b]0;title\u0007\r42%\u0000",
    ),
  ).toBe("Usage\n42%");
});

it("cleans every tracked provider child", () => {
  const kill = vi.fn();
  trackChild({ kill });
  expect(activeChildCount()).toBe(1);
  abortAllChildren();
  expect(kill).toHaveBeenCalledOnce();
  expect(activeChildCount()).toBe(0);
});
