import { expect, test } from "@playwright/test";

// Blocks of the user's own (`diagram.registerBlocks`, `blocks`): `@{ shape: "<name>" }` draws a
// block in place of the schematic style's equipment and the isometric style's pedestal.

const pump = {
  parts: [
    { name: "skid", from: 0, to: 0.5, color: "frame" },
    { name: "casing", shape: "circle", size: [0.6, 0.7], from: 0.5, to: 2, color: "steel" },
  ],
};

test.beforeEach(async ({ page }) => {
  await page.goto("/?backend=webgl2&example=bar-basic");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
});

test("registered and given blocks are named by @{ shape }, and only those used travel", async ({ page }) => {
  const result = await page.evaluate(async (pump) => {
    const d = (window as any).facetryDiagram;
    d.registerBlocks({ pump, mixer: { parts: [{ from: 0, to: 3, shape: "circle" }] } });
    const spec = await d.fromMermaid(
      "flowchart LR\n  a[Feed]@{ shape: pump } --> b@{ shape: cyl, label: \"Store\" } --> c@{ shape: valve2 }",
      { style: "schematic", blocks: { valve2: { parts: [{ from: 0, to: 1 }] } } },
    );
    return {
      names: d.blockNames(),
      blocks: Object.keys(spec.diagram.blocks).sort(),
      nodes: spec.diagram.nodes.map((n: any) => [n.id, n.block ?? null, n.shape ?? "rect", n.label ?? n.id]),
    };
  }, pump);
  expect(result.names).toEqual(["pump", "mixer"]);
  expect(result.blocks).toEqual(["pump", "valve2"]);
  expect(result.nodes).toEqual([
    ["a", "pump", "rect", "Feed"],
    ["b", null, "cylinder", "Store"],
    ["c", "valve2", "rect", "c"],
  ]);
});

test("an unknown shape names Mermaid's and the blocks given; a bad block is an invalid spec", async ({ page }) => {
  const result = await page.evaluate(async (pump) => {
    const d = (window as any).facetryDiagram;
    const fail = async (f: () => Promise<unknown>) => {
      try {
        await f();
        return null;
      } catch (e: any) {
        return {
          code: e.code,
          message: e.diagramIssues?.[0]?.message ?? e.issues?.[0]?.message ?? e.message,
          path: e.issues?.[0]?.path,
        };
      }
    };
    const unknown = await fail(() => d.fromMermaid("flowchart LR\n  a@{ shape: pumpp }", { blocks: { pump } }));
    const name = await fail(async () => d.registerBlocks({ "my pump": pump }));
    // A block with its heights the wrong way round lays out, and is refused when drawn.
    const spec = await d.fromMermaid("flowchart LR\n  a@{ shape: bad }", {
      style: "schematic",
      blocks: { bad: { parts: [{ from: 2, to: 1 }] } },
    });
    const drawn = await fail(() => (window as any).facetryChart.update(spec));
    return { unknown, name, drawn };
  }, pump);
  expect(result.unknown?.code).toBe("invalid-diagram");
  expect(result.unknown?.message).toMatch(/^no shape `pumpp`: .*; the blocks given are pump$/);
  expect(result.name?.code).toBe("invalid-options");
  expect(result.drawn).toEqual({
    code: "invalid-spec",
    message: "`to` is a height above `from`, up to 20 font sizes",
    path: "/diagram/blocks/bad/parts/0/to",
  });
});

// The example blocks (examples/diagrams/custom-blocks/blocks/*.json) in both styles that draw them.
test.describe("golden images", () => {
  test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1 });
  for (const style of ["schematic", "isometric"]) {
    test(`${style} diagram 38-custom-blocks matches its golden image (webgl2)`, async ({ page }) => {
      await page.goto(`/?backend=webgl2&ui=0&style=${style}&example=mermaid/38-custom-blocks`);
      await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
      await page.mouse.move(790, 495);
      await expect(page.locator("#chart")).toHaveScreenshot(`diagram-${style}-38-custom-blocks.png`);
    });
  }
});
