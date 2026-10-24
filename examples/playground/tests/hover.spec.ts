import { expect, test, type Page } from "@playwright/test";
import { PNG } from "pngjs";

// Hover, click and value labels. The playground (with its editor) is used so a test can change
// the spec; `markPosition(key)` says where a bar is on screen, which is how a test aims at it.

test.use({ viewport: { width: 1000, height: 640 }, deviceScaleFactor: 1 });

type Mark = { key: string; layer: number; datum: Record<string, number | string | null> } | null;
type Logged = { type: "hover" | "click"; mark: Mark };

async function open(page: Page, example: string) {
  await page.goto(`/?backend=webgl2&example=${example}`);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
}

let updates = 0;
/** Edits the spec in the playground's editor and applies it. */
async function edit(page: Page, change: (spec: any) => void) {
  const spec = JSON.parse(await page.locator("#json").inputValue());
  change(spec);
  await page.locator("#json").fill(JSON.stringify(spec));
  await page.locator("#apply").click();
  await expect(page.locator("html")).toHaveAttribute("data-updates", String(++updates));
}

async function listen(page: Page) {
  await page.evaluate(() => {
    const log: Logged[] = [];
    const chart = (window as any).facetryChart;
    chart.on("hover", (mark: Mark) => log.push({ type: "hover", mark }));
    chart.on("click", (mark: Mark) => log.push({ type: "click", mark }));
    (window as any).__log = log;
  });
}
const events = (page: Page): Promise<Logged[]> => page.evaluate(() => (window as any).__log.splice(0));

/** Page coordinates of a bar's centre (the canvas does not start at the page's corner). */
async function where(page: Page, key: string) {
  const at = await page.evaluate((k) => (window as any).facetryChart.markPosition(k), key);
  expect(at, `no bar ${key}`).toBeTruthy();
  const box = (await page.locator("#chart").boundingBox())!;
  return { x: box.x + at.x, y: box.y + at.y, inCanvas: at as { x: number; y: number } };
}

/** Page coordinates of the point `(x, y)` of the canvas. */
async function canvasPoint(page: Page, x: number, y: number) {
  const box = (await page.locator("#chart").boundingBox())!;
  return { x: box.x + x, y: box.y + y };
}

const settle = (page: Page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
const shot = async (page: Page) => {
  await settle(page);
  return PNG.sync.read(await page.locator("#chart").screenshot());
};

/** Bounding box and count of pixels that differ between two pictures. */
function diff(a: PNG, b: PNG, tolerance = 20) {
  let n = 0;
  let [x0, y0, x1, y1] = [Infinity, Infinity, -1, -1];
  for (let y = 0; y < a.height; y++) {
    for (let x = 0; x < a.width; x++) {
      const i = (y * a.width + x) * 4;
      const d = Math.max(Math.abs(a.data[i]! - b.data[i]!), Math.abs(a.data[i + 1]! - b.data[i + 1]!), Math.abs(a.data[i + 2]! - b.data[i + 2]!));
      if (d > tolerance) {
        n++;
        [x0, y0, x1, y1] = [Math.min(x0, x), Math.min(y0, y), Math.max(x1, x), Math.max(y1, y)];
      }
    }
  }
  return { n, x0, y0, x1, y1 };
}

test.beforeEach(() => {
  updates = 0;
});

test("2D: hovering a bar fires hover with its datum, moving within it does not repeat, leaving fires null", async ({ page }) => {
  await open(page, "bar-labels");
  await listen(page);
  const q3 = await where(page, "layer0/Q3");
  await page.mouse.move(q3.x, q3.y);
  expect(await events(page)).toEqual([
    { type: "hover", mark: { key: "layer0/Q3", layer: 0, datum: { quarter: "Q3", revenue: 1400 } } },
  ]);
  await page.mouse.move(q3.x + 20, q3.y + 30, { steps: 3 }); // still inside the same bar
  expect(await events(page)).toEqual([]);
  const q1 = await where(page, "layer0/Q1");
  await page.mouse.move(q1.x, q1.y, { steps: 12 }); // across the gaps and Q2 to Q1
  const crossing = await events(page);
  const keys = crossing.map((e) => e.mark?.key ?? null);
  expect(keys).toContain("layer0/Q2");
  expect(keys).toContain(null); // the gap between bars
  expect(keys.every((k, i) => i === 0 || k !== keys[i - 1])).toBe(true); // never the same twice in a row
  expect(crossing.at(-1)).toEqual({ type: "hover", mark: { key: "layer0/Q1", layer: 0, datum: { quarter: "Q1", revenue: 1200 } } });
  // Off the bars, in the margin above the plot but still on the canvas: null.
  const margin = await canvasPoint(page, q1.inCanvas.x, 8);
  await page.mouse.move(margin.x, margin.y);
  expect((await events(page)).at(-1)).toEqual({ type: "hover", mark: null });
  // Negative values report negative numbers.
  const q5 = await where(page, "layer0/Q5");
  await page.mouse.move(q5.x, q5.y);
  expect((await events(page)).at(-1)!.mark!.datum).toEqual({ quarter: "Q5", revenue: -300 });
  // Leaving the canvas altogether.
  await page.mouse.move(5, 5);
  expect((await events(page)).at(-1)).toEqual({ type: "hover", mark: null });
});

test("2D: a hover-mode label appears only on the hovered bar, and leaving clears it", async ({ page }) => {
  await open(page, "bar-labels");
  await edit(page, (s) => (s.layers[0].label.show = "hover"));
  await page.mouse.move(2, 2);
  const idle = await shot(page);

  // Hovered, with the label (B) and the same without it (C): the difference is the label alone.
  const q1 = await where(page, "layer0/Q1");
  await page.mouse.move(q1.x, q1.y);
  const withLabel = await shot(page);
  expect(diff(idle, withLabel).n).toBeGreaterThan(5_000); // the bar is lightened, and the label...
  await page.mouse.move(2, 2);
  expect(diff(idle, await shot(page)).n).toBe(0); // ...and leaving restores everything

  await edit(page, (s) => delete s.layers[0].label);
  const idleNoLabel = await shot(page);
  expect(diff(idle, idleNoLabel).n).toBe(0); // no hover, no label: nothing of it is drawn
  await page.mouse.move(q1.x, q1.y);
  const noLabel = await shot(page);

  const label = diff(withLabel, noLabel);
  expect(label.n).toBeGreaterThan(20);
  // It is right above the bar: centred on it, higher than the bar's middle, not wider than "1,200".
  expect(label.x0).toBeGreaterThan(q1.inCanvas.x - 40);
  expect(label.x1).toBeLessThan(q1.inCanvas.x + 40);
  expect(label.y1).toBeLessThan(q1.inCanvas.y);
  // Hovering another bar shows only that bar's label.
  await edit(page, (s) => (s.layers[0].label = { show: "hover" }));
  await page.mouse.move(2, 2);
  const q2 = await where(page, "layer0/Q2");
  await page.mouse.move(q2.x, q2.y);
  const second = await shot(page);
  await page.mouse.move(q1.x, q1.y);
  const first = await shot(page);
  const where1 = diff(first, second);
  expect(where1.n).toBeGreaterThan(5_000); // two different bars lit up, two different labels
});

test("2D: always-labels are drawn without any hover", async ({ page }) => {
  await open(page, "bar-labels");
  await page.mouse.move(2, 2);
  const withLabels = await shot(page);
  await edit(page, (s) => delete s.layers[0].label);
  const without = await shot(page);
  const d = diff(withLabels, without);
  expect(d.n).toBeGreaterThan(60); // five numbers
  // Each bar's label sits within its own column.
  const q1 = await where(page, "layer0/Q1");
  const q5 = await where(page, "layer0/Q5");
  expect(d.x0).toBeGreaterThan(q1.inCanvas.x - 60);
  expect(d.x1).toBeLessThan(q5.inCanvas.x + 60);
});

test("2D: a click fires once with the bar, a click on empty space fires null, off() removes a listener", async ({ page }) => {
  await open(page, "bar-labels");
  await listen(page);
  const q2 = await where(page, "layer0/Q2");
  await page.mouse.click(q2.x, q2.y);
  const log = await events(page);
  expect(log.filter((e) => e.type === "click")).toEqual([
    { type: "click", mark: { key: "layer0/Q2", layer: 0, datum: { quarter: "Q2", revenue: 950 } } },
  ]);
  const margin = await canvasPoint(page, q2.inCanvas.x, 8);
  await page.mouse.click(margin.x, margin.y); // above everything
  expect((await events(page)).filter((e) => e.type === "click")).toEqual([{ type: "click", mark: null }]);

  const removed = await page.evaluate(() => {
    const chart = (window as any).facetryChart;
    const seen: unknown[] = [];
    const once = (m: unknown) => seen.push(m);
    chart.on("click", once);
    chart.off("click", once);
    return seen;
  });
  await page.mouse.click(q2.x, q2.y);
  expect(removed).toEqual([]);
  // A listener that throws does not stop the others or the chart.
  await page.evaluate(() => (window as any).facetryChart.on("click", () => { throw new Error("listener bug"); }));
  await page.mouse.click(q2.x, q2.y);
  expect((await events(page)).filter((e) => e.type === "click").length).toBe(2);
});

test("3D: hovering a bar fires hover with its datum, and its label shows only while hovered", async ({ page }) => {
  await open(page, "bar3d-labels");
  await listen(page);
  await page.mouse.move(2, 2);
  const idle = await shot(page);
  const q3 = await where(page, "layer0/Q3/North");
  await page.mouse.move(q3.x, q3.y);
  expect(await events(page)).toEqual([
    { type: "hover", mark: { key: "layer0/Q3/North", layer: 0, datum: { quarter: "Q3", region: "North", revenue: 140 } } },
  ]);
  const withLabel = await shot(page);
  await page.mouse.move(2, 2);
  expect((await events(page)).at(-1)).toEqual({ type: "hover", mark: null });
  expect(diff(idle, await shot(page)).n).toBe(0);

  await edit(page, (s) => delete s.layers[0].label);
  await page.mouse.move(q3.x, q3.y);
  const noLabel = await shot(page);
  const label = diff(withLabel, noLabel);
  expect(label.n).toBeGreaterThan(20);
  // Above the bar's top face, centred on its x.
  expect(label.y1).toBeLessThan(q3.inCanvas.y);
  expect(Math.abs((label.x0 + label.x1) / 2 - q3.inCanvas.x)).toBeLessThan(40);
});

test("3D: picking follows the camera: the bar you see under the pointer is the one reported", async ({ page }) => {
  await open(page, "bar3d-labels");
  await listen(page);
  // At the default camera the North row is in front; after turning the chart around the West
  // row is. Aiming at the centre of a front-row bar must hit exactly that bar.
  const rows = async (row: string) => {
    for (const q of ["Q1", "Q2", "Q3", "Q4"]) {
      const key = `layer0/${q}/${row}`;
      const p = await where(page, key);
      await page.mouse.move(2, 2);
      await events(page);
      await page.mouse.move(p.x, p.y);
      const log = await events(page);
      expect(log.at(-1)?.mark?.key, `aiming at ${key}`).toBe(key);
    }
  };
  await rows("North");
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 215, elevation: 30 }));
  await settle(page);
  await rows("West");
  // And the first row seen from behind is hidden behind the others: its centre picks a nearer bar.
  const hidden = await where(page, "layer0/Q2/North");
  await page.mouse.move(2, 2);
  await events(page);
  await page.mouse.move(hidden.x, hidden.y);
  const log = await events(page);
  expect(log.at(-1)?.mark?.key).not.toBe("layer0/Q2/North");
});

test("3D: a click fires once, a drag that orbits does not click, a tiny wobble still does", async ({ page }) => {
  await open(page, "bar3d-labels");
  await listen(page);
  const q1 = await where(page, "layer0/Q1/North");
  await page.mouse.click(q1.x, q1.y);
  let log = await events(page);
  expect(log.filter((e) => e.type === "click")).toEqual([
    { type: "click", mark: { key: "layer0/Q1/North", layer: 0, datum: { quarter: "Q1", region: "North", revenue: 120 } } },
  ]);

  // A real drag starting on the bar: orbits, and does not click.
  const before = await page.evaluate(() => (window as any).facetryChart.camera.azimuth);
  await page.mouse.move(q1.x, q1.y);
  await events(page);
  await page.mouse.down();
  await page.mouse.move(q1.x + 90, q1.y + 10, { steps: 8 });
  await page.mouse.up();
  log = await events(page);
  expect(log.filter((e) => e.type === "click")).toEqual([]);
  expect(await page.evaluate(() => (window as any).facetryChart.camera.azimuth)).not.toBe(before);
  // The chart moved under the pointer: the hover was cleared when the drag began.
  expect(log.find((e) => e.type === "hover")?.mark).toBeNull();

  // A wobble within the click slop is a click, and does not orbit.
  await page.evaluate(() => (window as any).facetryChart.resetCamera());
  await page.mouse.move(2, 2);
  const q4 = await where(page, "layer0/Q4/North");
  await events(page);
  await page.mouse.move(q4.x, q4.y);
  await page.mouse.down();
  await page.mouse.move(q4.x + 2, q4.y + 1);
  await page.mouse.up();
  log = await events(page);
  expect(log.filter((e) => e.type === "click").map((e) => e.mark?.key)).toEqual(["layer0/Q4/North"]);
  expect(await page.evaluate(() => (window as any).facetryChart.camera.azimuth)).toBe(35);
});

test("changing the spec while hovering reports the new bar under the pointer", async ({ page }) => {
  await open(page, "bar-labels");
  await listen(page);
  const q1 = await where(page, "layer0/Q1");
  await page.mouse.move(q1.x, q1.y);
  await events(page);
  // Update the chart with the pointer staying where it is.
  await page.evaluate(() => {
    const spec = JSON.parse((document.querySelector("#json") as HTMLTextAreaElement).value);
    spec.data.sales.revenue[0] = 777;
    (window as any).facetryChart.update(spec);
  });
  const log = await events(page);
  // First "nothing" (the bars were rebuilt), then the new Q1 under the still pointer.
  expect(log.map((e) => e.mark?.datum?.revenue ?? null)).toEqual([null, 777]);
});

test("2D charts report hover and click even though they cannot orbit", async ({ page }) => {
  await open(page, "bar-basic");
  await listen(page);
  await expect(page.locator("#camera")).toBeHidden();
  const q2 = await where(page, "layer0/Q2");
  await page.mouse.click(q2.x, q2.y);
  const log = await events(page);
  expect(log.map((e) => `${e.type}:${e.mark?.key}`)).toEqual(["hover:layer0/Q2", "click:layer0/Q2"]);
});

test("stacked bars: hover reports the segment's row, and merged rows report their sum", async ({ page }) => {
  await open(page, "bar-stacked");
  await listen(page);
  const seg = await where(page, "layer0/Q2/Software");
  await page.mouse.move(seg.x, seg.y);
  expect(await events(page)).toEqual([
    { type: "hover", mark: { key: "layer0/Q2/Software", layer: 0, datum: { quarter: "Q2", product: "Software", revenue: 95 } } },
  ]);
  // A second Q2 Software row: the stack merges them (95 + 5) and the datum is the first row
  // with the summed revenue.
  await page.evaluate(() => {
    const spec = JSON.parse((document.querySelector("#json") as HTMLTextAreaElement).value);
    const t = spec.data.sales;
    t.quarter.push("Q2");
    t.product.push("Software");
    t.revenue.push(5);
    (window as any).facetryChart.update(spec);
  });
  // The pointer stayed where it was: hover cleared by the relayout, then found again.
  const log = await events(page);
  expect(log.at(-1)).toEqual({
    type: "hover",
    mark: { key: "layer0/Q2/Software", layer: 0, datum: { quarter: "Q2", product: "Software", revenue: 100 } },
  });
});

test("3D stacked bars: hovering a segment reports it", async ({ page }) => {
  await open(page, "bar3d-stacked");
  await listen(page);
  const key = "layer0/Q3/Software/North";
  const p = await where(page, key);
  await page.mouse.move(p.x, p.y);
  const log = await events(page);
  expect(log.at(-1)?.mark?.key).toBe(key);
  expect(log.at(-1)?.mark?.datum).toEqual({ quarter: "Q3", region: "North", product: "Software", revenue: 110 });
});

test("line points: the nearest point within reach is hovered, with a marker and its hover label; far from it a bar is", async ({ page }) => {
  await open(page, "combo-bar-line");
  await listen(page);
  await page.mouse.move(2, 2);
  const idle = await shot(page);
  const point = await where(page, "layer1/Q3");
  const bar = await where(page, "layer0/Q3");
  expect(Math.hypot(point.inCanvas.x - bar.inCanvas.x, point.inCanvas.y - bar.inCanvas.y)).toBeGreaterThan(30);

  // Ten pixels off the point still counts (the reach is 12), though the pointer is inside a bar.
  await page.mouse.move(point.x + 10, point.y);
  expect((await events(page)).at(-1)).toEqual({
    type: "hover",
    mark: { key: "layer1/Q3", layer: 1, datum: { quarter: "Q3", revenue: 140, margin: 21 } },
  });
  const hovered = await shot(page);
  const d = diff(idle, hovered);
  // The marker and the hover label (the layer asks for `show: "hover"`) appeared around the point...
  expect(d.n, JSON.stringify(d)).toBeGreaterThan(150);
  expect(d.x0).toBeLessThan(point.inCanvas.x);
  expect(d.x1).toBeGreaterThan(point.inCanvas.x);
  expect(d.y0).toBeLessThan(point.inCanvas.y);
  // ...and nowhere far from it (the bar did not light up).
  expect(d.x1 - d.x0).toBeLessThan(80);
  expect(d.y1 - d.y0).toBeLessThan(80);

  // Out of reach: the bar under the pointer.
  await page.mouse.move(bar.x, bar.y);
  expect((await events(page)).at(-1)).toEqual({
    type: "hover",
    mark: { key: "layer0/Q3", layer: 0, datum: { quarter: "Q3", revenue: 140, margin: 21 } },
  });
  // Leaving restores the idle picture exactly.
  await page.mouse.move(2, 2);
  expect(diff(idle, await shot(page)).n).toBe(0);
});

test("line points: click reports the point, and a point is reachable by its key", async ({ page }) => {
  await open(page, "line-basic");
  await listen(page);
  const p = await where(page, "layer0/Mar/Pro");
  await page.mouse.click(p.x, p.y);
  expect((await events(page)).filter((e) => e.type === "click")).toEqual([
    { type: "click", mark: { key: "layer0/Mar/Pro", layer: 0, datum: { month: "Mar", plan: "Pro", visits: 22 } } },
  ]);
});
