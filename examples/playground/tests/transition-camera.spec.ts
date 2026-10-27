import { expect, test, type Page } from "@playwright/test";
import { PNG } from "pngjs";

// Camera tweens (`setCamera(cam, { duration, easing })`, `resetCamera({ duration })`) and the
// animation of legend toggles. Like transition.spec.ts these hold the animation clock with
// `chart.__animationClock(ms)` and run with motion allowed.

test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1, reducedMotion: "no-preference" });
test.setTimeout(90_000);

async function open(page: Page, example: string, query = "") {
  await page.goto(`/?backend=webgl2&ui=0&example=${example}${query}`);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await page.mouse.move(790, 495);
}

const frames = (page: Page) =>
  page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

async function shot(page: Page) {
  await frames(page);
  return PNG.sync.read(await page.locator("#chart").screenshot());
}

async function at(page: Page, ms: number) {
  await page.evaluate((ms) => (window as any).facetryChart.__animationClock(ms), ms);
  return shot(page);
}

function differing(a: PNG, b: PNG, tolerance = 8) {
  let n = 0;
  for (let i = 0; i < a.data.length; i += 4) {
    const d = Math.max(
      Math.abs(a.data[i]! - b.data[i]!),
      Math.abs(a.data[i + 1]! - b.data[i + 1]!),
      Math.abs(a.data[i + 2]! - b.data[i + 2]!),
    );
    if (d > tolerance) n++;
  }
  return n;
}

const camera = (page: Page) => page.evaluate(() => (window as any).facetryChart.camera);
const animating = (page: Page) => page.evaluate(() => (window as any).facetryChart.animating as boolean);

test("a camera tween turns the camera over time and ends exactly on the target", async ({ page }) => {
  await open(page, "bar3d-grid", "&clock=manual");
  const start = await camera(page);
  const still = await shot(page);
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 120, elevation: 50, distance: 3.5 }, { duration: 600, easing: "linear" }));
  expect(await animating(page)).toBe(true);
  // Nothing has moved yet at time 0.
  expect(await camera(page)).toEqual(start);
  const first = await at(page, 0);
  expect(differing(first, still, 2)).toBe(0);
  // Halfway (linear): exactly in between.
  const mid = await at(page, 300);
  const cam = await camera(page);
  expect(cam.azimuth).toBeCloseTo((start.azimuth + 120) / 2, 2);
  expect(cam.elevation).toBeCloseTo((start.elevation + 50) / 2, 2);
  expect(cam.distance).toBeCloseTo((start.distance + 3.5) / 2, 2);
  expect(differing(mid, still)).toBeGreaterThan(3_000);
  const end = await at(page, 600);
  expect(await camera(page)).toEqual({ type: "perspective", azimuth: 120, elevation: 50, distance: 3.5, pan: [0, 0] });
  expect(await animating(page)).toBe(false);
  // The same picture as jumping there.
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 120, elevation: 50, distance: 3.5 }));
  expect(differing(end, await shot(page), 0)).toBe(0);
});

test("azimuth takes the shortest way round", async ({ page }) => {
  await open(page, "bar3d-grid", "&clock=manual");
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 350 }));
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 10 }, { duration: 400, easing: "linear" }));
  await at(page, 0);
  const seen: number[] = [];
  for (const ms of [100, 200, 300]) {
    await at(page, ms);
    seen.push((await camera(page)).azimuth);
  }
  // 350 -> 360/0 -> 10: it goes up through 0, never back through 180.
  expect(seen[0]).toBeCloseTo(355, 2);
  expect(seen[1]).toBeCloseTo(0, 2); // 360 wraps to 0
  expect(seen[2]).toBeCloseTo(5, 2);
  await at(page, 400);
  expect((await camera(page)).azimuth).toBeCloseTo(10, 3);
});

test("resetCamera tweens back to the spec's camera; orbiting stops a tween where it is", async ({ page }) => {
  await open(page, "bar3d-grid", "&clock=manual");
  const home = await camera(page);
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 200, elevation: 60 }));
  await page.evaluate(() => (window as any).facetryChart.resetCamera({ duration: 500 }));
  await at(page, 0);
  await at(page, 250);
  const mid = await camera(page);
  expect(mid.azimuth).not.toBeCloseTo(200, 0);
  expect(mid.azimuth).not.toBeCloseTo(home.azimuth, 0);
  await at(page, 500);
  expect(await camera(page)).toEqual(home);
  expect(await animating(page)).toBe(false);

  // A drag in the middle of a tween takes over: the camera stays where the drag leaves it.
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 200 }, { duration: 500 }));
  await at(page, 0);
  await at(page, 100);
  expect(await animating(page)).toBe(true);
  await page.mouse.move(400, 250);
  await page.mouse.down();
  await page.mouse.move(430, 250, { steps: 3 });
  await page.mouse.up();
  expect(await animating(page)).toBe(false);
  const after = await camera(page);
  await at(page, 10_000);
  expect(await camera(page)).toEqual(after);
});

test("a camera tween is a jump without options, with duration 0 and under reduced motion", async ({ page }) => {
  await open(page, "bar3d-grid");
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 100 }));
  expect(await animating(page)).toBe(false);
  expect((await camera(page)).azimuth).toBe(100);
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 150 }, { duration: 0 }));
  expect(await animating(page)).toBe(false);
  expect((await camera(page)).azimuth).toBe(150);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 200 }, { duration: 500 }));
  expect(await animating(page)).toBe(false);
  expect((await camera(page)).azimuth).toBe(200);
  // A bad option is reported.
  await page.emulateMedia({ reducedMotion: "no-preference" });
  const error = await page.evaluate(() => {
    try {
      (window as any).facetryChart.setCamera({ azimuth: 10 }, { duration: -5 });
      return null;
    } catch (e: any) {
      return e.code;
    }
  });
  expect(error).toBe("invalid-options");
});

test("the camera tween runs on the real clock, draws frames while it does and then stops", async ({ page }) => {
  await open(page, "bar3d-grid");
  await frames(page);
  const frameCount = () => page.evaluate(() => (window as any).facetryChart.renderCount as number);
  const idle = await frameCount();
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 120 }, { duration: 400 }));
  await expect.poll(() => animating(page)).toBe(false);
  const done = await frameCount();
  expect(done - idle).toBeGreaterThan(2);
  expect((await camera(page)).azimuth).toBe(120);
  await page.waitForTimeout(400);
  expect(await frameCount()).toBe(done);
});

test("a new camera in the spec moves the camera with the update's transition", async ({ page }) => {
  await open(page, "bar3d-grid", "&clock=manual");
  await page.evaluate(() => {
    const w = window as any;
    const spec = w.playgroundSpec();
    spec.camera = { azimuth: 150, elevation: 20, distance: 3 };
    w.facetryChart.update(spec, { transition: { duration: 400, easing: "linear" } });
  });
  expect(await animating(page)).toBe(true);
  await at(page, 0);
  await at(page, 200);
  const mid = await camera(page);
  expect(mid.azimuth).toBeGreaterThan(40);
  expect(mid.azimuth).toBeLessThan(140);
  await at(page, 400);
  expect(await camera(page)).toEqual({ type: "perspective", azimuth: 150, elevation: 20, distance: 3, pan: [0, 0] });
});

// --- legend toggles ----------------------------------------------------------------------------

async function entry(page: Page, series: string) {
  const at = await page.evaluate((s) => (window as any).facetryChart.legendPosition(s), series);
  expect(at, `no legend entry ${series}`).toBeTruthy();
  const box = (await page.locator("#chart").boundingBox())!;
  return { x: box.x + at.x, y: box.y + at.y };
}

test("a legend click animates the restack, and ends on what an instant toggle shows", async ({ page }) => {
  await open(page, "bar-stacked", "&clock=manual");
  const before = await shot(page);
  const { x, y } = await entry(page, "Software");
  await page.mouse.click(x, y);
  expect(await animating(page)).toBe(true);
  expect(await page.evaluate(() => (window as any).facetryChart.hiddenSeries)).toEqual(["Software"]);
  expect(differing(await at(page, 0), before, 2)).toBe(0);
  const mid = await at(page, 200);
  const end = await at(page, 10_000);
  expect(differing(mid, before)).toBeGreaterThan(3_000);
  expect(differing(mid, end)).toBeGreaterThan(3_000);
  expect(await animating(page)).toBe(false);
  // The same toggle without animation, through the API.
  await page.evaluate(() => (window as any).facetryChart.setSeriesVisible("Software", true, { transition: false }));
  await page.evaluate(() => (window as any).facetryChart.setSeriesVisible("Software", false, { transition: false }));
  expect(differing(end, await shot(page), 0)).toBe(0);
});

for (const example of ["pie", "bar3d-stacked", "bar-grouped", "bar-normalized"]) {
  test(`${example}: setSeriesVisible animates, can be interrupted and put back`, async ({ page }) => {
    await open(page, example, "&clock=manual");
    const names: string[] = await page.evaluate(() => {
      const w = window as any;
      const spec = w.playgroundSpec();
      const layer = spec.layers[0];
      const field = layer.encoding.color.field;
      return [...new Set(spec.data[layer.data][field] as string[])];
    });
    const before = await shot(page);
    expect(await page.evaluate((n) => (window as any).facetryChart.setSeriesVisible(n, false), names[1])).toBe(true);
    expect(await animating(page)).toBe(true);
    expect(differing(await at(page, 0), before, 2)).toBe(0);
    const mid = await at(page, 180);
    expect(differing(mid, before)).toBeGreaterThan(2_000);
    // Interrupt by bringing the series back: no jump, then it settles on the first picture.
    await page.evaluate((n) => (window as any).facetryChart.setSeriesVisible(n, true), names[1]);
    expect(differing(await at(page, 0), mid, 14)).toBeLessThan(120);
    const end = await at(page, 10_000);
    expect(differing(end, before, 0)).toBe(0);
  });
}
