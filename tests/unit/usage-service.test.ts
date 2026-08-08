import { expect, it } from "vitest";
import type {
  ProviderUsageSnapshot,
  UsageProvider,
} from "../../src/shared/types";
import { DEFAULT_SETTINGS } from "../../src/shared/types";
import { ProviderError } from "../../src/main/providers/common";
import { UsageService } from "../../src/main/usage-service";

const cache = {
  load: async () => ({}),
  save: async () => undefined,
};

it("preserves the last successful snapshot after a temporary failure", async () => {
  let fail = false;
  const provider: UsageProvider = {
    id: "claude",
    displayName: "Claude",
    detect: async () => ({
      provider: "claude",
      status: "available",
      executablePath: "/fake",
    }),
    fetchUsage: async ({ now }): Promise<ProviderUsageSnapshot> => {
      if (fail) throw new ProviderError("network", "offline");
      return {
        provider: "claude",
        displayName: "Claude",
        status: "ok",
        plan: null,
        windows: [
          {
            id: "session",
            label: "5h",
            usedPercent: 42,
            durationMinutes: 300,
            resetsAt: null,
            resetDescription: null,
          },
        ],
        updatedAt: now,
        source: "oauth",
        error: null,
      };
    },
  };
  const service = new UsageService([provider], () => DEFAULT_SETTINGS, cache);
  await service.start();
  fail = true;
  const state = await service.refreshAll("manual");
  expect(state.providers.claude?.status).toBe("stale");
  expect(state.providers.claude?.windows[0]?.usedPercent).toBe(42);
  await service.stop();
});
