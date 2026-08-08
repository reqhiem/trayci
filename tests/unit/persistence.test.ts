import { describe, expect, it } from "vitest";
import { DEFAULT_SETTINGS } from "../../src/shared/types";
import { mergeSettings } from "../../src/main/persistence";

describe("settings validation", () => {
  it("deep-merges provider changes", () => {
    const settings = mergeSettings(DEFAULT_SETTINGS, {
      providers: { claude: { enabled: false } },
    });
    expect(settings.providers.claude.enabled).toBe(false);
    expect(settings.providers.codex.enabled).toBe(true);
  });

  it("rejects invalid intervals and executable paths", () => {
    expect(() =>
      mergeSettings(DEFAULT_SETTINGS, { pollIntervalMinutes: 1 }),
    ).toThrow();
    expect(() =>
      mergeSettings(DEFAULT_SETTINGS, {
        providers: { codex: { executablePath: "relative/codex" } },
      }),
    ).toThrow();
  });
});
