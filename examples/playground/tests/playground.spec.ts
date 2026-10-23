import { expect, test, type Page } from "@playwright/test";
import { PNG } from "pngjs";

test.use({ viewport: { width: 1200, height: 700 }, deviceScaleFactor: 1 });

const BAR = [0x4c, 0x78, 0xa8];
const ORANGE = [0xf5, 0x85, 0x18]; // palette[1], used by bar-colored.json

async function canvasPng(page: Page) {
  return PNG.sync.read(await page.locator("#chart").screenshot());
}

/** Pixel counts and the rightmost x of pixels close to `want`. */
function stats(png: PNG, want: number[]) {
  let count = 0;
  let rightmost = -1;
  for (let y = 0; y < png.height; y++) {
    for (let x = 0; x < png.width; x++) {
      const i = (y * png.width + x) * 4;
      if (want.every((v, k) => Math.abs(png.data[i + k]! - v) <= 4)) {
        count++;
        rightmost = Math.max(rightmost, x);
      }
    }
  }
  return { count, rightmost };
}

async function ready(page: Page, query = "") {
  await page.goto(`/?backend=webgl2${query}`);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
}

test("the example dropdown swaps the chart", async ({ page }) => {
  await ready(page);
  expect(stats(await canvasPng(page), ORANGE).count).toBe(0);
  await page.locator("#example").selectOption("bar-colored");
  await expect(page.locator("html")).toHaveAttribute("data-updates", "1");
  await expect(page.locator("#json")).toHaveValue(/"Elderberry"/);
  expect(stats(await canvasPng(page), ORANGE).count).toBeGreaterThan(1000);
});

test("Apply shows every validation issue with its path and keeps the old chart", async ({ page }) => {
  await ready(page);
  const spec = JSON.parse(await page.locator("#json").inputValue());
  spec.layers[0].encoding.y.axis = "nope";
  spec.layers[0].data = "missing";
  await page.locator("#json").fill(JSON.stringify(spec));
  await page.locator("#apply").click();

  const items = page.locator("#issues li");
  await expect(items).toHaveCount(2);
  await expect(items.nth(0)).toContainText("/layers/0/encoding/y/axis");
  await expect(page.locator("#issues")).toContainText("/layers/0/data");
  await expect(page.locator("html")).toHaveAttribute("data-issues", "2");
  expect(stats(await canvasPng(page), BAR).count).toBeGreaterThan(50_000);

  // Fixing the spec clears the list and redraws.
  spec.layers[0].encoding.y.axis = "y";
  spec.layers[0].data = "sales";
  spec.data.sales.revenue = [10, 20, 30, 40];
  await page.locator("#json").fill(JSON.stringify(spec));
  await page.locator("#apply").click();
  await expect(items).toHaveCount(0);
  await expect(page.locator("html")).toHaveAttribute("data-updates", "1");
});

test("Apply reports JSON syntax errors", async ({ page }) => {
  await ready(page);
  await page.locator("#json").fill("{ not json");
  await page.locator("#apply").click();
  await expect(page.locator("#issues li")).toContainText("not valid JSON");
});

test("resizing re-runs layout, not just the surface", async ({ page }) => {
  await ready(page, "&ui=0");
  await page.setViewportSize({ width: 500, height: 400 });
  // After a relayout the plot ends 12 px (padding) before the right edge; with only the
  // surface resized, the 800-px-wide layout would be cropped and bars would run to the edge.
  await expect
    .poll(async () => stats(await canvasPng(page), BAR).rightmost, { timeout: 5000 })
    .toBeLessThan(500 - 12);
  expect(stats(await canvasPng(page), BAR).count).toBeGreaterThan(10_000);
});
