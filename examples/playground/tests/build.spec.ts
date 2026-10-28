import { expect, test } from "@playwright/test";

// A diagram drawn with `animateIn` builds itself in: group bases first, then the nodes rise
// rank by rank, each edge drawing out toward the node it leads to.

const URL = "/?backend=webgl2&ui=0&style=glossy&animateIn=1&example=mermaid/34-group-edges";

test.describe("with motion", () => {
  test.use({ reducedMotion: "no-preference" });

  test("the build-in plays once and the chart then rests", async ({ page }) => {
    // The clock is held at the start, so the build-in cannot be over before it is looked at
    // (a slow page can take longer to get ready than the build-in takes)...
    await page.goto(`${URL}&clock=manual`);
    await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
    expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(true);
    // ...then let go: it plays to its end on the real clock.
    await page.evaluate(() => (window as any).facetryChart.__animationClock(null));
    await expect
      .poll(() => page.evaluate(() => (window as any).facetryChart.animating), { timeout: 15_000 })
      .toBe(false);
    // At rest: no more frames.
    const count = () => page.evaluate(() => (window as any).facetryChart.renderCount as number);
    const rested = await count();
    await page.waitForTimeout(400);
    expect(await count()).toBe(rested);
    // And the nodes are where they belong, hoverable.
    const at = await page.evaluate(() => (window as any).facetryChart.markPosition("package"));
    expect(at).toBeTruthy();
  });

  test.describe("golden image", () => {
    test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1 });
    test("glossy diagram 34-group-edges part way through its build-in matches its golden image (webgl2)", async ({ page }) => {
      await page.goto(`${URL}&clock=manual`);
      await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
      await page.evaluate(() => (window as any).facetryChart.__animationClock(900));
      await page.mouse.move(790, 495);
      await expect(page.locator("#chart")).toHaveScreenshot("diagram-glossy-build-34-group-edges.png");
    });
  });
});

test("a viewer who asks for reduced motion gets the whole diagram at once", async ({ page }) => {
  // The project runs tests with `reducedMotion: "reduce"`.
  await page.goto(URL);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(false);
});
