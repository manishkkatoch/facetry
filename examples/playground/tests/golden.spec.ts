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
for (const name of ["bar-stacked", "bar-grouped", "line-basic", "line-numeric-x", "combo-bar-line", "pie", "donut"]) {
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
