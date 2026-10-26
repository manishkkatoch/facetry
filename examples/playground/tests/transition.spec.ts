import { expect, test, type Page } from "@playwright/test";
import { PNG } from "pngjs";

// Animated transitions (2D bars, axes, text and legend). The tests hold the animation clock
// with `chart.__animationClock(ms)` so a frame at "200 ms into the transition" is exactly that,
// whatever the machine's speed; a few tests use the real clock to check that the loop runs and
// stops. Every other spec file runs with reduced motion (see playwright.config.ts), which makes
// updates jump; these opt back in.

test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1, reducedMotion: "no-preference" });
test.setTimeout(90_000);

type Spec = any;

async function open(page: Page, example: string, query = "") {
  await page.goto(`/?backend=webgl2&ui=0&example=${example}${query}`);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await page.mouse.move(790, 495); // not over a bar
}

const frames = (page: Page) =>
  page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));

async function shot(page: Page) {
  await frames(page);
  return PNG.sync.read(await page.locator("#chart").screenshot());
}

/** Holds the clock at `ms` into the running transition and takes a picture. */
async function at(page: Page, ms: number) {
  await page.evaluate((ms) => (window as any).facetryChart.__animationClock(ms), ms);
  return shot(page);
}

/** Applies `change` to a copy of the spec on screen and updates the chart with it. */
async function update(page: Page, change: string, options?: object): Promise<Spec> {
  return page.evaluate(
    ([change, options]) => {
      const w = window as any;
      const spec = w.playgroundSpec();
      new Function("spec", change)(spec);
      w.facetryChart.update(spec, options);
      return spec;
    },
    [change, options] as const,
  );
}

/** Pixels that differ by more than `tolerance` in any channel. */
function differing(a: PNG, b: PNG, tolerance = 8, box?: [number, number, number, number]) {
  const [x0, y0, x1, y1] = box ?? [0, 0, a.width, a.height];
  let n = 0;
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const i = (y * a.width + x) * 4;
      const d = Math.max(
        Math.abs(a.data[i]! - b.data[i]!),
        Math.abs(a.data[i + 1]! - b.data[i + 1]!),
        Math.abs(a.data[i + 2]! - b.data[i + 2]!),
      );
      if (d > tolerance) n++;
    }
  }
  return n;
}

/** Pixels within `tolerance` of the colour `want`. */
function count(png: PNG, want: number[], tolerance = 6) {
  let n = 0;
  for (let i = 0; i < png.data.length; i += 4) {
    if (want.every((v, k) => Math.abs(png.data[i + k]! - v) <= tolerance)) n++;
  }
  return n;
}

const BLUE = [0x4c, 0x78, 0xa8];
const addQ5 = `
  const t = spec.data.sales;
  for (const [p, v] of [["Hardware", 150], ["Software", 120], ["Services", 60]]) {
    t.quarter.push("Q5"); t.product.push(p); t.revenue.push(v);
  }`;
const dropQ2 = `
  const t = spec.data.sales;
  const keep = t.quarter.map((q) => q !== "Q2");
  for (const k of Object.keys(t)) t[k] = t[k].filter((_, i) => keep[i]);`;
// Different relative heights (so bars really move) and a taller scale (so the axis does too).
const reshape = `spec.data.sales.revenue = spec.data.sales.revenue.map((v, i) => (i % 2 ? v * 0.3 : v * 1.8));`;

test("an update animates and settles on exactly what a direct render shows", async ({ page }) => {
  await open(page, "bar-stacked");
  const start = await shot(page);
  await update(page, addQ5);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(true);
  const first = await at(page, 0);
  expect(differing(first, start, 2)).toBe(0); // the first frame is the old picture
  await at(page, 10_000); // past the end: the transition finishes
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(false);
  const settled = await shot(page);
  // The same spec laid out and drawn without animation.
  await update(page, addQ5, { transition: false });
  const direct = await shot(page);
  expect(differing(settled, direct, 0)).toBe(0);
});

const GOLDENS: [string, string][] = [
  // [example, example it comes from]
  ["bar-stacked", "bar-basic"],
  ["bar-grouped", "bar-stacked"],
  ["bar-labels", "bar-basic"],
  ["bar-stacked-hidden", "bar-stacked"],
  ["line-basic", "bar-basic"],
  ["line-numeric-x", "line-basic"],
  ["combo-bar-line", "bar-basic"],
  ["pie", "donut"],
  ["donut", "pie"],
  ["bar3d-grid", "bar3d-stacked"],
  ["bar3d-stacked", "bar3d-grid"],
  ["pie3d", "bar3d-grid"],
  ["line3d", "pie3d"],
  ["combo3d-bar-line", "line3d"],
];
for (const [name, from] of GOLDENS) {
  // The end of a transition is the golden image of that example, whatever it came from.
  test(`animating from ${from} to ${name} ends on its golden image`, async ({ page }) => {
    await open(page, from);
    await page.evaluate((name) => {
      const w = window as any;
      w.facetryChart.update(w.playgroundExample(name));
    }, name);
    await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
    await at(page, 10_000);
    await expect(page.locator("#chart")).toHaveScreenshot(`${name}.png`);
  });
}

test("a frame in the middle differs from both ends", async ({ page }) => {
  await open(page, "bar-basic");
  const start = await shot(page);
  await update(page, reshape);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  const mid = await at(page, 200);
  const end = await at(page, 10_000);
  expect(differing(mid, start)).toBeGreaterThan(2_000);
  expect(differing(mid, end)).toBeGreaterThan(2_000);
  expect(differing(start, end)).toBeGreaterThan(2_000);
});

/** The colour of the pixel at `(x, y)`. */
function pixel(png: PNG, x: number, y: number) {
  const i = (Math.round(y) * png.width + Math.round(x)) * 4;
  return [png.data[i]!, png.data[i + 1]!, png.data[i + 2]!];
}

/** The box of colour `want` around `(x, y)`: how far it extends in each direction. */
function boxAround(png: PNG, want: number[], x: number, y: number, tolerance = 6): [number, number, number, number] {
  const is = (px: number, py: number) => {
    const i = (py * png.width + px) * 4;
    return want.every((v, k) => Math.abs(png.data[i + k]! - v) <= tolerance);
  };
  let [x0, x1, y0, y1] = [Math.round(x), Math.round(x), Math.round(y), Math.round(y)];
  while (is(x0 - 1, Math.round(y))) x0--;
  while (is(x1 + 1, Math.round(y))) x1++;
  while (is(Math.round(x), y0 - 1)) y0--;
  while (is(Math.round(x), y1 + 1)) y1++;
  return [x0, y0, x1 + 1, y1 + 1];
}

test("a bar that is added grows from its baseline", async ({ page }) => {
  await open(page, "bar-colored");
  const lemon = `const t = spec.data.fruit; t.name.push("Lemon"); t.tons.push(3000);`;
  await update(page, lemon);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  // Where the new bar ends up, and its colour.
  const target = await page.evaluate(() => (window as any).facetryChart.markPosition("layer0/Lemon"));
  const done = await at(page, 10_000);
  const colour = pixel(done, target.x, target.y);
  const box = boxAround(done, colour, target.x, target.y);
  expect(box[3] - box[1]).toBeGreaterThan(100);
  // From the old picture, play it again and watch the box fill from the bottom.
  await update(page, "", { transition: false });
  await update(page, lemon);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  const filled = [];
  const tops = [];
  for (const ms of [0, 100, 200, 300, 10_000]) {
    const png = await at(page, ms);
    filled.push(count(cropped(png, box), colour));
    tops.push(boxAround(png, colour, target.x, box[3] - 3)[1]);
  }
  expect(filled[0]).toBe(0); // not there yet
  for (let i = 1; i < filled.length; i++) expect(filled[i]!).toBeGreaterThan(filled[i - 1]!);
  // The bar's bottom stays on the baseline while its top rises.
  for (let i = 2; i < tops.length; i++) expect(tops[i]!).toBeLessThan(tops[i - 1]!);
});

/** The part of `png` inside `box` as an image of its own. */
function cropped(png: PNG, box: [number, number, number, number]) {
  const [x0, y0, x1, y1] = box;
  const out = new PNG({ width: x1 - x0, height: y1 - y0 });
  PNG.bitblt(png, out, x0, y0, x1 - x0, y1 - y0, 0, 0);
  return out;
}

test("a bar that is removed shrinks away", async ({ page }) => {
  await open(page, "bar-colored");
  const start = await shot(page);
  const kiwi = await page.evaluate(() => (window as any).facetryChart.markPosition("layer0/Kiwi"));
  const colour = pixel(start, kiwi.x, kiwi.y);
  const box = boxAround(start, colour, kiwi.x, kiwi.y);
  await update(
    page,
    `const t = spec.data.fruit;
     for (const k of Object.keys(t)) t[k] = t[k].slice(0, -1);`,
  );
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  const areas = [];
  for (const ms of [0, 130, 260, 10_000]) areas.push(count(cropped(await at(page, ms), box), colour));
  expect(areas[0]).toBeGreaterThan(3_000);
  expect(areas[1]!).toBeLessThan(areas[0]!);
  expect(areas[2]!).toBeLessThan(areas[1]!);
  expect(areas[3]).toBe(0);
});

test("an update in the middle of a transition continues from where the marks are", async ({ page }) => {
  await open(page, "bar-stacked");
  await update(page, addQ5);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  const before = await at(page, 180);
  // Interrupt with a different change. Its first frame must be what was on screen.
  const second = `
    const t = spec.data.sales;
    t.revenue = t.revenue.map((v) => v * 0.6);`;
  await update(page, second);
  const after = await at(page, 0);
  expect(differing(before, after, 12)).toBeLessThan(60);
  // It is a real interruption: the picture then moves on...
  expect(differing(after, await at(page, 200), 12)).toBeGreaterThan(1_000);
  // ...and it ends on the final picture of the second update.
  const end = await at(page, 10_000);
  await update(page, second, { transition: false });
  expect(differing(end, await shot(page), 0)).toBe(0);
});

test("transition: false (in the spec or in the call) and reduced motion are instant", async ({ page }) => {
  await open(page, "bar-stacked");
  const animating = () => page.evaluate(() => (window as any).facetryChart.animating);
  const bump = (by: number, extra = "") =>
    `${extra} spec.data.sales.revenue = spec.data.sales.revenue.map((v) => v + ${by});`;
  await update(page, addQ5, { transition: false });
  expect(await animating()).toBe(false);
  await update(page, bump(5, "spec.transition = false;"));
  expect(await animating()).toBe(false);
  // A zero duration is no animation either.
  await update(page, bump(10, "spec.transition = { duration: 0 };"));
  expect(await animating()).toBe(false);
  // The call's option beats the spec's.
  await update(page, bump(15, "spec.transition = false;"), { transition: { duration: 300 } });
  expect(await animating()).toBe(true);
  await update(page, bump(20, "spec.transition = { duration: 300 };"), { transition: false });
  expect(await animating()).toBe(false);
  // Reduced motion: every transition is a jump, and one that is running ends when it is turned on.
  await update(page, bump(25), { transition: { duration: 5000 } });
  expect(await animating()).toBe(true);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect.poll(animating).toBe(false);
  await update(page, bump(30, "spec.transition = { duration: 800 };"));
  expect(await animating()).toBe(false);
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await update(page, bump(35, "spec.transition = { duration: 800 };"));
  expect(await animating()).toBe(true);
});

test("a changing viewer preference ends a running transition at once", async ({ page }) => {
  await open(page, "bar-stacked");
  await update(page, addQ5, { transition: { duration: 5000 } });
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(true);
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect.poll(() => page.evaluate(() => (window as any).facetryChart.animating)).toBe(false);
});

test("rendering runs while a transition does and stops when it ends", async ({ page }) => {
  await open(page, "bar-stacked");
  const stats = () =>
    page.evaluate(() => {
      const c = (window as any).facetryChart;
      return { frames: c.renderCount as number, animating: c.animating as boolean };
    });
  await frames(page);
  const idle = (await stats()).frames;
  await frames(page);
  expect((await stats()).frames).toBe(idle); // an idle chart draws nothing
  await update(page, addQ5, { transition: { duration: 500 } });
  await expect.poll(async () => (await stats()).animating).toBe(false);
  const done = await stats();
  // Many frames were drawn during half a second (a software GL machine may manage fewer).
  expect(done.frames - idle).toBeGreaterThan(2);
  await page.waitForTimeout(500);
  expect((await stats()).frames).toBe(done.frames);
});

test("the real clock reaches the end by itself, and a stagger makes it last longer", async ({ page }) => {
  await open(page, "bar-stacked");
  const animating = () => page.evaluate(() => (window as any).facetryChart.animating);
  const t0 = Date.now();
  await update(page, addQ5, { transition: { duration: 300, stagger: 600 } });
  expect(await animating()).toBe(true);
  await page.waitForTimeout(500);
  expect(await animating()).toBe(true); // 300 + 600 ms in total
  await expect.poll(animating, { timeout: 5000 }).toBe(false);
  expect(Date.now() - t0).toBeGreaterThan(850);
});

test("stagger starts the bars one after another", async ({ page }) => {
  await open(page, "bar-basic");
  const doubled = (t: object) => update(page, reshape, { transition: t });
  await doubled({ duration: 400, easing: "linear" });
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  const together = await at(page, 200);
  await at(page, 10_000);
  await update(page, "", { transition: false });
  await doubled({ duration: 400, easing: "linear", stagger: 400 });
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  const staggered = await at(page, 200);
  expect(differing(together, staggered)).toBeGreaterThan(1_000);
  // Everything still lands in the same place.
  const end = await at(page, 10_000);
  await update(page, reshape, { transition: false });
  expect(differing(end, await shot(page), 0)).toBe(0);
});

test("every easing starts and ends in the same place", async ({ page }) => {
  await open(page, "bar-basic");
  const start = await shot(page);
  let reference: PNG | null = null;
  for (const easing of ["linear", "cubicInOut", "cubicOut", "backOut"]) {
    await update(page, reshape, { transition: { duration: 300, easing } });
    await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
    expect(differing(await at(page, 0), start, 2)).toBe(0);
    const end = await at(page, 10_000);
    reference ??= end;
    expect(differing(end, reference, 0)).toBe(0);
    await update(page, "", { transition: false });
    expect(differing(await shot(page), start, 0)).toBe(0);
  }
});

test("axis ticks slide with the scale: new ones come from where the old scale would put them", async ({ page }) => {
  await open(page, "bar-basic");
  await update(page, reshape);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  const grey = (png: PNG) => {
    // Gridline pixels: the faint grey the theme draws them in.
    let rows = new Set<number>();
    for (let y = 0; y < png.height; y++) {
      const i = (y * png.width + 400) * 4;
      const [r, g, b] = [png.data[i]!, png.data[i + 1]!, png.data[i + 2]!];
      if (r > 200 && r < 245 && Math.abs(r - g) < 4 && Math.abs(g - b) < 4) rows.add(y);
    }
    return [...rows];
  };
  const a = grey(await at(page, 0));
  const b = grey(await at(page, 200));
  const c = grey(await at(page, 10_000));
  // The gridlines are in different places at each moment: the scale is moving.
  expect(JSON.stringify(a)).not.toBe(JSON.stringify(b));
  expect(JSON.stringify(b)).not.toBe(JSON.stringify(c));
});

test("an invalid transition is reported with the path of the problem", async ({ page }) => {
  await open(page, "bar-basic");
  const result = await page.evaluate(() => {
    const w = window as any;
    const issues = (spec: any) => {
      try {
        w.facetryChart.update(spec);
        return null;
      } catch (e: any) {
        return { code: e.code, issues: e.issues };
      }
    };
    const base = () => w.playgroundSpec();
    const out: Record<string, unknown> = {};
    let s = base();
    s.transition = { duration: -5 };
    out.negative = issues(s);
    s = base();
    s.transition = { easing: "bounce" };
    out.easing = issues(s);
    s = base();
    s.transition = { stagger: -1, duration: 5 };
    out.stagger = issues(s);
    s = base();
    s.transition = { speed: 1 };
    out.unknown = issues(s);
    try {
      w.facetryChart.update(base(), { transition: { duration: -1 } });
    } catch (e: any) {
      out.option = { code: e.code, message: e.message };
    }
    try {
      w.facetryChart.update(base(), { transition: { easing: "bounce" } });
    } catch (e: any) {
      out.optionEasing = { code: e.code, message: e.message };
    }
    return out;
  });
  expect(result.negative).toMatchObject({ code: "invalid-spec", issues: [{ path: "/transition/duration" }] });
  expect(result.easing).toMatchObject({ code: "invalid-spec", issues: [{ path: "/transition/easing" }] });
  expect(result.stagger).toMatchObject({ code: "invalid-spec", issues: [{ path: "/transition/stagger" }] });
  expect(result.unknown).toMatchObject({ code: "invalid-spec", issues: [{ path: "/transition/speed" }] });
  expect(result.option).toMatchObject({ code: "invalid-options" });
  expect((result.option as any).message).toContain("/duration");
  expect((result.optionEasing as any).message).toContain("/easing");
});

test("hover and click during a transition address the target state", async ({ page }) => {
  await open(page, "bar-stacked");
  await update(page, addQ5);
  await page.evaluate(() => {
    const w = window as any;
    w.__hover = [];
    w.facetryChart.on("hover", (m: any) => w.__hover.push(m?.key ?? null));
    w.facetryChart.__animationClock(0);
  });
  // The new bar is still flat on the baseline, but its target is where hover looks.
  const pos = await page.evaluate(() => (window as any).facetryChart.markPosition("layer0/Q5/Hardware"));
  expect(pos).toBeTruthy();
  await page.mouse.move(pos.x, pos.y);
  await expect.poll(() => page.evaluate(() => (window as any).__hover.at(-1))).toBe("layer0/Q5/Hardware");
});

test("a snapshot taken during a transition shows the target state, and the animation goes on", async ({ page }) => {
  await open(page, "bar-stacked");
  await update(page, addQ5);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(100));
  const grab = () =>
    page.evaluate(async () => {
      const blob: Blob = await (window as any).facetryChart.snapshot({ scale: 1 });
      return Array.from(new Uint8Array(await blob.arrayBuffer()));
    });
  const during = PNG.sync.read(Buffer.from(await grab()));
  await at(page, 100); // the canvas itself is still mid-transition
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(true);
  await at(page, 10_000);
  const after = PNG.sync.read(Buffer.from(await grab()));
  expect(differing(during, after, 0)).toBe(0);
});

test("a resize during a transition lays out again and carries on", async ({ page }) => {
  await open(page, "bar-stacked");
  await update(page, addQ5);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(150));
  await page.setViewportSize({ width: 600, height: 400 });
  await expect
    .poll(async () => (await page.evaluate(() => (window as any).facetryChart.animating)) === true)
    .toBe(true);
  await page.waitForTimeout(100);
  await at(page, 10_000);
  const settled = await shot(page);
  await update(page, addQ5, { transition: false });
  expect(differing(settled, await shot(page), 0)).toBe(0);
});

test("animateIn makes the first frame a transition from nothing", async ({ page }) => {
  await open(page, "bar-stacked", "&animateIn=1&clock=manual");
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(true);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  const first = await at(page, 0);
  expect(count(first, BLUE)).toBe(0); // nothing yet
  const end = await at(page, 10_000);
  expect(count(end, BLUE)).toBeGreaterThan(10_000);
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(false);
  // And without the option the first frame is simply there.
  await open(page, "bar-stacked");
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(false);
});

// --- lines, pies and 3D ------------------------------------------------------------------------

const ORANGE = [0xf5, 0x85, 0x18];

/** The rightmost x of pixels close to `want` (within the plot, left of the legend). */
function rightmost(png: PNG, want: number[], tolerance = 8) {
  let best = -1;
  for (let y = 0; y < png.height; y++) {
    for (let x = 0; x < 640; x++) {
      const i = (y * png.width + x) * 4;
      if (want.every((v, k) => Math.abs(png.data[i + k]! - v) <= tolerance)) best = Math.max(best, x);
    }
  }
  return best;
}

test("a new line series is drawn on from left to right and an old one fades out", async ({ page }) => {
  await open(page, "line-basic");
  // Remove "Pro": the orange line leaves; then bring it back: it is drawn on.
  const without = `
    const t = spec.data.traffic;
    const keep = t.plan.map((p) => p === "Free");
    for (const k of Object.keys(t)) t[k] = t[k].filter((_, i) => keep[i]);`;
  const before = await shot(page);
  await update(page, without);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  // How orange the plot is: the red-over-blue excess of its pixels (grey stays 0, the blue
  // line is negative). A fading orange line adds less and less.
  const orangeness = (png: PNG) => {
    let sum = 0;
    for (let y = 0; y < png.height; y++) {
      for (let x = 0; x < 640; x++) {
        const i = (y * png.width + x) * 4;
        sum += Math.max(0, png.data[i]! - png.data[i + 2]! - 6);
      }
    }
    return sum;
  };
  const counts = [];
  for (const ms of [0, 130, 260, 399, 10_000]) counts.push(orangeness(await at(page, ms)));
  expect(counts[0]).toBeGreaterThan(5_000);
  for (let i = 1; i < 4; i++) expect(counts[i]!).toBeLessThan(counts[i - 1]!);
  expect(counts[4]).toBe(0);
  // Back: drawn on from the left, so how far right it reaches grows.
  await update(page, "", { transition: false });
  await update(page, without, { transition: false });
  await update(page, "", { transition: { duration: 400, easing: "linear" } });
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  const reach = [];
  for (const ms of [40, 160, 280, 10_000]) reach.push(rightmost(await at(page, ms), ORANGE));
  for (let i = 1; i < reach.length; i++) expect(reach[i]!).toBeGreaterThan(reach[i - 1]!);
  expect(reach[0]!).toBeLessThan(reach[3]! - 150);
  expect(before).toBeTruthy();
});

test("markers stay on their line while a point is added to it", async ({ page }) => {
  await open(page, "line-basic");
  await update(
    page,
    `const t = spec.data.traffic;
     for (const [plan, v] of [["Free", 40], ["Pro", 31]]) { t.month.push("Jul"); t.plan.push(plan); t.visits.push(v); }`,
  );
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  // Markers have a rim of background colour round a fill of the line colour: the fill is
  // within the line's stroke. Check that at mid-flight every blue marker centre has line pixels
  // of the same colour on both sides (it is on a line, not floating).
  const BLUE_LINE = [0x4c, 0x78, 0xa8];
  const png = await at(page, 180);
  const solid = (x: number, y: number) => {
    const i = (y * png.width + x) * 4;
    return BLUE_LINE.every((v, k) => Math.abs(png.data[i + k]! - v) <= 10);
  };
  // Each column of the plot with blue pixels has them in one connected vertical run or on a
  // line: a floating marker would be an isolated blob far from any line pixel. Count blobs of
  // blue that touch no other blue pixel within 12 px horizontally.
  let isolated = 0;
  for (let y = 4; y < png.height - 4; y++) {
    for (let x = 60; x < 600; x++) {
      if (!solid(x, y)) continue;
      let near = false;
      for (let dx = -12; dx <= 12 && !near; dx += 4) {
        if (dx !== 0 && (solid(x + dx, y) || solid(x + dx, y - 3) || solid(x + dx, y + 3) || solid(x + dx, y - 8) || solid(x + dx, y + 8))) near = true;
      }
      if (!near) isolated++;
    }
  }
  expect(isolated).toBeLessThan(40);
});

test("slices of a donut open and close without overlapping", async ({ page }) => {
  await open(page, "donut");
  const start = await shot(page);
  await update(page, `spec.data.plans = { plan: ["Free", "Team", "Startup", "Enterprise"], users: [200, 420, 260, 90] };`);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  const mid = await at(page, 200);
  const end = await at(page, 10_000);
  expect(differing(mid, start)).toBeGreaterThan(3_000);
  expect(differing(mid, end)).toBeGreaterThan(3_000);
  await update(page, `spec.data.plans = { plan: ["Free", "Team", "Startup", "Enterprise"], users: [200, 420, 260, 90] };`, { transition: false });
  expect(differing(end, await shot(page), 0)).toBe(0);
});

test("a 3D update animates: a box grows from the floor and the first frame is the old picture", async ({ page }) => {
  await open(page, "bar3d-stacked");
  const start = await shot(page);
  await update(
    page,
    `const t = spec.data.sales;
     const keys = Object.keys(t);
     const strs = keys.filter((k) => typeof t[k][0] === "string");
     const num = keys.find((k) => typeof t[k][0] === "number");
     const seen = new Set();
     const rows = [];
     for (let i = 0; i < t[num].length; i++) { const id = strs.slice(1).map((k) => t[k][i]).join("|"); if (!seen.has(id)) { seen.add(id); rows.push(i); } }
     for (const i of rows) for (const k of keys) t[k].push(k === strs[0] ? "Q5" : k === num ? 90 : t[k][i]);`,
  );
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  // A box that is still flat is not drawn (no plate on the floor): the first frame is exactly
  // the old picture.
  expect(differing(await at(page, 0), start, 2)).toBe(0);
  const mid = await at(page, 200);
  const end = await at(page, 10_000);
  expect(differing(mid, start)).toBeGreaterThan(3_000);
  expect(differing(mid, end)).toBeGreaterThan(3_000);
  await update(
    page,
    `const t = spec.data.sales;
     const keys = Object.keys(t);
     const strs = keys.filter((k) => typeof t[k][0] === "string");
     const num = keys.find((k) => typeof t[k][0] === "number");
     const seen = new Set();
     const rows = [];
     for (let i = 0; i < t[num].length; i++) { const id = strs.slice(1).map((k) => t[k][i]).join("|"); if (!seen.has(id)) { seen.add(id); rows.push(i); } }
     for (const i of rows) for (const k of keys) t[k].push(k === strs[0] ? "Q5" : k === num ? 90 : t[k][i]);`,
    { transition: false },
  );
  expect(differing(end, await shot(page), 0)).toBe(0);
});

test("an update that interrupts a 3D transition continues from where the picture is", async ({ page }) => {
  await open(page, "bar3d-grid");
  await update(page, `const t = spec.data.sales; t.revenue = t.revenue.map((v, i) => (i % 2 ? v * 2 : v * 0.3));`);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  const before = await at(page, 170);
  await update(page, `const t = spec.data.sales; t.revenue = t.revenue.map((v, i) => (i % 3 ? v * 0.6 : v * 1.4));`);
  const after = await at(page, 0);
  expect(differing(before, after, 12)).toBeLessThan(80);
  expect(differing(after, await at(page, 200), 12)).toBeGreaterThan(1_500);
});

test("the legend and the axes of a 3D chart animate too, and the camera is kept", async ({ page }) => {
  await open(page, "bar3d-grid");
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 80, elevation: 30 }));
  const cam = await page.evaluate(() => (window as any).facetryChart.camera);
  await update(page, `const t = spec.data.sales; t.revenue = t.revenue.map((v) => v * 3);`);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(0));
  await at(page, 150);
  expect(await page.evaluate(() => (window as any).facetryChart.camera)).toEqual(cam);
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(true);
  await at(page, 10_000);
  expect(await page.evaluate(() => (window as any).facetryChart.camera)).toEqual(cam);
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(false);
});

test("switching between 2D and 3D jumps", async ({ page }) => {
  await open(page, "bar-basic");
  await page.evaluate(() => {
    const w = (window as any);
    w.facetryChart.update(w.playgroundExample("bar3d-grid"));
  });
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(false);
});

test("hover on a 3D box works while it is still growing", async ({ page }) => {
  await open(page, "bar3d-grid");
  await update(page, `const t = spec.data.sales; t.revenue = t.revenue.map((v) => v * 1.5);`);
  await page.evaluate(() => {
    const w = window as any;
    w.__hover = [];
    w.facetryChart.on("hover", (m: any) => w.__hover.push(m?.key ?? null));
    w.facetryChart.__animationClock(0);
  });
  const key = await page.evaluate(() => {
    const w = window as any;
    return w.playgroundSpec().data.sales.quarter.length ? "layer0/Q1/North" : "";
  });
  const pos = await page.evaluate((key) => (window as any).facetryChart.markPosition(key), key);
  expect(pos).toBeTruthy();
  await page.mouse.move(pos.x, pos.y);
  await expect.poll(() => page.evaluate(() => (window as any).__hover.at(-1))).toBe(key);
});

test("animateIn works in 3D too: the boxes grow, the camera framing is final from the first frame", async ({ page }) => {
  await open(page, "bar3d-stacked", "&animateIn=1&clock=manual");
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(true);
  const first = await at(page, 0);
  const mid = await at(page, 200);
  const end = await at(page, 10_000);
  // Nothing of the bars at the start (a pixel is a box if it is not background, grid or text:
  // count saturated pixels).
  const saturated = (png: PNG) => {
    let n = 0;
    for (let i = 0; i < png.data.length; i += 4) {
      const [r, g, b] = [png.data[i]!, png.data[i + 1]!, png.data[i + 2]!];
      if (Math.max(r, g, b) - Math.min(r, g, b) > 60) n++;
    }
    return n;
  };
  // (The legend's swatches are saturated too, hence not 0.)
  expect(saturated(first)).toBeLessThan(600);
  expect(saturated(mid)).toBeGreaterThan(saturated(first) + 2_000);
  expect(saturated(end)).toBeGreaterThan(saturated(mid));
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(false);
});

test("3D: a snapshot during a transition shows the target, and a resize carries the transition on", async ({ page }) => {
  await open(page, "bar3d-stacked");
  await update(page, `const t = spec.data.sales; t.revenue = t.revenue.map((v, i) => (i % 2 ? v * 2 : v * 0.4));`);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(120));
  const grab = () =>
    page.evaluate(async () => {
      const blob: Blob = await (window as any).facetryChart.snapshot({ scale: 1 });
      return Array.from(new Uint8Array(await blob.arrayBuffer()));
    });
  const during = PNG.sync.read(Buffer.from(await grab()));
  expect(await page.evaluate(() => (window as any).facetryChart.animating)).toBe(true);
  await at(page, 10_000);
  const after = PNG.sync.read(Buffer.from(await grab()));
  expect(differing(during, after, 0)).toBe(0);

  // Resize in the middle of another one.
  await update(page, `const t = spec.data.sales; t.revenue = t.revenue.map((v) => v * 0.7);`);
  await page.evaluate(() => (window as any).facetryChart.__animationClock(150));
  await page.setViewportSize({ width: 620, height: 420 });
  await page.waitForTimeout(150);
  await at(page, 10_000);
  const settled = await shot(page);
  await update(page, `const t = spec.data.sales; t.revenue = t.revenue.map((v) => v * 0.7);`, { transition: false });
  expect(differing(settled, await shot(page), 0)).toBe(0);
});
