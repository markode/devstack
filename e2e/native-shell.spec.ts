import { expect, test } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const shot = (name: string) => path.join(here, "screenshots", `${name}.png`);

test.describe("Native desktop shell", () => {
  test("Windows: edge-to-edge shell + caption buttons", async ({ page }) => {
    await page.setViewportSize({ width: 1100, height: 720 });
    await page.goto("/e2e/fixtures/header-shell.html");

    const shell = page.getByTestId("app-shell");
    await expect(shell).toHaveAttribute("data-platform", "windows");
    await expect(page.getByTestId("window-controls")).toBeVisible();
    await expect(page.getByTestId("page-header")).toHaveAttribute(
      "data-tauri-drag-region",
      "",
    );

    // Edge-to-edge: no outer page margins / max-width card.
    const metrics = await page.evaluate(() => {
      const shellEl = document.querySelector(".app-shell") as HTMLElement;
      const body = document.body;
      const cs = getComputedStyle(shellEl);
      return {
        bodyPad: getComputedStyle(body).padding,
        maxWidth: cs.maxWidth,
        borderRadius: cs.borderRadius,
        shellWidth: shellEl.getBoundingClientRect().width,
        viewport: window.innerWidth,
      };
    });
    expect(metrics.bodyPad).toBe("0px");
    expect(metrics.maxWidth === "none" || metrics.maxWidth === "0px").toBeTruthy();
    expect(metrics.borderRadius === "0px" || metrics.borderRadius === "0").toBeTruthy();
    expect(metrics.shellWidth).toBeGreaterThanOrEqual(metrics.viewport - 1);

    await page.screenshot({ path: shot("shell-windows"), fullPage: true });
  });

  test("macOS: traffic-light spacer in sidebar, no caption buttons", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1100, height: 720 });
    await page.goto("/e2e/fixtures/shell-macos.html");

    await expect(page.getByTestId("app-shell")).toHaveAttribute(
      "data-platform",
      "macos",
    );
    await expect(page.getByTestId("sidebar-drag")).toBeVisible();
    await expect(page.locator(".traffic-light-spacer")).toBeVisible();
    await expect(page.getByTestId("window-controls")).toHaveCount(0);

    await page.screenshot({ path: shot("shell-macos"), fullPage: true });
  });

  test("PHP table has no stray status-dot in header", async ({ page }) => {
    await page.setViewportSize({ width: 1100, height: 720 });
    await page.goto("/e2e/fixtures/shell-php.html");

    const table = page.getByTestId("php-version-table");
    await expect(table).toBeVisible();
    await expect(table.locator("thead .status-dot")).toHaveCount(0);
    await expect(table.locator("thead .nav-badge")).toHaveCount(0);
    await expect(page.getByTestId("section-header")).toContainText(
      "Available versions",
    );
    // No hero card remnant.
    await expect(page.locator(".hero-card")).toHaveCount(0);

    await page.screenshot({ path: shot("shell-php-windows"), fullPage: true });
  });
});
