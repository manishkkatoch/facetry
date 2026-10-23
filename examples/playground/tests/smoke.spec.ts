import { expect, test } from "@playwright/test";
import { PNG } from "pngjs";

const WHITE = [0xff, 0xff, 0xff]; // the light theme's background
const BAR = [0x4c, 0x78, 0xa8]; // "#4C78A8", the colour in examples/specs/bar-basic.json

test("auto backend reaches ready and reports a backend", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  const backend = await page.locator("html").getAttribute("data-backend");
  console.log(`auto backend in this browser: ${backend}`);
  expect(["webgpu", "webgl2"]).toContain(backend);
  expect(errors).toEqual([]);
});

test("webgl2 fallback draws the background and the bars", async ({ page }) => {
  await page.goto("/?backend=webgl2&ui=0");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("html")).toHaveAttribute("data-backend", "webgl2");

  const png = PNG.sync.read(await page.locator("#chart").screenshot());
  const near = (i: number, want: number[], tolerance = 3) =>
    want.every((v, k) => Math.abs(png.data[i + k]! - v) <= tolerance);
  expect(near(0, WHITE)).toBe(true); // top-left corner: background
  expect(png.data[3]).toBe(255);
  let bar = 0;
  for (let i = 0; i < png.data.length; i += 4) if (near(i, BAR)) bar++;
  // Four bars of roughly 144 x 270..430 px.
  expect(bar).toBeGreaterThan(100_000);
});
