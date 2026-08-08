import { _electron as electron, expect, test } from "@playwright/test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

test("renders usage, switches density and persists settings", async () => {
  const config = await mkdtemp(join(tmpdir(), "trayci-e2e-"));
  const application = await electron.launch({
    args: ["."],
    env: { ...process.env, TRAYCI_E2E: "1", XDG_CONFIG_HOME: config },
  });
  try {
    const window = await application.firstWindow();
    await expect(window.getByRole("heading", { name: "Usage" })).toBeVisible();
    await expect(window.getByRole("button", { name: /Claude/ })).toBeVisible();
    await expect(window.getByRole("button", { name: /Codex/ })).toBeVisible();
    await expect(
      window.getByRole("button", { name: /Antigravity/ }),
    ).toBeVisible();
    await expect(window.getByText("Gem 5h")).toBeVisible();
    await expect(window.getByText("C/G wk")).toBeVisible();
    expect(
      await window
        .locator(".inline-metrics-grid")
        .evaluate((element) => element.scrollWidth <= element.clientWidth),
    ).toBe(true);
    await expect(window.getByText("wk").first()).toBeVisible();
    await expect
      .poll(() =>
        application.evaluate(
          ({ BrowserWindow }) =>
            BrowserWindow.getAllWindows()[0]?.getContentSize()[1],
        ),
      )
      .toBeGreaterThan(190);
    expect(
      await application.evaluate(
        ({ BrowserWindow }) =>
          BrowserWindow.getAllWindows()[0]?.getContentSize()[1],
      ),
    ).toBeLessThan(400);
    await window.getByRole("button", { name: /Claude/ }).click();
    await expect(window.getByLabel("Claude details")).toBeVisible();
    await expect
      .poll(() =>
        application.evaluate(
          ({ BrowserWindow }) =>
            BrowserWindow.getAllWindows()[0]?.getContentSize()[0],
        ),
      )
      .toBeGreaterThan(670);
    expect(
      await application.evaluate(
        ({ BrowserWindow }) =>
          BrowserWindow.getAllWindows()[0]?.getContentSize()[0],
      ),
    ).toBeLessThan(690);
    await window.getByRole("button", { name: "Compact" }).click();
    await expect(window.getByLabel("Claude details")).toHaveCount(0);
    await expect
      .poll(() =>
        application.evaluate(
          ({ BrowserWindow }) =>
            BrowserWindow.getAllWindows()[0]?.getContentSize()[0],
        ),
      )
      .toBeLessThan(370);
    expect(
      await application.evaluate(
        ({ BrowserWindow }) =>
          BrowserWindow.getAllWindows()[0]?.getContentSize()[0],
      ),
    ).toBeGreaterThan(350);
    await window.getByRole("button", { name: /Codex/ }).click();
    await expect(window.getByLabel("Codex details")).toBeVisible();
    await expect
      .poll(() =>
        application.evaluate(
          ({ BrowserWindow }) =>
            BrowserWindow.getAllWindows()[0]?.getContentSize()[0],
        ),
      )
      .toBeGreaterThan(670);
    const beforeReopen = await application.evaluate(({ BrowserWindow }) =>
      BrowserWindow.getAllWindows()[0]?.getBounds(),
    );
    await window.evaluate(() =>
      (
        globalThis as unknown as {
          trayci: { app: { hidePopover(): Promise<void> } };
        }
      ).trayci.app.hidePopover(),
    );
    await application.evaluate(({ app }) =>
      app.emit("second-instance", {} as Electron.Event, [], process.cwd(), {}),
    );
    await expect
      .poll(() =>
        application.evaluate(({ BrowserWindow }) =>
          BrowserWindow.getAllWindows()[0]?.isVisible(),
        ),
      )
      .toBe(true);
    expect(
      await application.evaluate(({ BrowserWindow }) =>
        BrowserWindow.getAllWindows()[0]?.getBounds(),
      ),
    ).toEqual(beforeReopen);
    await window.getByRole("button", { name: "Settings" }).click();
    await expect(
      window.getByRole("switch", { name: "Antigravity" }),
    ).toBeChecked();
    await window.getByRole("switch", { name: "Start Trayci on login" }).check();
    await expect(
      window.getByRole("switch", { name: "Start Trayci on login" }),
    ).toBeChecked();
  } finally {
    await application.close();
    await rm(config, { recursive: true, force: true });
  }
});

test("supports the critical keyboard flow", async () => {
  const config = await mkdtemp(join(tmpdir(), "trayci-e2e-"));
  const application = await electron.launch({
    args: ["."],
    env: { ...process.env, TRAYCI_E2E: "1", XDG_CONFIG_HOME: config },
  });
  try {
    const window = await application.firstWindow();
    await expect(window.getByRole("heading", { name: "Usage" })).toBeVisible();

    const refresh = window.getByRole("button", { name: "Refresh usage" });
    const settingsButton = window.getByRole("button", { name: "Settings" });
    const detailed = window.getByRole("button", { name: "Detailed" });
    const compact = window.getByRole("button", { name: "Compact" });
    const claude = window.getByRole("button", { name: /Claude/ });
    const antigravity = window.getByRole("button", { name: /Antigravity/ });
    const codex = window.getByRole("button", { name: /Codex/ });

    await window.keyboard.press("Tab");
    await expect(refresh).toBeFocused();
    await window.keyboard.press("Tab");
    await expect(settingsButton).toBeFocused();
    await window.keyboard.press("Tab");
    await expect(detailed).toBeFocused();
    await expect(detailed).toHaveAttribute("aria-pressed", "true");
    await window.keyboard.press("Tab");
    await expect(compact).toBeFocused();
    await window.keyboard.press("Tab");
    await expect(claude).toBeFocused();
    await window.keyboard.press("Tab");
    await expect(antigravity).toBeFocused();
    await window.keyboard.press("Tab");
    await expect(codex).toBeFocused();

    await expect(
      window.getByRole("progressbar", { name: "5h usage" }).first(),
    ).toHaveAttribute("aria-valuenow", "28");

    await window.keyboard.press("Shift+Tab");
    await expect(antigravity).toBeFocused();
    await window.keyboard.press("Shift+Tab");
    await expect(claude).toBeFocused();
    await window.keyboard.press("Enter");
    await expect(claude).toHaveAttribute("aria-expanded", "true");
    await expect(window.getByLabel("Claude details")).toBeVisible();
    await window.keyboard.press("Escape");
    await expect(claude).toHaveAttribute("aria-expanded", "false");
    await expect(claude).toBeFocused();
    await expect(window.getByLabel("Claude details")).toHaveCount(0);
    await expect
      .poll(() =>
        application.evaluate(({ BrowserWindow }) =>
          BrowserWindow.getAllWindows()[0]?.isVisible(),
        ),
      )
      .toBe(true);

    await settingsButton.click();
    const backButton = window.locator(".back-button");
    await expect(backButton).toBeFocused();
    await window.keyboard.press("Tab");
    await expect(window.locator(".settings-button")).toBeFocused();
    const startOnLogin = window.getByRole("switch", {
      name: "Start Trayci on login",
    });
    await window.keyboard.press("Tab");
    await expect(startOnLogin).toBeFocused();
    await window.keyboard.press("Space");
    await expect(startOnLogin).toBeChecked();
    await window.keyboard.press("Shift+Tab");
    await window.keyboard.press("Shift+Tab");
    await expect(backButton).toBeFocused();
    await window.keyboard.press("Enter");
    await expect(window.getByRole("heading", { name: "Usage" })).toBeVisible();
    await expect(settingsButton).toBeFocused();
    await window.keyboard.press("Escape");
    await expect
      .poll(() =>
        application.evaluate(({ BrowserWindow }) =>
          BrowserWindow.getAllWindows()[0]?.isVisible(),
        ),
      )
      .toBe(false);
  } finally {
    await application.close();
    await rm(config, { recursive: true, force: true });
  }
});
