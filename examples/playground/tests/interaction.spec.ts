import { expect, test, type Page } from "@playwright/test";

// Moving around a 3D picture: pan (right-drag, Shift+drag, two fingers), zoom toward the
// pointer (wheel, pinch), and reset. Diagrams are where this matters most (a big diagram is
// read a corner at a time), so most tests use one.

test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1 });

type Camera = {
  type: string;
  azimuth: number;
  elevation: number;
  distance: number;
  pan: [number, number];
};

async function open(page: Page, query: string) {
  await page.goto(`/?backend=webgl2&ui=0&${query}`);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
}

const camera = (page: Page): Promise<Camera> => page.evaluate(() => (window as any).facetryChart.camera);
const position = (page: Page, key: string): Promise<{ x: number; y: number }> =>
  page.evaluate((k) => (window as any).facetryChart.markPosition(k), key);

test("right-drag and Shift+drag pan: the picture follows the pointer, and nothing clicks", async ({ page }) => {
  await open(page, "style=isometric&example=mermaid/11-microservices");
  await page.evaluate(() => {
    (window as any).__clicks = 0;
    (window as any).facetryChart.on("click", () => (window as any).__clicks++);
  });
  const before = await position(page, "orders");
  await page.mouse.move(400, 250);
  await page.mouse.down({ button: "right" });
  await page.mouse.move(460, 290, { steps: 6 });
  await page.mouse.up({ button: "right" });
  const pan = (await camera(page)).pan;
  expect(pan).not.toEqual([0, 0]);
  const after = await position(page, "orders");
  // The node moved with the pointer (60 right, 40 down).
  expect(after.x - before.x).toBeCloseTo(60, -1);
  expect(after.y - before.y).toBeCloseTo(40, -1);
  // The orbit did not change.
  expect((await camera(page)).azimuth).toBe(45);

  await page.keyboard.down("Shift");
  await page.mouse.move(400, 250);
  await page.mouse.down();
  await page.mouse.move(340, 250, { steps: 6 });
  await page.mouse.up();
  await page.keyboard.up("Shift");
  expect((await camera(page)).pan).not.toEqual(pan);
  expect((await camera(page)).azimuth).toBe(45);
  // A right click without a drag is no chart click either.
  await page.mouse.click(400, 250, { button: "right" });
  expect(await page.evaluate(() => (window as any).__clicks)).toBe(0);
});

test("the wheel zooms toward the pointer: what is under it stays under it", async ({ page }) => {
  await open(page, "style=isometric&example=mermaid/11-microservices");
  const target = await position(page, "payments");
  await page.mouse.move(target.x, target.y);
  for (let i = 0; i < 5; i++) await page.mouse.wheel(0, -200);
  await expect.poll(async () => (await camera(page)).distance).toBeLessThan(1);
  const after = await position(page, "payments");
  expect(Math.hypot(after.x - target.x, after.y - target.y)).toBeLessThan(6);
  // Diagrams may come much closer than charts (whose limit is 1.3 radii).
  for (let i = 0; i < 20; i++) await page.mouse.wheel(0, -400);
  await expect.poll(async () => (await camera(page)).distance).toBeLessThan(0.3);
  // Double-click goes back to the start, pan included.
  await page.mouse.dblclick(target.x, target.y);
  await expect.poll(async () => (await camera(page)).pan).toEqual([0, 0]);
});

test("two fingers pinch to zoom and move together to pan", async ({ browser }) => {
  const context = await browser.newContext({ hasTouch: true, viewport: { width: 800, height: 500 } });
  const page = await context.newPage();
  await open(page, "style=isometric&example=mermaid/11-microservices");
  const client = await context.newCDPSession(page);
  const touch = (type: "touchStart" | "touchMove" | "touchEnd", points: [number, number][]) =>
    client.send("Input.dispatchTouchEvent", {
      type,
      touchPoints: points.map(([x, y], id) => ({ x, y, id })),
    });
  const start = await camera(page);
  await touch("touchStart", [
    [350, 250],
    [450, 250],
  ]);
  for (let k = 1; k <= 8; k++) {
    // Fingers spread (zoom in) and drift right together (pan).
    await touch("touchMove", [
      [350 - k * 10 + k * 5, 250],
      [450 + k * 10 + k * 5, 250],
    ]);
  }
  await touch("touchEnd", []);
  const end = await camera(page);
  expect(end.distance).toBeLessThan(start.distance * 0.75);
  expect(end.pan).not.toEqual(start.pan);
  expect(end.azimuth).toBe(start.azimuth);
  await context.close();
});

test("charts pan too, and keep their closest distance", async ({ page }) => {
  await open(page, "example=bar3d-grid");
  await page.mouse.move(400, 250);
  await page.mouse.down({ button: "right" });
  await page.mouse.move(450, 250, { steps: 5 });
  await page.mouse.up({ button: "right" });
  expect((await camera(page)).pan).not.toEqual([0, 0]);
  await page.evaluate(() => (window as any).facetryChart.setCamera({ distance: 0.1 }));
  expect((await camera(page)).distance).toBe(1.3);
  // setCamera takes a pan, and keeps it inside the plot.
  await page.evaluate(() => (window as any).facetryChart.setCamera({ pan: [100, -100] }));
  const pan = (await camera(page)).pan;
  expect(Math.abs(pan[0])).toBeLessThan(2);
  expect(Math.abs(pan[1])).toBeLessThan(2);
});
