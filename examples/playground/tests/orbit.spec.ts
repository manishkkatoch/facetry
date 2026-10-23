import { expect, test, type Page } from "@playwright/test";
import { PNG } from "pngjs";

test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1 });

type Camera = { type: string; azimuth: number; elevation: number; distance: number };

async function open(page: Page, example: string) {
  await page.goto(`/?backend=webgl2&ui=0&example=${example}`);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
}

/** The camera the chart reports (mirrored into a data attribute by the playground). */
async function camera(page: Page): Promise<Camera> {
  return page.evaluate(() => (window as any).facetryChart.camera);
}

/** Waits for the playground's mirrored attribute to show `azimuth` (it updates on the next frame). */
async function shownAzimuth(page: Page) {
  return JSON.parse((await page.locator("html").getAttribute("data-camera")) ?? "{}").azimuth as number;
}

async function pixels(page: Page) {
  return PNG.sync.read(await page.locator("#chart").screenshot());
}

/** How many pixels differ visibly between two screenshots. */
function changedPixels(a: PNG, b: PNG) {
  let n = 0;
  for (let i = 0; i < a.data.length; i += 4) {
    const d = Math.abs(a.data[i]! - b.data[i]!) + Math.abs(a.data[i + 1]! - b.data[i + 1]!) + Math.abs(a.data[i + 2]! - b.data[i + 2]!);
    if (d > 30) n++;
  }
  return n;
}

async function drag(page: Page, dx: number, dy: number) {
  await page.mouse.move(400, 250);
  await page.mouse.down();
  await page.mouse.move(400 + dx, 250 + dy, { steps: 8 });
  await page.mouse.up();
}

test("dragging orbits: the azimuth changes and so do the pixels", async ({ page }) => {
  await open(page, "bar3d-grid");
  const before = await pixels(page);
  expect((await camera(page)).azimuth).toBe(35);

  await drag(page, 100, 0); // 100 px right = 40 degrees, the chart follows the pointer
  await expect.poll(() => shownAzimuth(page)).toBeCloseTo(355, 1);
  expect((await camera(page)).elevation).toBe(25);
  expect(changedPixels(before, await pixels(page))).toBeGreaterThan(20_000);
});

test("dragging up and down tilts and stops at the elevation limits", async ({ page }) => {
  await open(page, "bar3d-grid");
  await drag(page, 0, 20); // down 20 px = +8 degrees
  await expect.poll(async () => (await camera(page)).elevation).toBeCloseTo(33, 1);
  await page.mouse.move(400, 100);
  await page.mouse.down();
  await page.mouse.move(400, 100 + 400, { steps: 4 }); // way past the top
  await page.mouse.up();
  await expect.poll(async () => (await camera(page)).elevation).toBe(85);
});

test("double-click resets the camera to the spec's", async ({ page }) => {
  await open(page, "bar3d-grid");
  const initial = await pixels(page);
  await drag(page, 100, 30);
  expect((await camera(page)).azimuth).not.toBe(35);
  await page.locator("#chart").dblclick({ position: { x: 400, y: 250 } });
  await expect.poll(() => camera(page)).toEqual({ type: "perspective", azimuth: 35, elevation: 25, distance: 2.7 });
  await expect.poll(async () => changedPixels(initial, await pixels(page))).toBe(0);
});

test("the wheel zooms without scrolling the page, and zooming changes the picture", async ({ page }) => {
  await open(page, "bar3d-grid");
  const before = await pixels(page);
  await page.mouse.move(400, 250);
  await page.mouse.wheel(0, -300); // wheel up = zoom in
  await expect.poll(async () => (await camera(page)).distance).toBeLessThan(2.7);
  expect(await page.evaluate(() => window.scrollY)).toBe(0);
  expect(changedPixels(before, await pixels(page))).toBeGreaterThan(20_000);
});

test("setCamera moves the camera and clamps it; bad values are typed errors", async ({ page }) => {
  await open(page, "bar3d-grid");
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 200, elevation: 400, distance: 0.1 }));
  const c = await camera(page);
  expect(c).toEqual({ type: "perspective", azimuth: 200, elevation: 85, distance: 1.3 });
  await page.evaluate(() => (window as any).facetryChart.setCamera({ type: "orthographic" }));
  expect((await camera(page)).type).toBe("orthographic");
  await page.evaluate(() => (window as any).facetryChart.resetCamera());
  expect((await camera(page)).azimuth).toBe(35);
  // A bad value is a typed error, not a silent no-op.
  const message = await page.evaluate(() => {
    try {
      (window as any).facetryChart.setCamera({ azimuth: "left" });
    } catch (e) {
      return `${(e as Error).name}: ${(e as Error).message}`;
    }
    return "no error";
  });
  expect(message).toContain("FacetryError");
  expect(message).toContain("azimuth");
});

test("every 3D example in the dropdown draws without errors", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 700 });
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.goto("/?backend=webgl2");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  const names = await page.locator("#example option").allTextContents();
  const threeD = names.filter((n) => n.startsWith("bar3d-"));
  expect(threeD).toEqual(["bar3d-grid", "bar3d-negative", "bar3d-row"]);
  let updates = 0;
  for (const name of threeD) {
    await page.locator("#example").selectOption(name);
    await expect(page.locator("html")).toHaveAttribute("data-updates", String(++updates));
    await expect(page.locator("#camera")).toBeVisible();
    // Something other than the white background was drawn.
    const png = await pixels(page);
    expect(changedPixels(png, new PNG({ width: png.width, height: png.height, fill: true }))).toBeGreaterThan(5_000);
  }
  expect(errors).toEqual([]);
});

test("2D charts and orbit:false charts ignore gestures", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 700 });
  await page.goto("/?backend=webgl2");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  // 2D: no camera readout, nothing moves.
  await expect(page.locator("#camera")).toBeHidden();
  const before = await pixels(page);
  await page.mouse.move(300, 300);
  await page.mouse.down();
  await page.mouse.move(400, 330, { steps: 4 });
  await page.mouse.up();
  await page.mouse.wheel(0, 200);
  expect(changedPixels(before, await pixels(page))).toBe(0);

  // Switch to a 3D example, then turn orbiting off in its spec.
  await page.locator("#example").selectOption("bar3d-grid");
  await expect(page.locator("html")).toHaveAttribute("data-updates", "1");
  await expect(page.locator("#camera")).toContainText("azimuth 35");
  const spec = JSON.parse(await page.locator("#json").inputValue());
  spec.interaction = { orbit: false };
  await page.locator("#json").fill(JSON.stringify(spec));
  await page.locator("#apply").click();
  await expect(page.locator("html")).toHaveAttribute("data-updates", "2");
  const still = await pixels(page);
  await page.mouse.move(300, 300);
  await page.mouse.down();
  await page.mouse.move(420, 300, { steps: 4 });
  await page.mouse.up();
  expect((await camera(page)).azimuth).toBe(35);
  expect(changedPixels(still, await pixels(page))).toBe(0);
});

test("switching 3D to 2D and back works and keeps the orbited camera", async ({ page }) => {
  await page.setViewportSize({ width: 1200, height: 700 });
  await page.goto("/?backend=webgl2&example=bar3d-grid");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await page.mouse.move(300, 350);
  await page.mouse.down();
  await page.mouse.move(350, 350, { steps: 4 });
  await page.mouse.up();
  const turned = (await camera(page)).azimuth;
  expect(turned).not.toBe(35);
  // Re-applying the same spec keeps the user's camera (the spec's camera did not change).
  await page.locator("#apply").click();
  await expect(page.locator("html")).toHaveAttribute("data-updates", "1");
  expect((await camera(page)).azimuth).toBe(turned);
  await page.locator("#example").selectOption("bar-basic");
  await expect(page.locator("html")).toHaveAttribute("data-updates", "2");
  await expect(page.locator("#camera")).toBeHidden();
  await page.locator("#example").selectOption("bar3d-row");
  await expect(page.locator("html")).toHaveAttribute("data-updates", "3");
  await expect(page.locator("#camera")).toBeVisible();
});
