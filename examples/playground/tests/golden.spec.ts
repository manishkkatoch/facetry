import { expect, test } from "@playwright/test";

// A fixed viewport and pixel ratio make the output reproducible. 800x500 is the size the
// Rust scene snapshot (crates/facetry-core/tests/snapshots) is laid out for.
test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1 });

test("basic bar chart matches its golden image (webgl2)", async ({ page }) => {
  await page.goto("/?backend=webgl2&ui=0&example=bar-basic");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("#chart")).toHaveScreenshot("bar-basic.png");
});
