import { expect, test } from "@playwright/test";

// A fixed viewport and pixel ratio make the output reproducible. 800x500 is the size the
// Rust scene snapshot (crates/facetry-core/tests/snapshots) is laid out for.
test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1 });

test("basic bar chart matches its golden image (webgl2)", async ({ page }) => {
  await page.goto("/?backend=webgl2&ui=0&example=bar-basic");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("#chart")).toHaveScreenshot("bar-basic.png");
});

// Value labels: `label.show: "always"` on bars of both signs; the tallest bar's label does not
// fit above the plot, so it goes inside the bar in the contrasting colour.
test("bar chart with value labels matches its golden image (webgl2)", async ({ page }) => {
  await page.goto("/?backend=webgl2&ui=0&example=bar-labels");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await page.mouse.move(790, 495); // not over a bar
  await expect(page.locator("#chart")).toHaveScreenshot("bar-labels.png");
});

// 3D: the default camera (azimuth 35, elevation 25) of the 4 quarters x 3 regions example.
// Also checks that depth testing, lighting and the back-wall choice have not changed.
test("3D bar chart matches its golden image (webgl2)", async ({ page }) => {
  await page.goto("/?backend=webgl2&ui=0&example=bar3d-grid");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("#chart")).toHaveScreenshot("bar3d-grid.png");
});

// Bar positions. Stacked: segments labelled inside (the thin Q4 Services one has no room for
// its label); grouped: one slot per product in each quarter's band. Lines: categorical x with
// markers, numeric x (a gap in one series), and bars with a line on a second y axis.
for (const name of ["bar-stacked", "bar-stacked-hidden", "bar-grouped", "line-basic", "line-numeric-x", "combo-bar-line", "pie", "donut"]) {
  test(`${name} matches its golden image (webgl2)`, async ({ page }) => {
    await page.goto(`/?backend=webgl2&ui=0&example=${name}`);
    await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
    await page.mouse.move(790, 495); // not over a bar
    await expect(page.locator("#chart")).toHaveScreenshot(`${name}.png`);
  });
}

// Pie (percent labels; the thin slices' labels go outside on leader lines) and donut.
// 3D stacked: quarters x regions, each cell a stack of three products.
test("3D stacked bar chart matches its golden image (webgl2)", async ({ page }) => {
  await page.goto("/?backend=webgl2&ui=0&example=bar3d-stacked");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("#chart")).toHaveScreenshot("bar3d-stacked.png");
});

// The extruded pie at its spec camera: lit top faces and walls, labels on the top face.
test("3D pie matches its golden image (webgl2)", async ({ page }) => {
  await page.goto("/?backend=webgl2&ui=0&example=pie3d");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await page.mouse.move(790, 495);
  await expect(page.locator("#chart")).toHaveScreenshot("pie3d.png");
});

// Lines in 3D: three series on three depth rows with markers, and bars with a line per row.
for (const name of ["line3d", "combo3d-bar-line"]) {
  test(`3D ${name} matches its golden image (webgl2)`, async ({ page }) => {
    await page.goto(`/?backend=webgl2&ui=0&example=${name}`);
    await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
    await page.mouse.move(790, 495);
    await expect(page.locator("#chart")).toHaveScreenshot(`${name}.png`);
  });
}

// Diagrams from Mermaid (facetry/diagrams): a decision with a cycle and labelled edges, and a
// left-to-right pipeline with a fan-out and a fan-in. Orthographic, square on, edges at the
// height of the node tops with flat arrowheads.
for (const name of ["01-decision", "02-ci-pipeline"]) {
  test(`diagram ${name} matches its golden image (webgl2)`, async ({ page }) => {
    await page.goto(`/?backend=webgl2&ui=0&example=mermaid/${name}`);
    await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
    await page.mouse.move(790, 495);
    await expect(page.locator("#chart")).toHaveScreenshot(`diagram-${name}.png`);
  });
}

// The glossy style: coloured slabs (by shape, and by classDef in 26-styled), thick
// square-cornered arrows on the floor, label cards, soft shadows and printed labels, seen in
// perspective at an angle.
for (const name of ["01-decision", "26-styled"]) {
  test(`glossy diagram ${name} matches its golden image (webgl2)`, async ({ page }) => {
    await page.goto(`/?backend=webgl2&ui=0&style=glossy&example=mermaid/${name}`);
    await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
    await page.mouse.move(790, 495);
    await expect(page.locator("#chart")).toHaveScreenshot(`diagram-glossy-${name}.png`);
  });
}

// The isometric style: layered pedestals in palette colours (27-timeline, after infographic
// timelines) and a flowchart with a cycle and labelled roads, in the classic isometric view.
for (const name of ["27-timeline", "01-decision"]) {
  test(`isometric diagram ${name} matches its golden image (webgl2)`, async ({ page }) => {
    await page.goto(`/?backend=webgl2&ui=0&style=isometric&example=mermaid/${name}`);
    await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
    await page.mouse.move(790, 495);
    await expect(page.locator("#chart")).toHaveScreenshot(`diagram-isometric-${name}.png`);
  });
}

// Cards and icons: the roadmap with a card beside each pedestal (title, wrapped text, icon;
// the pedestals show step numbers), and icons above the labels on the node tops.
for (const [name, query] of [
  ["isometric-cards-28-roadmap", "style=isometric&cards=1&example=mermaid/28-roadmap"],
  ["isometric-29-icons", "style=isometric&example=mermaid/29-icons"],
]) {
  test(`diagram ${name} matches its golden image (webgl2)`, async ({ page }) => {
    await page.goto(`/?backend=webgl2&ui=0&${query}`);
    await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
    await page.mouse.move(790, 495);
    await expect(page.locator("#chart")).toHaveScreenshot(`diagram-${name}.png`);
  });
}

// The schematic style: equipment by shape (buildings, silos on plinths, a tank, a valve,
// reactors, a kiln), pipes and conveyors on supports with flow marks, and signs standing above
// the equipment (with icons, in 29-icons), over a floor grid.
for (const name of ["30-plant", "29-icons"]) {
  test(`schematic diagram ${name} matches its golden image (webgl2)`, async ({ page }) => {
    await page.goto(`/?backend=webgl2&ui=0&style=schematic&example=mermaid/${name}`);
    await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
    await page.mouse.move(790, 495);
    await expect(page.locator("#chart")).toHaveScreenshot(`diagram-schematic-${name}.png`);
  });
}

// The user's own icons (examples/diagrams/custom-icons/icons/*.svg, from line-icon and filled
// icon sets): on the schematic signs, and on the isometric cards.
for (const [name, query] of [
  ["schematic-31-custom-icons", "style=schematic&example=mermaid/31-custom-icons"],
  ["isometric-cards-31-custom-icons", "style=isometric&cards=1&example=mermaid/31-custom-icons"],
]) {
  test(`diagram ${name} matches its golden image (webgl2)`, async ({ page }) => {
    await page.goto(`/?backend=webgl2&ui=0&${query}`);
    await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
    await page.mouse.move(790, 495);
    await expect(page.locator("#chart")).toHaveScreenshot(`diagram-${name}.png`);
  });
}
