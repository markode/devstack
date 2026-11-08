import { expect, test } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const shot = (name: string) =>
  path.join(here, "screenshots", `${name}.png`);

const VIEWPORTS = [
  { name: "desktop", width: 1280, height: 800 },
  { name: "tablet", width: 768, height: 1024 },
  { name: "mobile", width: 390, height: 844 },
] as const;

test.describe("Herd-style header + Sites toolbar", () => {
  for (const vp of VIEWPORTS) {
    test(`renders without overflow at ${vp.name} (${vp.width}px)`, async ({
      page,
    }) => {
      await page.setViewportSize({ width: vp.width, height: vp.height });
      await page.goto("/e2e/fixtures/header-shell.html");

      const header = page.getByTestId("page-header");
      await expect(header).toBeVisible();
      await expect(page.getByTestId("page-header-title")).toHaveText("Sites");
      await expect(page.getByTestId("sites-toolbar")).toBeVisible();
      await expect(page.getByTestId("stack-status-pill")).toContainText(
        "Stopped",
      );

      // No horizontal page scroll from toolbar wrapping.
      const scrollWidth = await page.evaluate(
        () => document.documentElement.scrollWidth,
      );
      const clientWidth = await page.evaluate(
        () => document.documentElement.clientWidth,
      );
      expect(scrollWidth).toBeLessThanOrEqual(clientWidth + 1);

      await page.screenshot({
        path: shot(`header-${vp.name}`),
        fullPage: true,
      });
    });
  }

  test("header loader bar is present while refreshing", async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 });
    await page.goto("/e2e/fixtures/header-shell.html");
    await expect(page.getByTestId("sites-header-loader")).toBeVisible();
  });
});
