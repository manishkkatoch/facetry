import { expect, test } from "@playwright/test";
import { PNG } from "pngjs";

const CLEAR = [0x33, 0x66, 0x99]; // keep in sync with src/main.ts

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

test("webgl2 fallback renders the clear colour", async ({ page }) => {
  await page.goto("/?backend=webgl2");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("html")).toHaveAttribute("data-backend", "webgl2");

  const png = PNG.sync.read(await page.locator("#chart").screenshot());
  const i = ((png.height >> 1) * png.width + (png.width >> 1)) * 4;
  const [r, g, b, a] = [png.data[i]!, png.data[i + 1]!, png.data[i + 2]!, png.data[i + 3]!];
  const tolerance = 3;
  CLEAR.forEach((want, k) => expect(Math.abs([r, g, b][k]! - want)).toBeLessThanOrEqual(tolerance));
  expect(a).toBe(255);
});
