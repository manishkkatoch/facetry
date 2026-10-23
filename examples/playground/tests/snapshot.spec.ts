import { expect, test, type Page } from "@playwright/test";
import { PNG } from "pngjs";

// `chart.snapshot()` draws the chart again into an offscreen texture and returns a PNG. These
// tests compare it with what is on screen (same pipelines, so they should agree) and check the
// size, scale, transparency and the "does not disturb the canvas" promises.

test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1 });
test.setTimeout(60_000);

const BAR = [0x4c, 0x78, 0xa8];

async function open(page: Page, example: string) {
  await page.goto(`/?backend=webgl2&ui=0&example=${example}`);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
}

/** Calls `chart.snapshot(options)` in the page and decodes the PNG it resolves with. */
async function snapshot(page: Page, options?: object) {
  const { type, base64 } = await page.evaluate(async (opts) => {
    const blob: Blob = await (window as any).facetryChart.snapshot(opts);
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = "";
    for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return { type: blob.type, base64: btoa(binary) };
  }, options);
  const buffer = Buffer.from(base64, "base64");
  return { type, buffer, png: PNG.sync.read(buffer) };
}

async function screen(page: Page) {
  return PNG.sync.read(await page.locator("#chart").screenshot());
}

const at = (png: PNG, x: number, y: number) => {
  const i = (y * png.width + x) * 4;
  return [png.data[i]!, png.data[i + 1]!, png.data[i + 2]!, png.data[i + 3]!];
};

/** Share of pixels whose channels differ by more than `tolerance`. */
function differing(a: PNG, b: PNG, tolerance = 8) {
  expect([a.width, a.height]).toEqual([b.width, b.height]);
  let n = 0;
  for (let i = 0; i < a.data.length; i += 4) {
    const d = Math.max(
      Math.abs(a.data[i]! - b.data[i]!),
      Math.abs(a.data[i + 1]! - b.data[i + 1]!),
      Math.abs(a.data[i + 2]! - b.data[i + 2]!),
    );
    if (d > tolerance) n++;
  }
  return n / (a.width * a.height);
}

const count = (png: PNG, want: number[], tolerance = 4) => {
  let n = 0;
  for (let i = 0; i < png.data.length; i += 4) {
    if (want.every((v, k) => Math.abs(png.data[i + k]! - v) <= tolerance)) n++;
  }
  return n;
};

const near = (got: number[], want: number[], tolerance = 3) => want.every((v, k) => Math.abs(got[k]! - v) <= tolerance);

test("a 3D snapshot after orbiting is a PNG of the canvas's size that matches the screen", async ({ page }) => {
  await open(page, "bar3d-grid");
  await page.mouse.move(400, 250);
  await page.mouse.down();
  await page.mouse.move(470, 270, { steps: 6 });
  await page.mouse.up();
  await page.mouse.move(2, 2); // off the bars: no hover highlight
  await expect.poll(async () => (await page.evaluate(() => (window as any).facetryChart.camera)).azimuth).not.toBe(35);
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

  const shot = await snapshot(page);
  expect(shot.type).toBe("image/png");
  expect(shot.buffer.subarray(0, 8).toString("hex")).toBe("89504e470d0a1a0a");
  expect([shot.png.width, shot.png.height]).toEqual([800, 500]);

  const onScreen = await screen(page);
  // Same pipelines, same camera, same layout: the pictures agree (allowing for tiny
  // anti-aliasing differences), and so do individual sample pixels.
  expect(differing(shot.png, onScreen)).toBeLessThan(0.002);
  for (const [x, y] of [[400, 250], [5, 5], [795, 495], [300, 300], [500, 200], [250, 400]] as const) {
    const a = at(shot.png, x, y);
    const b = at(onScreen, x, y);
    expect(near(a, b, 6), `pixel ${x},${y}: snapshot ${a} vs screen ${b}`).toBe(true);
  }
  // Not an empty or flat picture: it has the bars and the white background.
  expect(count(shot.png, [255, 255, 255])).toBeGreaterThan(100_000);
  expect(differing(shot.png, new PNG({ width: 800, height: 500, fill: true }), 40)).toBeGreaterThan(0.1);
});

test("orbiting changes the snapshot, which follows the current camera", async ({ page }) => {
  await open(page, "bar3d-grid");
  const before = await snapshot(page);
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 200, elevation: 40 }));
  const after = await snapshot(page);
  expect(differing(before.png, after.png, 30)).toBeGreaterThan(0.05);
  // Each one matches the screen as it was at that time.
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  expect(differing(after.png, await screen(page))).toBeLessThan(0.002);
  // Going back gives the first picture again.
  await page.evaluate(() => (window as any).facetryChart.resetCamera());
  expect(differing(before.png, (await snapshot(page)).png, 8)).toBe(0);
});

test("a snapshot at scale 2 has twice the size, and its text and bars are laid out for it", async ({ page }) => {
  await open(page, "bar-basic");
  const one = await snapshot(page, { scale: 1 });
  const two = await snapshot(page, { scale: 2 });
  expect([one.png.width, one.png.height]).toEqual([800, 500]);
  expect([two.png.width, two.png.height]).toEqual([1600, 1000]);
  // The bars' area grows with the square of the scale (layout was redone, not stretched).
  const ratio = count(two.png, BAR) / count(one.png, BAR);
  expect(ratio).toBeGreaterThan(3.8);
  expect(ratio).toBeLessThan(4.2);
  // A 1x snapshot is the canvas, pixel for pixel.
  expect(differing(one.png, await screen(page))).toBeLessThan(0.002);
  // The wide picture still has the same overall look: shrunk to half size it resembles the 1x one.
  let close = 0;
  for (let y = 0; y < 500; y += 5) {
    for (let x = 0; x < 800; x += 5) {
      if (near(at(two.png, x * 2, y * 2), at(one.png, x, y).slice(0, 3), 12)) close++;
    }
  }
  expect(close / (160 * 100)).toBeGreaterThan(0.97);
});

test("the default scale is the device pixel ratio, and scale is capped at 4", async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 400, height: 250 }, deviceScaleFactor: 2 });
  const page = await context.newPage();
  await open(page, "bar-basic");
  const canvas = await page.evaluate(() => {
    const c = document.querySelector<HTMLCanvasElement>("#chart")!;
    return [c.width, c.height];
  });
  expect(canvas).toEqual([800, 500]);
  const dflt = await snapshot(page);
  expect([dflt.png.width, dflt.png.height]).toEqual([800, 500]);
  expect(differing(dflt.png, await screen(page))).toBeLessThan(0.002);
  const capped = await snapshot(page, { scale: 100 });
  expect([capped.png.width, capped.png.height]).toEqual([1600, 1000]); // 4 x 400 x 250
  await context.close();
});

test("a transparent background has alpha 0 where nothing is drawn and exact colours where it is", async ({ page }) => {
  await open(page, "bar-basic");
  const clear = await snapshot(page, { background: "transparent" });
  const themed = await snapshot(page, { background: "theme" });
  expect([clear.png.width, clear.png.height]).toEqual([800, 500]);
  expect(at(clear.png, 2, 2)).toEqual([0, 0, 0, 0]);
  expect(at(clear.png, 797, 2)[3]).toBe(0);
  expect(at(themed.png, 2, 2)).toEqual([255, 255, 255, 255]);
  // Inside a bar: the bar's own colour, fully opaque, either way.
  expect(at(clear.png, 150, 300)).toEqual([...BAR, 255]);
  expect(at(themed.png, 150, 300)).toEqual([...BAR, 255]);
  // Where the two differ it is only the background: composited over white (blending in
  // linear light, as the GPU does for the themed one) the transparent picture is the themed
  // one, anti-aliased edges and text included. That proves the colours were un-premultiplied
  // in the right colour space.
  const decode = (v: number) => (v / 255 <= 0.04045 ? v / 255 / 12.92 : ((v / 255 + 0.055) / 1.055) ** 2.4);
  const encode = (l: number) => 255 * (l <= 0.0031308 ? l * 12.92 : 1.055 * l ** (1 / 2.4) - 0.055);
  let worst = 0;
  for (let i = 0; i < clear.png.data.length; i += 4) {
    const a = clear.png.data[i + 3]! / 255;
    for (let c = 0; c < 3; c++) {
      const over = Math.round(encode(decode(clear.png.data[i + c]!) * a + (1 - a)));
      worst = Math.max(worst, Math.abs(over - themed.png.data[i + c]!));
    }
  }
  expect(worst).toBeLessThanOrEqual(4);
  // Antialiased pixels (partial alpha) exist: the text and bar edges.
  let partial = 0;
  for (let i = 3; i < clear.png.data.length; i += 4) if (clear.png.data[i]! > 0 && clear.png.data[i]! < 255) partial++;
  expect(partial).toBeGreaterThan(500);
});

test("a snapshot leaves the canvas untouched and the chart keeps working", async ({ page }) => {
  await open(page, "bar3d-grid");
  const size = () => page.evaluate(() => {
    const c = document.querySelector<HTMLCanvasElement>("#chart")!;
    return [c.width, c.height];
  });
  const before = await screen(page);
  await snapshot(page, { scale: 3 });
  await snapshot(page, { background: "transparent" });
  expect(await size()).toEqual([800, 500]);
  // Still the same picture, with no render requested in between...
  expect(differing(before, await screen(page), 0)).toBe(0);
  // ...and it still responds: orbiting redraws.
  await page.mouse.move(400, 250);
  await page.mouse.down();
  await page.mouse.move(480, 250, { steps: 5 });
  await page.mouse.up();
  await page.mouse.move(2, 2);
  await expect.poll(async () => differing(before, await screen(page), 30)).toBeGreaterThan(0.05);
  // A resize of the viewport after snapshots lays out normally too.
  await page.setViewportSize({ width: 600, height: 400 });
  await expect.poll(size).toEqual([600, 400]);
  const resized = await snapshot(page);
  expect([resized.png.width, resized.png.height]).toEqual([600, 400]);
});

test("bad options reject with typed errors; concurrent snapshots both finish", async ({ page }) => {
  await open(page, "bar-basic");
  const codes = await page.evaluate(async () => {
    const chart = (window as any).facetryChart;
    const out: string[] = [];
    for (const options of [{ scale: -1 }, { scale: 0 }, { scale: NaN }, { scale: "2" }, { background: "red" }]) {
      try {
        await chart.snapshot(options);
        out.push("resolved");
      } catch (e) {
        out.push(`${(e as Error).name}/${(e as { code: string }).code}`);
      }
    }
    return out;
  });
  expect(codes).toEqual(Array(5).fill("FacetryError/invalid-options"));

  const sizes = await page.evaluate(async () => {
    const chart = (window as any).facetryChart;
    const decode = async (blob: Blob) => {
      const bitmap = await createImageBitmap(blob);
      return [bitmap.width, bitmap.height];
    };
    const [a, b, c] = await Promise.all([chart.snapshot({ scale: 1 }), chart.snapshot({ scale: 2 }), chart.snapshot({ scale: 0.5 })]);
    return [await decode(a), await decode(b), await decode(c)];
  });
  expect(sizes).toEqual([[800, 500], [1600, 1000], [400, 250]]);
});

test("a snapshot started before a camera change shows the state at the time of the call", async ({ page }) => {
  await open(page, "bar3d-grid");
  const before = await snapshot(page);
  const pair = await page.evaluate(async () => {
    const chart = (window as any).facetryChart;
    const pending = chart.snapshot();
    chart.setCamera({ azimuth: 150 }); // after the call: must not show up in `pending`
    const blob: Blob = await pending;
    const bytes = new Uint8Array(await blob.arrayBuffer());
    let binary = "";
    for (let i = 0; i < bytes.length; i += 0x8000) binary += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(binary);
  });
  const early = PNG.sync.read(Buffer.from(pair, "base64"));
  expect(differing(before.png, early, 8)).toBe(0);
});

test("the Save PNG button downloads the picture", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 700 });
  await page.goto("/?backend=webgl2&example=bar-basic");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  const download = page.waitForEvent("download");
  await page.locator("#save-png").click();
  const file = await download;
  expect(file.suggestedFilename()).toBe("facetry-bar-basic.png");
  const path = await file.path();
  const png = PNG.sync.read((await import("node:fs")).readFileSync(path));
  const canvas = await page.evaluate(() => {
    const c = document.querySelector<HTMLCanvasElement>("#chart")!;
    return [c.width, c.height];
  });
  expect([png.width, png.height]).toEqual(canvas);
});
