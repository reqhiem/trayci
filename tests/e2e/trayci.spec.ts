import { _electron as electron, expect, test } from "@playwright/test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

test("renders usage, switches density and persists settings", async () => {
  const config = await mkdtemp(join(tmpdir(), "trayci-e2e-"));
  const application = await electron.launch({
    args: ["."],
    env: { ...process.env, TRAYCI_E2E: "1", XDG_CONFIG_HOME: config }
  });
  try {
    const window = await application.firstWindow();
    await expect(window.getByRole("heading", { name: "Trayci" })).toBeVisible();
    await expect(window.getByRole("heading", { name: "Claude" })).toBeVisible();
    await expect(window.getByText("Weekly")).toBeVisible();
    await window.getByRole("button", { name: "Compact" }).click();
    await expect(window.getByText("5h")).toHaveCount(0);
    await window.getByRole("button", { name: "Settings" }).click();
    await window.getByRole("switch", { name: "Start Trayci on login" }).check();
    await expect(window.getByRole("switch", { name: "Start Trayci on login" })).toBeChecked();
  } finally {
    await application.close();
    await rm(config, { recursive: true, force: true });
  }
});
