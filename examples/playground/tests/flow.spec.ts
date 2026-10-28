import { expect, test, type Page } from "@playwright/test";
import { PNG } from "pngjs";

// Animated edges (`flow`): marks move along them, one frame per animation frame while the
// canvas is on screen, and never for a viewer who asks for reduced motion.

const URL = "/?backend=webgl2&ui=0&style=glossy&flow=1&example=mermaid/01-decision";

async function ready(page: Page, url = URL) {
  await page.goto(url);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
}

const renderCount = (page: Page) => page.evaluate(() => (window as any).facetryChart.renderCount as number);

/**
 * Waits until no frame is drawn for 400 ms in a row: frames that were already on their way
 * (late, under load) may still come first. A chart that keeps drawing never gets there and fails.
 */
async function settles(page: Page) {
  await expect
    .poll(
      async () => {
        const before = await renderCount(page);
        await page.waitForTimeout(400);
        return (await renderCount(page)) - before;
      },
      { timeout: 10_000 },
    )
    .toBe(0);
}

async function shot(page: Page): Promise<PNG> {
  return PNG.sync.read(await page.locator("#chart").screenshot());
}

/** How many pixels differ between two pictures of the same size. */
function changed(a: PNG, b: PNG): number {
  let n = 0;
  for (let i = 0; i < a.data.length; i += 4) {
    if (Math.abs(a.data[i]! - b.data[i]!) + Math.abs(a.data[i + 1]! - b.data[i + 1]!) > 30) n++;
  }
  return n;
}

test.describe("with motion", () => {
  test.use({ reducedMotion: "no-preference" });

  test("the marks keep moving, frame after frame, until flow is switched off", async ({ page }) => {
    await ready(page);
    // Software rendering under parallel tests is slow: wait for frames rather than a time.
    const before = await renderCount(page);
    await expect.poll(() => renderCount(page), { timeout: 10_000 }).toBeGreaterThan(before + 3);
    expect(await page.evaluate(() => (window as any).facetryChart.flow)).toBe(true);
    // Switched off: the frames stop, and the marks stay where they were.
    await page.evaluate(() => ((window as any).facetryChart.flow = false));
    await settles(page);
    const held = await renderCount(page);
    // And on again.
    await page.evaluate(() => ((window as any).facetryChart.flow = true));
    await expect.poll(() => renderCount(page), { timeout: 10_000 }).toBeGreaterThan(held + 3);
  });

  test("the marks move the way the edges run", async ({ page }) => {
    await ready(page);
    const at = (ms: number) => page.evaluate((ms) => (window as any).facetryChart.__animationClock(ms), ms);
    await at(0);
    const a = await shot(page);
    await at(200);
    const b = await shot(page);
    await at(0);
    const again = await shot(page);
    expect(changed(a, b)).toBeGreaterThan(50);
    // The same time draws the same picture.
    expect(changed(a, again)).toBe(0);
  });

  test("a diagram without animated edges draws no frames of its own", async ({ page }) => {
    await ready(page, URL.replace("&flow=1", ""));
    await settles(page);
  });
});

test("a viewer who asks for reduced motion sees still marks and no frames", async ({ page }) => {
  // The project runs tests with `reducedMotion: "reduce"`.
  await ready(page);
  await settles(page);
});

// The glossy pulses and the schematic flow marks, a quarter second in, in the golden images'
// fixed viewport (as in golden.spec.ts).
test.describe("golden images", () => {
  test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1 });
  for (const style of ["glossy", "schematic"]) {
    test(`${style} diagram 01-decision with flowing edges matches its golden image (webgl2)`, async ({ page }) => {
      await ready(page, URL.replace("style=glossy", `style=${style}`));
      await page.evaluate(() => (window as any).facetryChart.__animationClock(250));
      await page.mouse.move(790, 495);
      await expect(page.locator("#chart")).toHaveScreenshot(`diagram-${style}-flow-01-decision.png`);
    });
  }
});
