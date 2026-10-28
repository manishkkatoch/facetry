import { expect, test } from "@playwright/test";

// Icon packs (`diagram.registerIconPacks`): `@{ icon: "<pack>:<icon>" }` draws an icon of a
// registered Iconify set or icon record, loaded on first use, and only used icons travel.

test.beforeEach(async ({ page }) => {
  await page.goto("/?backend=webgl2&example=bar-basic");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
});

test("Iconify sets and icon records are drawn by `pack:icon`, aliases followed, only used icons kept", async ({ page }) => {
  const result = await page.evaluate(async () => {
    const d = (window as any).facetryDiagram;
    d.registerIconPacks([
      {
        name: "set",
        icons: {
          prefix: "set",
          width: 24,
          height: 24,
          icons: {
            box: { body: '<path fill="none" stroke="currentColor" d="M4 4h16v16H4z"/>' },
            dot: { body: '<circle cx="12" cy="12" r="6"/>', width: 24, height: 24 },
            unused: { body: '<path d="M0 0h1v1z"/>' },
          },
          aliases: { square: { parent: "box" } },
        },
      },
      { name: "mine", icons: { pump: "M2 2h20v20H2z", other: "M0 0h4v4z" } },
    ]);
    const spec = await d.fromMermaid(
      'flowchart LR\n  a@{ icon: "set:box" } --> b@{ icon: set:square } --> c@{ icon: "mine:pump" } --> e@{ icon: "set:dot" }',
      { style: "schematic" },
    );
    return {
      packs: d.iconPacks(),
      icons: Object.keys(spec.diagram.icons).sort(),
      box: spec.diagram.icons["set:box"],
      dot: spec.diagram.icons["set:dot"],
    };
  });
  expect(result.packs).toEqual(["set", "mine"]);
  expect(result.icons).toEqual(["mine:pump", "set:box", "set:dot", "set:square"]);
  // A stroked icon as lines, a filled one as a fill, in the set's box.
  expect(result.box.stroke).toBe("M4 4h16v16H4z");
  expect(result.box.viewBox).toEqual([0, 0, 24, 24]);
  expect(result.dot.fill).toHaveLength(1);
});

test("a pack's loader runs once, and only when a diagram names one of its icons", async ({ page }) => {
  const result = await page.evaluate(async () => {
    const d = (window as any).facetryDiagram;
    let calls = 0;
    d.registerIconPacks([
      {
        name: "lazy",
        loader: async () => {
          calls++;
          return { icons: { star: { body: '<path d="M12 2l3 7h7l-6 4 2 8-6-5-6 5 2-8-6-4h7z"/>' } }, width: 24, height: 24 };
        },
      },
    ]);
    await d.fromMermaid("flowchart LR\n  a --> b");
    const before = calls;
    await d.fromMermaid('flowchart LR\n  a@{ icon: "lazy:star" } --> b');
    await d.fromMermaid('flowchart LR\n  a@{ icon: "lazy:star" } --> b@{ icon: "lazy:star" }');
    return { before, after: calls };
  });
  expect(result).toEqual({ before: 0, after: 1 });
});

test("an icon missing from its pack, or a pack that fails to load, rejects naming it", async ({ page }) => {
  const result = await page.evaluate(async () => {
    const d = (window as any).facetryDiagram;
    let tries = 0;
    d.registerIconPacks([
      { name: "small", icons: { one: "M0 0h4v4z" } },
      {
        name: "broken",
        loader: async () => {
          tries++;
          throw new Error("404 Not Found");
        },
      },
    ]);
    const fail = async (src: string) => {
      try {
        await d.fromMermaid(src);
        return null;
      } catch (e: any) {
        return { code: e.code, message: e.message };
      }
    };
    const missing = await fail('flowchart LR\n  a@{ icon: "small:two" }');
    const broken = await fail('flowchart LR\n  a@{ icon: "broken:x" }');
    await fail('flowchart LR\n  a@{ icon: "broken:x" }');
    // A name given with the call wins over the pack, and needs no loading.
    const given = await d.fromMermaid('flowchart LR\n  a@{ icon: "broken:x" }', { icons: { "broken:x": "M0 0h4v4z" } });
    return { missing, broken, tries, given: Object.keys(given.diagram.icons) };
  });
  expect(result.missing).toEqual({
    code: "invalid-options",
    message: "icon `small:two` is not in the `small` icon pack",
  });
  expect(result.broken).toEqual({
    code: "invalid-options",
    message: "icon pack `broken` failed to load: 404 Not Found",
  });
  // A failed load is tried again next time.
  expect(result.tries).toBe(2);
  expect(result.given).toEqual(["broken:x"]);
});
