import { expect, test, type Page } from "@playwright/test";
import { PNG } from "pngjs";

// examples/playground/run.html: <pre class="mermaid"> blocks drawn by `diagram.run()`.

/** How many pixels of the element's screenshot are not the white page. */
async function inked(page: Page, selector: string): Promise<number> {
  const png = PNG.sync.read(await page.locator(selector).screenshot());
  let n = 0;
  for (let i = 0; i < png.data.length; i += 4) {
    if (png.data[i]! < 200 || png.data[i + 1]! < 200 || png.data[i + 2]! < 200) n++;
  }
  return n;
}

test.beforeEach(async ({ page }) => {
  await page.goto("/run.html?backend=webgl2");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
});

test("run draws every Mermaid block in its place, each in its own style", async ({ page }) => {
  expect(await page.evaluate(() => (window as any).facetryCharts.length)).toBe(2);
  for (const id of ["flat", "schematic"]) {
    const canvas = page.locator(`#${id} > canvas`);
    await expect(canvas).toHaveCount(1);
    await expect(page.locator(`#${id}`)).toHaveAttribute("data-processed", "true");
    // As wide as the block, as tall as asked, and something is drawn on it.
    const box = (await canvas.boundingBox())!;
    expect(box.height).toBe(320);
    expect(box.width).toBeGreaterThan(700);
    expect(await inked(page, `#${id} > canvas`)).toBeGreaterThan(500);
    // The Mermaid text is the canvas's text alternative.
    await expect(canvas).toHaveAttribute("aria-label", /^flowchart LR\n/);
  }
  // The schematic block's data-style: it starts in the isometric view (the flat style is seen square on).
  const camera = await page.evaluate(() => (window as any).facetryCharts[1].camera);
  expect(camera.azimuth).toBe(45);
});

test("a broken block shows its message in its place, and other elements are left alone", async ({ page }) => {
  await expect(page.locator("#broken [role=alert]")).toHaveText(
    "Diagram error:\nline 2, column 9: expected a node after the link",
  );
  await expect(page.locator("#broken canvas")).toHaveCount(0);
  await expect(page.locator("#plain")).toHaveText("Not a diagram: no class, left alone.");
  await expect(page.locator("#plain")).not.toHaveAttribute("data-processed");
});

test("running again draws only new blocks, and rejects with the first failure", async ({ page }) => {
  const result = await page.evaluate(async () => {
    const add = (id: string, text: string) => {
      const pre = document.createElement("pre");
      pre.className = "mermaid";
      pre.id = id;
      pre.textContent = text;
      document.body.append(pre);
    };
    add("later", "flowchart TB\n  x --> y");
    add("wrong", "flowchart TB\n  x[");
    try {
      await (window as any).facetryDiagram.run({ chart: { backend: "webgl2" } });
      return { rejected: false };
    } catch (e: any) {
      return { rejected: true, code: e.code, issues: e.diagramIssues.length };
    }
  });
  expect(result).toEqual({ rejected: true, code: "invalid-diagram", issues: 1 });
  // The good new block was still drawn; the old ones were not drawn twice.
  await expect(page.locator("#later > canvas")).toHaveCount(1);
  await expect(page.locator("#wrong [role=alert]")).toHaveCount(1);
  await expect(page.locator("#flat > canvas")).toHaveCount(1);
  // Nothing left to do.
  expect(await page.evaluate(async () => (await (window as any).facetryDiagram.run()).length)).toBe(0);
});
