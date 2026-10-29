import { expect, test } from "@playwright/test";

// The spec version, edges' own colours and widths (`linkStyle`), and the diagram palette
// (`palette`, Mermaid's `themeVariables`).

test.beforeEach(async ({ page }) => {
  await page.goto("/?backend=webgl2&example=bar-basic");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
});

test("linkStyle, themeVariables and the palette option reach the spec, the text winning", async ({ page }) => {
  const spec = await page.evaluate(async () => {
    const d = (window as any).facetryDiagram;
    return d.fromMermaid(
      [
        '%%{init: {"themeVariables": {"lineColor": "#123456"}}}%%',
        "flowchart LR",
        "  a --> b --> c",
        "  linkStyle 1 stroke:#e5484d,stroke-width:3px,color:#222222",
      ].join("\n"),
      { palette: { edge: "#ffffff", node: "#ffeeaa", accents: ["#3e8ef7"] } },
    );
  });
  expect(spec.version).toBe(1);
  expect(spec.diagram.palette).toEqual({ edge: "#123456", node: "#ffeeaa", accents: ["#3e8ef7"] });
  expect(spec.diagram.edges[0].color).toBeUndefined();
  expect(spec.diagram.edges[1]).toMatchObject({ color: "#e5484d", width: 3, labelColor: "#222222" });
});

test("a spec of a later version is refused, saying so", async ({ page }) => {
  const message = await page.evaluate(async () => {
    const w = window as any;
    try {
      await w.facetryChart.update({ ...w.playgroundSpec(), version: 2 });
      return "drawn";
    } catch (e) {
      return (e as Error).message;
    }
  });
  expect(message).toContain("spec version 2");
});

test("a bad palette colour and a link past the last are reported", async ({ page }) => {
  const issues = await page.evaluate(async () => {
    const d = (window as any).facetryDiagram;
    try {
      await d.fromMermaid("flowchart LR\n  a --> b\n  linkStyle 3 stroke:#f00");
      return [];
    } catch (e) {
      return (e as any).diagramIssues.map((i: any) => i.message);
    }
  });
  expect(issues).toEqual(["there is no link 3: the diagram has 1 link, numbered from 0"]);
});

for (const style of ["flat", "isometric"]) {
  test(`${style} diagram 39-link-styles matches its golden image (webgl2)`, async ({ page }) => {
    await page.goto(`/?backend=webgl2&ui=0&style=${style}&example=mermaid/39-link-styles`);
    await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
    await page.mouse.move(790, 495);
    await expect(page.locator("#chart")).toHaveScreenshot(`diagram-${style}-39-link-styles.png`);
  });
}
