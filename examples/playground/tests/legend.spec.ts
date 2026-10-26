import { expect, test, type Page } from "@playwright/test";
import { PNG } from "pngjs";

// The legend: drawn with the chart (and in PNG snapshots), laid out beside the plot, never
// hovered, and in 3D an overlay that the plot is framed beside (so picking must offset by it).

test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1 });
test.setTimeout(60_000);

const BLUE: [number, number, number] = [0x4c, 0x78, 0xa8];
const ORANGE: [number, number, number] = [0xf5, 0x85, 0x18];
const RED: [number, number, number] = [0xe4, 0x57, 0x56];

async function open(page: Page, example: string) {
  await page.goto(`/?backend=webgl2&example=${example}`);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
}

let updates = 0;
test.beforeEach(() => (updates = 0));
async function edit(page: Page, change: (spec: any) => void) {
  const spec = JSON.parse(await page.locator("#json").inputValue());
  change(spec);
  await page.locator("#json").fill(JSON.stringify(spec));
  await page.locator("#apply").click();
  await expect(page.locator("html")).toHaveAttribute("data-updates", String(++updates));
}

async function shot(page: Page) {
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  // The config runs these tests with reduced motion, so every update is a jump: by now nothing
  // may still be moving. Checked here so a failure says why the picture is not the final one.
  const motion = await page.evaluate(() => ({
    reduced: matchMedia("(prefers-reduced-motion: reduce)").matches,
    animating: (window as any).facetryChart.animating as boolean,
  }));
  expect(motion).toEqual({ reduced: true, animating: false });
  return PNG.sync.read(await page.locator("#chart").screenshot());
}

/** Pixels of `png` within `box` close to `want`. */
function count(png: PNG, want: number[], box: [number, number, number, number]) {
  let n = 0;
  for (let y = box[1]; y < Math.min(box[3], png.height); y++) {
    for (let x = box[0]; x < Math.min(box[2], png.width); x++) {
      const i = (y * png.width + x) * 4;
      if (want.every((v, k) => Math.abs(png.data[i + k]! - v) <= 4)) n++;
    }
  }
  return n;
}

async function snapshotPng(page: Page, options?: object) {
  const base64 = await page.evaluate(async (opts) => {
    const blob: Blob = await (window as any).facetryChart.snapshot(opts);
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = "";
    for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(binary);
  }, options);
  return PNG.sync.read(Buffer.from(base64, "base64"));
}

test("a colour field gets a legend on the right: swatches in the series colours, and the plot gives way", async ({ page }) => {
  await open(page, "bar-stacked");
  // Low bars on a fixed axis, so nothing but the legend can be in the top-right corner.
  await edit(page, (s) => {
    s.data.sales.revenue = s.data.sales.revenue.map((v: number) => v / 10);
    s.axes[1].scale = { domain: [0, 300] };
  });
  await page.mouse.move(2, 2);
  const png = await shot(page);
  // Three 10x10 swatches in the top-right corner.
  const corner: [number, number, number, number] = [png.width - 150, 0, png.width, 110];
  for (const colour of [BLUE, ORANGE, RED]) expect(count(png, colour, corner)).toBeGreaterThanOrEqual(90);
  // With the legend off the corner is empty (no bar reaches that high there)...
  await edit(page, (s) => (s.legend = false));
  await page.mouse.move(2, 2);
  const without = await shot(page);
  for (const colour of [BLUE, ORANGE, RED]) expect(count(without, colour, corner)).toBe(0);
  // ...and the bars are wider.
  expect(count(without, BLUE, [0, 0, without.width, without.height])).toBeGreaterThan(
    count(png, BLUE, [0, 0, png.width, png.height]),
  );
});

test("every orientation puts the legend on its side and still shows every swatch", async ({ page }) => {
  await open(page, "bar-stacked");
  await page.mouse.move(2, 2);
  const { width: w, height: h } = (await page.locator("#chart").boundingBox())!;
  const where = {
    right: [w - 160, 0, w, h],
    left: [0, 0, 160, h],
    top: [0, 0, w, 40],
    bottom: [0, h - 40, w, h],
  } as const;
  for (const [orient, box] of Object.entries(where)) {
    await edit(page, (s) => (s.legend = { orient, title: "Product" }));
    await page.mouse.move(2, 2);
    const png = await shot(page);
    for (const colour of [BLUE, ORANGE, RED]) {
      // The bars have these colours too, so look only in the legend's strip, which the
      // plot does not enter (top and bottom strips are 40 px; side strips 160).
      expect(count(png, colour, box as unknown as [number, number, number, number]), `${orient} ${colour}`).toBeGreaterThanOrEqual(90);
    }
  }
});

test("the legend is part of PNG snapshots", async ({ page }) => {
  await open(page, "line-basic");
  const { width: w, height: h } = (await page.locator("#chart").boundingBox())!;
  const png = await snapshotPng(page, { scale: 1 });
  expect([png.width, png.height]).toEqual([Math.round(w), Math.round(h)]);
  // Line swatches are 18 px wide and a couple of px thick in the series colours.
  expect(count(png, BLUE, [png.width - 120, 0, png.width, 60])).toBeGreaterThan(20);
  expect(count(png, ORANGE, [png.width - 120, 0, png.width, 60])).toBeGreaterThan(20);
  const big = await snapshotPng(page, { scale: 2 });
  expect([big.width, big.height]).toEqual([Math.round(w) * 2, Math.round(h) * 2]);
  expect(count(big, BLUE, [big.width - 240, 0, big.width, 120])).toBeGreaterThan(80);
});

test("hovering a legend entry hovers nothing, while the marks beside it still do", async ({ page }) => {
  await open(page, "bar-stacked");
  const events = await page.evaluate(() => {
    const log: unknown[] = [];
    (window as any).facetryChart.on("hover", (m: unknown) => log.push(m));
    (window as any).__log = log;
    return log.length;
  });
  expect(events).toBe(0);
  const box = (await page.locator("#chart").boundingBox())!;
  await page.mouse.move(box.x + box.width - 60, box.y + 24); // on a legend label
  expect(await page.evaluate(() => (window as any).__log.length)).toBe(0);
  const at = await page.evaluate(() => (window as any).facetryChart.markPosition("layer0/Q2/Software"));
  await page.mouse.move(box.x + at.x, box.y + at.y);
  expect(await page.evaluate(() => (window as any).__log.at(-1).key)).toBe("layer0/Q2/Software");
});

test("3D: the plot is framed beside the legend and picking follows it", async ({ page }) => {
  await open(page, "bar3d-stacked");
  const box = (await page.locator("#chart").boundingBox())!;
  for (const orient of ["right", "left", "top", "bottom"]) {
    await edit(page, (s) => (s.legend = { orient }));
    await page.evaluate(() => {
      const log: any[] = [];
      (window as any).facetryChart.on("hover", (m: any) => log.push(m));
      (window as any).__log = log;
    });
    for (const key of ["layer0/Q2/Software/North", "layer0/Q4/Hardware/North"]) {
      await page.mouse.move(2, 2);
      const at = await page.evaluate((k) => (window as any).facetryChart.markPosition(k), key);
      // The box's projected middle is inside the canvas, clear of the legend's band...
      expect(at.x).toBeGreaterThan(0);
      expect(at.x).toBeLessThan(box.width);
      await page.mouse.move(box.x + at.x, box.y + at.y);
      // ...and hovering there finds that stack (the segment aimed at, or another segment of the
      // same stack that is in front of it from this angle): never nothing, never another cell.
      const last = await page.evaluate(() => (window as any).__log.at(-1));
      const [, quarter] = key.split("/");
      expect(last?.key, `${orient} ${key}`).toMatch(new RegExp(`^layer0/${quarter}/[A-Za-z]+/North$`));
    }
  }
});

test("3D: a PNG snapshot includes the legend overlay and the 3D picture beside it", async ({ page }) => {
  await open(page, "bar3d-grid");
  const { width: w, height: h } = (await page.locator("#chart").boundingBox())!;
  const png = await snapshotPng(page, { scale: 1 });
  expect([png.width, png.height]).toEqual([Math.round(w), Math.round(h)]);
  // North's swatch (the series colour, 10x10) in the top-right corner, a lot of it.
  expect(count(png, BLUE, [png.width - 150, 0, png.width, 40])).toBeGreaterThanOrEqual(90);
  // And the screen agrees with the snapshot (same pipelines, same passes).
  await page.mouse.move(2, 2);
  const onScreen = await shot(page);
  let different = 0;
  for (let i = 0; i < png.data.length; i += 4) {
    const d = Math.max(
      Math.abs(png.data[i]! - onScreen.data[i]!),
      Math.abs(png.data[i + 1]! - onScreen.data[i + 1]!),
      Math.abs(png.data[i + 2]! - onScreen.data[i + 2]!),
    );
    if (d > 24) different++;
  }
  expect(different / (png.width * png.height)).toBeLessThan(0.01);
});
