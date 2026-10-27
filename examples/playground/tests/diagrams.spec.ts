import { expect, test, type Page } from "@playwright/test";

// Mermaid diagrams through `facetry/diagrams`: the playground lists the corpus in
// examples/diagrams as `mermaid/<name>` and lays the text out with the add-on module.

test.use({ viewport: { width: 1000, height: 640 }, deviceScaleFactor: 1 });

type Mark = { key: string; layer: number; datum: Record<string, number | string | null> } | null;

async function open(page: Page, example: string) {
  await page.goto(`/?backend=webgl2&example=${example}`);
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
}

/** Page coordinates of a node's top face, from the chart's own projection. */
async function where(page: Page, key: string) {
  const at = await page.evaluate((k) => (window as any).facetryChart.markPosition(k), key);
  expect(at, `no node ${key}`).toBeTruthy();
  const box = (await page.locator("#chart").boundingBox())!;
  return { x: box.x + at.x, y: box.y + at.y };
}

test("a Mermaid flowchart starts orthographic and square on, and its nodes report hover and click", async ({
  page,
}) => {
  await open(page, "mermaid/01-decision");
  const camera = await page.evaluate(() => (window as any).facetryChart.camera);
  expect(camera.type).toBe("orthographic");
  expect(camera.azimuth).toBe(0);
  expect(camera.elevation).toBe(50);

  await page.evaluate(() => {
    const log: { type: string; mark: Mark }[] = [];
    const chart = (window as any).facetryChart;
    chart.on("hover", (mark: Mark) => log.push({ type: "hover", mark }));
    chart.on("click", (mark: Mark) => log.push({ type: "click", mark }));
    (window as any).__log = log;
  });
  const debug = await where(page, "D");
  await page.mouse.move(debug.x, debug.y);
  await page.mouse.click(debug.x, debug.y);
  const log = await page.evaluate(() => (window as any).__log);
  const node = { key: "D", layer: -1, datum: { id: "D", label: "Debug", shape: "rect" } };
  expect(log).toEqual([
    { type: "hover", mark: node },
    { type: "click", mark: node },
  ]);
  // Every node can be found, after orbiting too.
  await page.evaluate(() => (window as any).facetryChart.setCamera({ azimuth: 40, elevation: 70 }));
  for (const key of ["A", "B", "C", "D"]) {
    const p = await where(page, key);
    await page.mouse.move(p.x, p.y);
    await expect
      .poll(() => page.evaluate(() => (window as any).__log.at(-1)?.mark?.key))
      .toBe(key);
  }
});

test("Mermaid problems are listed with their line and column, and the chart keeps its diagram", async ({
  page,
}) => {
  await open(page, "mermaid/01-decision");
  await page.locator("#json").fill("flowchart TD\n  A --> B\n  subgraph one\n  C[[sub]]\n");
  await page.locator("#apply").click();
  const issues = page.locator("#issues li");
  await expect(issues).toHaveCount(2);
  await expect(issues.nth(0)).toContainText("line 3:3");
  await expect(issues.nth(0)).toContainText("subgraphs are not supported");
  await expect(issues.nth(1)).toContainText("line 4:4");
  await expect(issues.nth(1)).toContainText("subroutine");
  // Still the old diagram.
  expect(await page.evaluate(() => (window as any).facetryChart.markPosition("D"))).toBeTruthy();
  // Fixing the text draws the new one.
  await page.locator("#json").fill("flowchart LR\n  x[One] --> y[Two]");
  await page.locator("#apply").click();
  await expect(page.locator("#issues li")).toHaveCount(0);
  await expect
    .poll(() => page.evaluate(() => (window as any).facetryChart.markPosition("y")))
    .toBeTruthy();
});

test("the diagrams module is only downloaded by pages that lay out a diagram", async ({ page }) => {
  const urls: string[] = [];
  page.on("request", (r) => urls.push(new URL(r.url()).pathname));
  const diagrams = (u: string) => /facetry_diagrams/.test(u);
  await page.goto("/?backend=webgl2&ui=0&example=bar-basic");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  expect(urls.filter(diagrams)).toEqual([]);

  urls.length = 0;
  await page.goto("/?backend=webgl2&ui=0&example=mermaid/02-ci-pipeline");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  expect(urls.filter((u) => /facetry_diagrams_bg-.*\.wasm/.test(u))).toHaveLength(1);
});

test("the glossy style starts in perspective, and its slabs report hover like the flat ones", async ({
  page,
}) => {
  await page.goto("/?backend=webgl2&style=glossy&example=mermaid/26-styled");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  const camera = await page.evaluate(() => (window as any).facetryChart.camera);
  expect(camera.type).toBe("perspective");
  expect(camera.azimuth).toBeGreaterThan(0);
  const p = await where(page, "backorder");
  await page.mouse.move(p.x, p.y);
  await expect(page.locator("#hover")).toContainText("backorder");
  await expect(page.locator("#hover")).toContainText('"label":"Backorder"');
});

test("changing the style lays the Mermaid text out again", async ({ page }) => {
  await open(page, "mermaid/01-decision");
  const before = await page.evaluate(() => (window as any).facetryChart.markPosition("D"));
  await page.locator("#style").selectOption("glossy");
  await expect
    .poll(() => page.evaluate(() => (window as any).facetryChart.camera.type))
    .toBe("perspective");
  const after = await page.evaluate(() => (window as any).facetryChart.markPosition("D"));
  expect(after).not.toEqual(before);
  await page.locator("#style").selectOption("flat");
  await expect
    .poll(() => page.evaluate(() => (window as any).facetryChart.camera.type))
    .toBe("orthographic");
});

test("the isometric style starts in the isometric view, and its pedestals report hover", async ({ page }) => {
  await page.goto("/?backend=webgl2&style=isometric&example=mermaid/27-timeline");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  const camera = await page.evaluate(() => (window as any).facetryChart.camera);
  expect(camera.type).toBe("orthographic");
  expect(camera.azimuth).toBe(45);
  const p = await where(page, "exec");
  await page.mouse.move(p.x, p.y);
  await expect(page.locator("#hover")).toContainText("exec");
  await expect(page.locator("#hover")).toContainText('"shape":"circle"');
});

test("the Cards box lays a card beside each node and numbers the nodes", async ({ page }) => {
  await page.goto("/?backend=webgl2&style=isometric&example=mermaid/28-roadmap");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  const before = await where(page, "plan");
  await page.locator("#cards").check();
  // The node now shows its number, and the layout made room for the cards.
  await expect
    .poll(async () => {
      const after = await page.evaluate(() => (window as any).facetryChart.markPosition("plan"));
      return Math.hypot(after.x - before.x, after.y - before.y);
    })
    .toBeGreaterThan(5);
  const p = await where(page, "plan");
  await page.mouse.move(p.x, p.y);
  await expect(page.locator("#hover")).toContainText('"label":"2"');
});

test("the schematic style starts in the isometric view, and its equipment reports hover", async ({ page }) => {
  await page.goto("/?backend=webgl2&style=schematic&example=mermaid/30-plant");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  const camera = await page.evaluate(() => (window as any).facetryChart.camera);
  expect(camera.type).toBe("orthographic");
  expect(camera.azimuth).toBe(45);
  await expect(page.locator("#style")).toHaveValue("schematic");
  const p = await where(page, "kiln");
  await page.mouse.move(p.x, p.y);
  await expect(page.locator("#hover")).toContainText("kiln");
  await expect(page.locator("#hover")).toContainText('"shape":"stadium"');
});
