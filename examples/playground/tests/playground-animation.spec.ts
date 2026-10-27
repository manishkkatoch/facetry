import { expect, test, type Page } from "@playwright/test";

// The playground's animation controls: "Randomize data" and the "Animate" checkbox.

test.use({ viewport: { width: 1200, height: 700 }, deviceScaleFactor: 1, reducedMotion: "no-preference" });

async function ready(page: Page, query = "") {
  await page.goto(`/?backend=webgl2&clock=manual${query}`);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
}

const animating = (page: Page) => page.evaluate(() => (window as any).facetryChart.animating as boolean);
const spec = async (page: Page) => JSON.parse(await page.locator("#json").inputValue());

test("Randomize data changes the numbers and the chart animates to them", async ({ page }) => {
  await ready(page, "&example=bar-stacked&seed=7");
  const before = await spec(page);
  await page.locator("#randomize").click();
  await expect(page.locator("html")).toHaveAttribute("data-updates", "1");
  const after = await spec(page);
  expect(after.data.sales.revenue).not.toEqual(before.data.sales.revenue);
  expect(await animating(page)).toBe(true);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(10_000));
  expect(await animating(page)).toBe(false);
});

test("with Animate off an update jumps", async ({ page }) => {
  await ready(page, "&example=bar-stacked&seed=7");
  await page.locator("#animate").uncheck();
  await page.locator("#randomize").click();
  await expect(page.locator("html")).toHaveAttribute("data-updates", "1");
  expect(await animating(page)).toBe(false);
  await page.locator("#animate").check();
  await page.locator("#randomize").click();
  await expect(page.locator("html")).toHaveAttribute("data-updates", "2");
  expect(await animating(page)).toBe(true);
});

test("Randomize data sometimes adds a category and sometimes removes one, in any kind of chart", async ({ page }) => {
  for (const example of ["bar-colored", "pie", "line-basic", "bar3d-stacked", "combo-bar-line"]) {
    await ready(page, `&example=${example}&seed=3`);
    const categories = async () => {
      const s = await spec(page);
      const layer = s.layers[0];
      const field = layer.mark === "arc" ? layer.encoding.color.field : layer.encoding.x.field;
      return new Set<string>(s.data[layer.data][field]).size;
    };
    const sizes = [await categories()];
    for (let i = 1; i <= 12; i++) {
      await page.locator("#randomize").click();
      await expect(page.locator("html")).toHaveAttribute("data-updates", String(i));
      // The chart took the spec (no validation issues) and is in a sound state.
      await expect(page.locator("html")).toHaveAttribute("data-issues", "0");
      sizes.push(await categories());
      await page.evaluate(() => (window as any).facetryChart.__animationClock(10_000));
    }
    expect(new Set(sizes).size, `${example}: ${sizes}`).toBeGreaterThan(1);
    expect(sizes.some((n, i) => i > 0 && n > sizes[i - 1]!), `${example} added: ${sizes}`).toBe(true);
    expect(sizes.some((n, i) => i > 0 && n < sizes[i - 1]!), `${example} removed: ${sizes}`).toBe(true);
  }
});

test("Spin turns a 3D chart a quarter turn as a tween, and is only there for 3D", async ({ page }) => {
  await ready(page, "&example=bar-basic");
  await expect(page.locator("#spin")).toBeHidden();
  await ready(page, "&example=bar3d-grid");
  await expect(page.locator("#spin")).toBeVisible();
  const azimuth = () => page.evaluate(() => (window as any).facetryChart.camera.azimuth as number);
  const start = await azimuth();
  await page.locator("#spin").click();
  expect(await animating(page)).toBe(true);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(10_000));
  expect(await azimuth()).toBeCloseTo((start + 90) % 360, 3);
});
