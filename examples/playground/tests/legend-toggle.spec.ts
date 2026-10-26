import { expect, test, type Page } from "@playwright/test";
import { PNG } from "pngjs";

// Click a legend entry to hide or show its series: bars, stacks, pie slices and 3D boxes leave
// and come back, axes refit, the legend keeps the entry (dimmed), the camera stays, and a legend
// click is never also a chart click or an orbit drag.

test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1 });
test.setTimeout(90_000);

const BLUE: [number, number, number] = [0x4c, 0x78, 0xa8];
const ORANGE: [number, number, number] = [0xf5, 0x85, 0x18];
const RED: [number, number, number] = [0xe4, 0x57, 0x56];

type Logged = { type: "legendclick" | "click" | "hover"; payload: unknown };

async function open(page: Page, example: string) {
  await page.goto(`/?backend=webgl2&ui=0&example=${example}`);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await page.evaluate(() => {
    const chart = (window as any).facetryChart;
    const log: Logged[] = [];
    chart.on("legendclick", (payload: unknown) => log.push({ type: "legendclick", payload }));
    chart.on("click", (payload: unknown) => log.push({ type: "click", payload }));
    (window as any).__log = log;
  });
}

const events = (page: Page): Promise<Logged[]> => page.evaluate(() => (window as any).__log.splice(0));
const hidden = (page: Page): Promise<string[]> => page.evaluate(() => (window as any).facetryChart.hiddenSeries);
const camera = (page: Page) => page.evaluate(() => (window as any).facetryChart.camera);
const cursor = (page: Page) => page.locator("#chart").evaluate((c) => (c as HTMLCanvasElement).style.cursor);

async function shot(page: Page) {
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  return PNG.sync.read(await page.locator("#chart").screenshot());
}

/** Pixels of `png` within `box` close to `want`. */
function count(png: PNG, want: number[], box: [number, number, number, number], tolerance = 4) {
  let n = 0;
  for (let y = box[1]; y < Math.min(box[3], png.height); y++) {
    for (let x = box[0]; x < Math.min(box[2], png.width); x++) {
      const i = (y * png.width + x) * 4;
      if (want.every((v, k) => Math.abs(png.data[i + k]! - v) <= tolerance)) n++;
    }
  }
  return n;
}

/** Pixels in `box` that look like Software's orange, lit or not (3D shading darkens colours). */
function countOrangish(png: PNG, box: [number, number, number, number]) {
  let n = 0;
  for (let y = box[1]; y < Math.min(box[3], png.height); y++) {
    for (let x = box[0]; x < Math.min(box[2], png.width); x++) {
      const i = (y * png.width + x) * 4;
      const [r, g, b] = [png.data[i]!, png.data[i + 1]!, png.data[i + 2]!];
      if (r > 100 && r / g > 1.5 && r / g < 2.4 && g / Math.max(b, 1) > 3) n++;
    }
  }
  return n;
}

/** How many pixels differ by more than `tolerance` in any channel. */
function differing(a: PNG, b: PNG, tolerance = 20) {
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

/** Page coordinates of a legend entry. */
async function entry(page: Page, series: string) {
  const at = await page.evaluate((s) => (window as any).facetryChart.legendPosition(s), series);
  expect(at, `no legend entry ${series}`).toBeTruthy();
  const box = (await page.locator("#chart").boundingBox())!;
  return { x: box.x + at.x, y: box.y + at.y };
}

async function clickEntry(page: Page, series: string) {
  const { x, y } = await entry(page, series);
  await page.mouse.click(x, y);
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
}

/** The plot's part of the 800x500 canvas (everything left of the right-hand legend), and the legend's. */
const PLOT: [number, number, number, number] = [0, 0, 700, 500];
const LEGEND: [number, number, number, number] = [706, 0, 800, 120];
/** A swatch drawn at 40% opacity over the white background. */
const DIMMED_ORANGE = [251, 218, 205];

test("2D stacked bars: a legend click hides the series, restacks, refits the axis and restores on a second click", async ({ page }) => {
  await open(page, "bar-stacked");
  await page.mouse.move(2, 2);
  const before = await shot(page);
  expect(count(before, ORANGE, PLOT)).toBeGreaterThan(2000); // Software's segments
  expect(count(before, ORANGE, LEGEND)).toBeGreaterThanOrEqual(90);
  const bottom = await page.evaluate(() => (window as any).facetryChart.markPosition("layer0/Q3/Hardware"));

  await clickEntry(page, "Software");
  expect(await events(page)).toEqual([{ type: "legendclick", payload: { series: "Software", visible: false } }]);
  expect(await hidden(page)).toEqual(["Software"]);
  const after = await shot(page);
  // The series is gone from the plot, and its entry is still in the legend, dimmed (not the
  // full orange any more, but not removed either: something other than white is there).
  expect(count(after, ORANGE, PLOT)).toBe(0);
  expect(count(after, ORANGE, LEGEND)).toBe(0);
  expect(count(after, DIMMED_ORANGE, LEGEND, 14)).toBeGreaterThanOrEqual(80);
  expect(differing(before, after)).toBeGreaterThan(5000);
  // Colours do not shift: the others keep theirs.
  expect(count(after, BLUE, PLOT)).toBeGreaterThan(2000);
  expect(count(after, RED, PLOT)).toBeGreaterThan(500);
  // The axis refits: without Software the tallest stack is 170, not 280, so the bottom
  // segment of Q3 (140 of 170 instead of 140 of 300) now centres much higher.
  const refit = await page.evaluate(() => (window as any).facetryChart.markPosition("layer0/Q3/Hardware"));
  expect(refit.y).toBeLessThan(bottom.y - 40);
  expect(await page.evaluate(() => (window as any).facetryChart.markPosition("layer0/Q3/Software"))).toBeUndefined();

  // Second click: back exactly as it was.
  await clickEntry(page, "Software");
  expect(await events(page)).toEqual([{ type: "legendclick", payload: { series: "Software", visible: true } }]);
  expect(await hidden(page)).toEqual([]);
  await page.mouse.move(2, 2);
  expect(differing(before, await shot(page))).toBe(0);
});

test("2D: the legend entry is clickable on its label too, shows a pointer cursor, and a legend click is not a chart click", async ({ page }) => {
  await open(page, "bar-stacked");
  const swatch = await entry(page, "Services");
  expect(await cursor(page)).toBe("");
  await page.mouse.move(swatch.x, swatch.y);
  expect(await cursor(page)).toBe("pointer");
  await page.mouse.move(100, 100);
  expect(await cursor(page)).toBe("");
  // On the label, a few px from the swatch.
  await page.mouse.click(swatch.x + 18, swatch.y);
  const log = await events(page);
  expect(log).toEqual([{ type: "legendclick", payload: { series: "Services", visible: false } }]);
  // A click on the plot still clicks the chart, and does not toggle anything.
  const bar = await page.evaluate(() => (window as any).facetryChart.markPosition("layer0/Q2/Hardware"));
  const box = (await page.locator("#chart").boundingBox())!;
  await page.mouse.click(box.x + bar.x, box.y + bar.y);
  const after = await events(page);
  expect(after.map((e) => e.type)).toEqual(["click"]);
  expect(await hidden(page)).toEqual(["Services"]);
});

test("a press that starts on a legend entry and is released elsewhere toggles nothing", async ({ page }) => {
  await open(page, "bar-stacked");
  const e = await entry(page, "Hardware");
  await page.mouse.move(e.x, e.y);
  await page.mouse.down();
  await page.mouse.move(e.x - 200, e.y + 100, { steps: 5 });
  await page.mouse.up();
  expect(await events(page)).toEqual([]);
  expect(await hidden(page)).toEqual([]);
});

test("pie: hiding a slice re-divides the circle; the hidden slice stays in the legend", async ({ page }) => {
  await open(page, "pie");
  await page.mouse.move(2, 2);
  const before = await shot(page);
  expect(count(before, ORANGE, PLOT)).toBeGreaterThan(2000); // Social
  const searchBefore = count(before, BLUE, PLOT);
  await clickEntry(page, "Social");
  expect(await events(page)).toEqual([{ type: "legendclick", payload: { series: "Social", visible: false } }]);
  const after = await shot(page);
  expect(count(after, ORANGE, PLOT)).toBe(0);
  // Search is now a bigger share of the same circle (420 of 642 instead of 420 of 982).
  expect(count(after, BLUE, PLOT)).toBeGreaterThan(searchBefore * 1.3);
  // Percent labels follow.
  await clickEntry(page, "Social");
  expect(await events(page)).toEqual([{ type: "legendclick", payload: { series: "Social", visible: true } }]);
  await page.mouse.move(2, 2);
  expect(differing(before, await shot(page))).toBe(0);
});

test("3D stacked bars: a legend click hides the series, keeps the camera, and does not orbit", async ({ page }) => {
  await open(page, "bar3d-stacked");
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 120, elevation: 40 }));
  const cam = await camera(page);
  await page.mouse.move(2, 2);
  const before = await shot(page);
  expect(countOrangish(before, PLOT)).toBeGreaterThan(1000);

  // A drag that starts on a legend entry does not orbit.
  const e = await entry(page, "Software");
  await page.mouse.move(e.x, e.y);
  expect(await cursor(page)).toBe("pointer");
  await page.mouse.down();
  await page.mouse.move(e.x - 150, e.y + 80, { steps: 6 });
  await page.mouse.up();
  expect(await camera(page)).toEqual(cam);
  expect(await events(page)).toEqual([]);
  expect(await hidden(page)).toEqual([]);

  // The click toggles it.
  await clickEntry(page, "Software");
  expect(await events(page)).toEqual([{ type: "legendclick", payload: { series: "Software", visible: false } }]);
  expect(await camera(page)).toEqual(cam);
  const after = await shot(page);
  expect(countOrangish(after, PLOT)).toBe(0);
  expect(count(after, BLUE, PLOT, 60)).toBeGreaterThan(500);
  expect(differing(before, after)).toBeGreaterThan(5000);
  // The marks that are left are pickable where they are drawn.
  const at = await page.evaluate(() => (window as any).facetryChart.markPosition("layer0/Q2/Services/North"));
  expect(at).toBeTruthy();
  expect(await page.evaluate(() => (window as any).facetryChart.markPosition("layer0/Q2/Software/North"))).toBeUndefined();

  await clickEntry(page, "Software");
  expect(await hidden(page)).toEqual([]);
  expect(await camera(page)).toEqual(cam);
  await page.mouse.move(2, 2);
  expect(differing(before, await shot(page))).toBe(0);
  // Orbiting by dragging the chart still works.
  await page.mouse.move(300, 300);
  await page.mouse.down();
  await page.mouse.move(380, 300, { steps: 5 });
  await page.mouse.up();
  expect((await camera(page)).azimuth).not.toBe(cam.azimuth);
});

test("the last visible series cannot be hidden; the entry click then does nothing and fires nothing", async ({ page }) => {
  await open(page, "bar-stacked");
  await clickEntry(page, "Hardware");
  await clickEntry(page, "Software");
  expect((await events(page)).map((e) => e.payload)).toEqual([
    { series: "Hardware", visible: false },
    { series: "Software", visible: false },
  ]);
  await clickEntry(page, "Services");
  expect(await events(page)).toEqual([]);
  expect(await hidden(page)).toEqual(["Hardware", "Software"]);
  expect(await page.evaluate(() => (window as any).facetryChart.setSeriesVisible("Services", false))).toBe(false);
  // The API: unknown names and no-op calls report false.
  const api = await page.evaluate(() => {
    const c = (window as any).facetryChart;
    return [c.setSeriesVisible("Nope", false), c.setSeriesVisible("Hardware", false), c.setSeriesVisible("Hardware", true)];
  });
  expect(api).toEqual([false, false, true]);
  expect(await hidden(page)).toEqual(["Software"]);
  // The API does not fire legendclick.
  expect(await events(page)).toEqual([]);
});

test("update keeps the viewer's hidden series for series that still exist, unless the spec says legend.hidden", async ({ page }) => {
  await open(page, "bar-stacked");
  await page.evaluate(() => (window as any).facetryChart.setSeriesVisible("Software", false));
  const spec = JSON.parse(await page.locator("#json").inputValue());

  await page.evaluate((s) => (window as any).facetryChart.update(s), spec);
  expect(await hidden(page)).toEqual(["Software"]);

  // Data without Software: nothing left to keep.
  const without = structuredClone(spec);
  without.data.sales.product = without.data.sales.product.map((p: string) => (p === "Software" ? "Services" : p));
  await page.evaluate((s) => (window as any).facetryChart.update(s), without);
  expect(await hidden(page)).toEqual([]);

  // Software is back in the data: the viewer's choice was forgotten with the series.
  await page.evaluate((s) => (window as any).facetryChart.update(s), spec);
  expect(await hidden(page)).toEqual([]);

  // The spec's own legend.hidden replaces the viewer's set.
  await page.evaluate(() => (window as any).facetryChart.setSeriesVisible("Software", false));
  const explicit = { ...spec, legend: { hidden: ["Hardware", "Nope"] } };
  await page.evaluate((s) => (window as any).facetryChart.update(s), explicit);
  expect(await hidden(page)).toEqual(["Hardware"]);
  // Hiding everything is a validation issue with a path.
  const bad = { ...spec, legend: { hidden: ["Hardware", "Software", "Services"] } };
  const issues = await page.evaluate((s) => {
    try {
      (window as any).facetryChart.update(s);
      return null;
    } catch (e: any) {
      return e.issues;
    }
  }, bad);
  expect(issues).toEqual([{ path: "/legend/hidden", message: expect.stringContaining("every series") }]);
});

test("legend.interactive: false keeps the legend but ignores clicks and shows no pointer", async ({ page }) => {
  await open(page, "bar-stacked");
  const spec = JSON.parse(await page.locator("#json").inputValue());
  await page.evaluate((s) => (window as any).facetryChart.update(s), { ...spec, legend: { interactive: false } });
  expect(await page.evaluate(() => (window as any).facetryChart.legendPosition("Software"))).toBeUndefined();
  const png = await shot(page);
  expect(count(png, ORANGE, LEGEND)).toBeGreaterThanOrEqual(90); // still drawn
  await page.mouse.click(720, 41);
  await page.mouse.click(745, 41);
  // Plain chart clicks (on empty space), no legend clicks.
  expect((await events(page)).map((e) => e.type)).toEqual(["click", "click"]);
  expect(await hidden(page)).toEqual([]);
  expect(await cursor(page)).toBe("");
  // The API still hides.
  expect(await page.evaluate(() => (window as any).facetryChart.setSeriesVisible("Software", false))).toBe(true);
});

test("a PNG snapshot shows the hidden state, and the dimmed entry", async ({ page }) => {
  await open(page, "bar-stacked");
  await clickEntry(page, "Software");
  const base64 = await page.evaluate(async () => {
    const blob: Blob = await (window as any).facetryChart.snapshot({ scale: 1 });
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = "";
    for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(binary);
  });
  const png = PNG.sync.read(Buffer.from(base64, "base64"));
  expect(count(png, ORANGE, [0, 0, png.width, png.height])).toBe(0);
  expect(count(png, BLUE, PLOT)).toBeGreaterThan(2000);
  expect(count(png, DIMMED_ORANGE, LEGEND, 14)).toBeGreaterThanOrEqual(80);
});

test("legend.hidden starts a chart with a series hidden", async ({ page }) => {
  await open(page, "bar-stacked-hidden");
  expect(await hidden(page)).toEqual(["Software"]);
  const png = await shot(page);
  expect(count(png, ORANGE, PLOT)).toBe(0);
  // Clicking its entry brings it back.
  await clickEntry(page, "Software");
  expect(await hidden(page)).toEqual([]);
  expect(count(await shot(page), ORANGE, PLOT)).toBeGreaterThan(2000);
});

test("lines: hiding a series removes its line and markers, hover then finds only what is left", async ({ page }) => {
  await open(page, "line-basic");
  await page.mouse.move(2, 2);
  const before = await shot(page);
  expect(count(before, BLUE, PLOT)).toBeGreaterThan(300); // Free
  expect(count(before, ORANGE, PLOT)).toBeGreaterThan(300); // Pro
  await clickEntry(page, "Free");
  expect(await events(page)).toEqual([{ type: "legendclick", payload: { series: "Free", visible: false } }]);
  const after = await shot(page);
  expect(count(after, BLUE, PLOT)).toBe(0);
  expect(count(after, ORANGE, PLOT)).toBeGreaterThan(300);
  expect(await page.evaluate(() => (window as any).facetryChart.markPosition("layer0/Jan/Free"))).toBeUndefined();
  expect(await page.evaluate(() => (window as any).facetryChart.markPosition("layer0/Jan/Pro"))).toBeTruthy();
  await clickEntry(page, "Free");
  await page.mouse.move(2, 2);
  expect(differing(before, await shot(page))).toBe(0);
});
