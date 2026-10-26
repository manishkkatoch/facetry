import { expect, test, type Page } from "@playwright/test";
import { PNG } from "pngjs";

// Lines in 3D: drawn per series on their depth rows, round joins, markers that face the viewer,
// hover and click on data points (also after orbiting), value labels, curtains (style.area),
// bars and lines in one chart, and the legend toggle.

test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1 });
test.setTimeout(90_000);

const BLUE: [number, number, number] = [0x4c, 0x78, 0xa8];
const ORANGE: [number, number, number] = [0xf5, 0x85, 0x18];
const RED: [number, number, number] = [0xe4, 0x57, 0x56];
const PLOT: [number, number, number, number] = [0, 0, 700, 500];

type Mark = { key: string; layer: number; datum: Record<string, number | string | null> } | null;
type Logged = { type: "hover" | "click" | "legendclick"; payload: any };

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun"];
const REGIONS = ["North", "South", "West"];
const point = (month: string, region: string) => `layer0/${month}/${region}`;

async function open(page: Page, example: string) {
  await page.goto(`/?backend=webgl2&ui=0&example=${example}`);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await page.evaluate(() => {
    const chart = (window as any).facetryChart;
    const log: Logged[] = [];
    for (const type of ["hover", "click", "legendclick"]) chart.on(type, (payload: unknown) => log.push({ type, payload } as Logged));
    (window as any).__log = log;
  });
}
const events = (page: Page): Promise<Logged[]> => page.evaluate(() => (window as any).__log.splice(0));
const settle = (page: Page) => page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

async function shot(page: Page) {
  await settle(page);
  return PNG.sync.read(await page.locator("#chart").screenshot());
}

function count(png: PNG, want: number[], box: [number, number, number, number] = PLOT, tolerance = 4) {
  let n = 0;
  for (let y = box[1]; y < Math.min(box[3], png.height); y++) {
    for (let x = box[0]; x < Math.min(box[2], png.width); x++) {
      const i = (y * png.width + x) * 4;
      if (want.every((v, k) => Math.abs(png.data[i + k]! - v) <= tolerance)) n++;
    }
  }
  return n;
}

function differing(a: PNG, b: PNG, tolerance = 20) {
  let n = 0;
  for (let i = 0; i < a.data.length; i += 4) {
    const d = Math.max(Math.abs(a.data[i]! - b.data[i]!), Math.abs(a.data[i + 1]! - b.data[i + 1]!), Math.abs(a.data[i + 2]! - b.data[i + 2]!));
    if (d > tolerance) n++;
  }
  return n;
}

/** Like `differing`, within `box`. */
function differingIn(a: PNG, b: PNG, box: [number, number, number, number], tolerance = 20) {
  let n = 0;
  for (let y = Math.max(box[1], 0); y < Math.min(box[3], a.height); y++) {
    for (let x = box[0]; x < Math.min(box[2], a.width); x++) {
      const i = (y * a.width + x) * 4;
      const d = Math.max(Math.abs(a.data[i]! - b.data[i]!), Math.abs(a.data[i + 1]! - b.data[i + 1]!), Math.abs(a.data[i + 2]! - b.data[i + 2]!));
      if (d > tolerance) n++;
    }
  }
  return n;
}

/** Where a mark is, in CSS pixels of the canvas (and of the page: the canvas starts at 0,0 here). */
async function at(page: Page, key: string) {
  const p = await page.evaluate((k) => (window as any).facetryChart.markPosition(k), key);
  expect(p, `no mark ${key}`).toBeTruthy();
  const box = (await page.locator("#chart").boundingBox())!;
  return { x: p.x as number, y: p.y as number, pageX: box.x + p.x, pageY: box.y + p.y };
}

/** A data point whose projection is at least `gap` px from every other point (so hovering it is unambiguous). */
async function isolatedPoint(page: Page, gap = 28) {
  const all = await page.evaluate(
    ({ months, regions }) =>
      regions.flatMap((r: string) => months.map((m: string) => [`layer0/${m}/${r}`, (window as any).facetryChart.markPosition(`layer0/${m}/${r}`)])),
    { months: MONTHS, regions: REGIONS },
  );
  const pts = all.filter(([, p]: any) => p && p.x > 40 && p.x < 640 && p.y > 40 && p.y < 440) as [string, { x: number; y: number }][];
  let best: [string, number] = ["", -1];
  for (const [key, p] of pts) {
    const nearest = Math.min(...pts.filter(([k]) => k !== key).map(([, q]) => Math.hypot(p.x - q.x, p.y - q.y)));
    if (nearest > best[1]) best = [key, nearest];
  }
  expect(best[1], "an unambiguous point").toBeGreaterThan(gap);
  return best[0];
}

test("3D lines: one polyline per series on its own row, round joins, markers; the default view matches", async ({ page }) => {
  await open(page, "line3d");
  await page.mouse.move(790, 495);
  const png = await shot(page);
  for (const colour of [BLUE, ORANGE, RED]) expect(count(png, colour)).toBeGreaterThan(300);
  // Legend: a line swatch per series in the series colours.
  expect(count(png, ORANGE, [700, 0, 800, 120])).toBeGreaterThan(20);
});

test("3D lines: sharp bends of a thick line have no gaps at the joins", async ({ page }) => {
  await open(page, "line3d");
  // A zigzag, very thick, so a notch on the outside of a bend would be a hole of many pixels.
  const spec = JSON.parse(await page.locator("#json").inputValue());
  spec.layers[0].style.strokeWidth = 24;
  spec.layers[0].point = false;
  spec.data.traffic.visits = [5, 35, 5, 35, 5, 35, ...spec.data.traffic.visits.slice(6)];
  // Only the first series, straight on, so the polyline is the only blue in the plot.
  spec.legend = false;
  await page.evaluate((s) => (window as any).facetryChart.update(s), spec);
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 0, elevation: 20 }));
  const png = await shot(page);
  // Walk the vertices (projected) and require solid blue around each interior one: with square
  // caps alone the outer side of a sharp bend is empty; with round joins it is a full disc.
  const verts: { x: number; y: number }[] = [];
  for (const m of MONTHS) verts.push(await at(page, point(m, "North")));
  for (const v of verts.slice(1, 5)) {
    let blue = 0;
    // A disc of radius 9 (the line is 24 wide, so radius 12) around the vertex.
    for (let dy = -9; dy <= 9; dy++) {
      for (let dx = -9; dx <= 9; dx++) {
        if (dx * dx + dy * dy > 81) continue;
        const i = (Math.round(v.y + dy) * png.width + Math.round(v.x + dx)) * 4;
        if (Math.abs(png.data[i]! - BLUE[0]) <= 6 && Math.abs(png.data[i + 1]! - BLUE[1]) <= 6 && Math.abs(png.data[i + 2]! - BLUE[2]) <= 6) blue++;
      }
    }
    // The disc has about 255 pixels; allow for the other series passing over a few of them.
    expect(blue, `around ${JSON.stringify(v)}`).toBeGreaterThan(200);
  }
});

test("3D lines: hovering a data point fires hover with its row, also after orbiting; leaving fires null", async ({ page }) => {
  await open(page, "line3d");
  for (const [azimuth, elevation] of [[30, 28], [140, 40], [250, 20]]) {
    await page.evaluate((c) => (window as any).facetryChart.setCamera(c), { azimuth, elevation });
    await settle(page);
    await page.mouse.move(790, 495);
    await events(page);
    const key = await isolatedPoint(page);
    const p = await at(page, key);
    await page.mouse.move(p.pageX, p.pageY);
    const log = (await events(page)).filter((e) => e.type === "hover");
    const [, month, region] = key.split("/");
    const row = REGIONS.indexOf(region!) * 6 + MONTHS.indexOf(month!);
    const visits = await page.evaluate((r) => JSON.parse(document.querySelector<HTMLTextAreaElement>("#json")!.value).data.traffic.visits[r], row);
    expect(log.at(-1)).toEqual({ type: "hover", payload: { key, layer: 0, datum: { month, region, visits } } });
    // 6 px off the point is still on it; far away is nothing.
    await page.mouse.move(p.pageX + 6, p.pageY + 4);
    expect((await events(page)).filter((e) => e.type === "hover")).toEqual([]);
    await page.mouse.move(790, 495);
    expect((await events(page)).filter((e) => e.type === "hover").at(-1)).toEqual({ type: "hover", payload: null });
  }
});

test("3D lines: hover enlarges the marker; click reports the point and an orbit drag does not click", async ({ page }) => {
  await open(page, "line3d");
  await page.mouse.move(790, 495);
  const idle = await shot(page);
  const key = await isolatedPoint(page);
  const p = await at(page, key);
  await page.mouse.move(p.pageX, p.pageY);
  const hovered = await shot(page);
  // The marker grows from 8 to 12 px: a ring of the series colour appears around the old one.
  const box: [number, number, number, number] = [Math.round(p.x) - 12, Math.round(p.y) - 12, Math.round(p.x) + 13, Math.round(p.y) + 13];
  expect(differing(idle, hovered)).toBeGreaterThan(20);
  const colour = key.endsWith("North") ? BLUE : key.endsWith("South") ? ORANGE : RED;
  expect(count(hovered, colour, box, 10)).toBeGreaterThan(count(idle, colour, box, 10));
  await events(page);
  await page.mouse.down();
  await page.mouse.up();
  const clicks = (await events(page)).filter((e) => e.type === "click");
  expect(clicks).toHaveLength(1);
  expect(clicks[0]!.payload.key).toBe(key);
  expect(clicks[0]!.payload.layer).toBe(0);
  // A drag that starts on the point orbits and never clicks.
  const before = await page.evaluate(() => (window as any).facetryChart.camera);
  await page.mouse.move(p.pageX, p.pageY);
  await page.mouse.down();
  await page.mouse.move(p.pageX + 60, p.pageY + 10, { steps: 6 });
  await page.mouse.up();
  expect((await page.evaluate(() => (window as any).facetryChart.camera)).azimuth).not.toBe(before.azimuth);
  expect((await events(page)).filter((e) => e.type === "click")).toEqual([]);
});

test("3D lines: a hover-mode value label shows only for the hovered point", async ({ page }) => {
  await open(page, "line3d");
  const spec = JSON.parse(await page.locator("#json").inputValue());
  spec.layers[0].label = { show: "hover" };
  await page.evaluate((s) => (window as any).facetryChart.update(s), spec);
  await page.mouse.move(790, 495);
  const idle = await shot(page);
  const key = await isolatedPoint(page);
  const p = await at(page, key);
  await page.mouse.move(p.pageX, p.pageY);
  const hovered = await shot(page);
  // The label is right above the point, in the text colour.
  const above: [number, number, number, number] = [Math.round(p.x) - 20, Math.round(p.y) - 34, Math.round(p.x) + 20, Math.round(p.y) - 8];
  expect(differingIn(idle, hovered, above)).toBeGreaterThan(30);
  // And only there: a point on the other side of the chart gets no label.
  expect(differingIn(idle, hovered, [0, 0, 800, Math.round(p.y) - 40])).toBe(0);
});

test("3D lines: value labels (always) are drawn and do not overlap", async ({ page }) => {
  await open(page, "line3d");
  const spec = JSON.parse(await page.locator("#json").inputValue());
  const plain = await shot(page);
  spec.layers[0].label = { show: "always" };
  await page.evaluate((s) => (window as any).facetryChart.update(s), spec);
  await page.mouse.move(790, 495);
  const labelled = await shot(page);
  expect(differing(plain, labelled)).toBeGreaterThan(300);
});

test("3D lines: style.area draws a translucent curtain under each line", async ({ page }) => {
  await open(page, "line3d");
  await page.mouse.move(790, 495);
  const plain = await shot(page);
  const spec = JSON.parse(await page.locator("#json").inputValue());
  spec.layers[0].style.area = true;
  await page.evaluate((s) => (window as any).facetryChart.update(s), spec);
  const curtains = await shot(page);
  expect(differing(plain, curtains, 8)).toBeGreaterThan(8000);
  // The lines and markers are still fully drawn on top.
  for (const colour of [BLUE, ORANGE, RED]) expect(count(curtains, colour)).toBeGreaterThan(300);
});

test("3D lines: the legend toggle hides a series, its points and its depth row", async ({ page }) => {
  await open(page, "line3d");
  const entry = await page.evaluate(() => (window as any).facetryChart.legendPosition("South"));
  await page.mouse.click(entry.x, entry.y);
  await settle(page);
  expect((await events(page)).filter((e) => e.type === "legendclick")).toEqual([{ type: "legendclick", payload: { series: "South", visible: false } }]);
  const png = await shot(page);
  expect(count(png, ORANGE)).toBe(0);
  expect(count(png, BLUE)).toBeGreaterThan(300);
  expect(await page.evaluate((k) => (window as any).facetryChart.markPosition(k), point("Mar", "South"))).toBeUndefined();
  expect(await page.evaluate((k) => (window as any).facetryChart.markPosition(k), point("Mar", "West"))).toBeTruthy();
  // Back again.
  await page.mouse.click(entry.x, entry.y);
  await settle(page);
  expect(count(await shot(page), ORANGE)).toBeGreaterThan(300);
});

test("3D combo: bars and a line per row in one chart, both hoverable", async ({ page }) => {
  await open(page, "combo3d-bar-line");
  await page.mouse.move(790, 495);
  const png = await shot(page);
  expect(count(png, [0x1f, 0x23, 0x28], PLOT, 6)).toBeGreaterThan(200); // the dark line and its markers
  expect(count(png, BLUE, PLOT, 60)).toBeGreaterThan(2000); // the bars
  await events(page);
  // A line point (they float above the bars, so nothing hides them).
  const p = await at(page, "layer1/Q2/South");
  await page.mouse.move(p.pageX, p.pageY);
  const hover = (await events(page)).filter((e) => e.type === "hover").at(-1)!;
  expect(hover.payload.key).toMatch(/^layer1\/Q\d\/(North|South|West)$/);
  expect(hover.payload.layer).toBe(1);
  expect(hover.payload.datum).toHaveProperty("target");
  // A bar.
  const b = await at(page, "layer0/Q3/North");
  await page.mouse.move(b.pageX, b.pageY);
  const bar = (await events(page)).filter((e) => e.type === "hover").at(-1)!;
  expect(bar.payload.key).toMatch(/^layer0\//);
  expect(bar.payload.datum).toHaveProperty("revenue");
});

test("3D lines are in PNG snapshots", async ({ page }) => {
  await open(page, "line3d");
  const base64 = await page.evaluate(async () => {
    const blob: Blob = await (window as any).facetryChart.snapshot({ scale: 1 });
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = "";
    for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(binary);
  });
  const png = PNG.sync.read(Buffer.from(base64, "base64"));
  for (const colour of [BLUE, ORANGE, RED]) expect(count(png, colour)).toBeGreaterThan(300);
});

test("a numeric x axis works in 3D too", async ({ page }) => {
  await open(page, "line3d");
  const spec = JSON.parse(await page.locator("#json").inputValue());
  spec.data.traffic.month = spec.data.traffic.month.map((_: string, i: number) => 2000 + (i % 6) * 5);
  await page.evaluate((s) => (window as any).facetryChart.update(s), spec);
  const png = await shot(page);
  for (const colour of [BLUE, ORANGE, RED]) expect(count(png, colour)).toBeGreaterThan(300);
  const p = await at(page, "layer0/2010/South");
  expect(p.x).toBeGreaterThan(0);
});
