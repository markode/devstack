import { expect, test } from "@playwright/test";

test.describe("Sites v0.2.9 layout", () => {
  test("nav lists mail, dumps, backups and sharing", async ({ page }) => {
    await page.setViewportSize({ width: 1100, height: 720 });
    await page.goto("/e2e/fixtures/shell-sites.html");
    for (const id of ["sites", "mail", "dumps", "backups", "sharing"]) {
      await expect(page.locator(`[data-nav="${id}"]`)).toBeVisible();
    }
    await expect(page.locator(".nav-item.muted")).toHaveCount(0);
  });

  test("stopped banner is large, running banner is compact", async ({ page }) => {
    await page.setViewportSize({ width: 1100, height: 720 });
    await page.goto("/e2e/fixtures/shell-sites.html");
    const stopped = await page.getByTestId("stack-banner").boundingBox();
    const running = await page.getByTestId("stack-banner-running").boundingBox();
    expect(stopped!.height).toBeGreaterThan(running!.height + 20);
    expect(running!.height).toBeLessThan(40);
    await expect(page.locator(".hero-card")).toHaveCount(0);
  });

  test("tile shows port URL and quick actions without overflow", async ({ page }) => {
    await page.setViewportSize({ width: 900, height: 720 });
    await page.goto("/e2e/fixtures/shell-sites.html");
    const tile = page.getByTestId("site-tile");
    await expect(page.getByTestId("site-tile-url")).toHaveText("blog.test:8443");
    for (const name of ["Open", "Folder", "Terminal", "Database", "Logs"]) {
      await expect(tile.getByRole("button", { name, exact: true })).toBeVisible();
    }
    const overflow = await tile.evaluate((el) => el.scrollWidth > el.clientWidth);
    expect(overflow).toBe(false);
  });
});
