import { expect, test, type Page } from "@playwright/test";

// The package ships two wasm modules (WebGPU-only and WebGL2-only). These tests watch the
// network to prove a page only ever downloads the one it needs. Vite names the assets
// `facetry_wasm_bg-<hash>.wasm` and `facetry_wasm_gl_bg-<hash>.wasm`.
const PRIMARY = /facetry_wasm_bg-.*\.wasm/;
const FALLBACK = /facetry_wasm_gl_bg-.*\.wasm/;
const FALLBACK_GLUE = /facetry_wasm_gl-.*\.js/;
const PRIMARY_GLUE = /facetry_wasm-[^/]*\.js/;

function watchRequests(page: Page): string[] {
  const urls: string[] = [];
  page.on("request", (r) => urls.push(new URL(r.url()).pathname));
  return urls;
}
const count = (urls: string[], re: RegExp) => urls.filter((u) => re.test(u)).length;

/** True when this browser really has WebGPU (then the headless-fallback tests do not apply). */
async function hasWebGpu(page: Page): Promise<boolean> {
  // Any page on the origin will do; wait until it has finished loading its module so that its
  // requests cannot leak into the test's own request log.
  await page.goto("/?ui=0&backend=webgl2");
  await page.waitForFunction(() => document.documentElement.dataset.ready === "true");
  return page.evaluate(async () => {
    try {
      const gpu = (navigator as unknown as { gpu?: { requestAdapter(): Promise<unknown> } }).gpu;
      return gpu !== undefined && (await gpu.requestAdapter()) != null;
    } catch {
      return false;
    }
  });
}

test("?backend=webgl2 downloads only the WebGL2 module", async ({ page }) => {
  const urls = watchRequests(page);
  await page.goto("/?backend=webgl2&ui=0");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("html")).toHaveAttribute("data-backend", "webgl2");
  expect(count(urls, FALLBACK)).toBe(1);
  expect(count(urls, FALLBACK_GLUE)).toBe(1);
  expect(count(urls, PRIMARY)).toBe(0);
  expect(count(urls, PRIMARY_GLUE)).toBe(0);
});

test("auto without WebGPU falls back to WebGL2 and never fetches the WebGPU module", async ({
  page,
}) => {
  test.skip(await hasWebGpu(page), "this browser has WebGPU");
  const urls = watchRequests(page);
  await page.goto("/?ui=0");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("html")).toHaveAttribute("data-backend", "webgl2");
  expect(count(urls, FALLBACK)).toBe(1);
  expect(count(urls, PRIMARY)).toBe(0);
});

test("?backend=webgpu loads only the WebGPU module and fails cleanly without WebGPU", async ({
  page,
}) => {
  test.skip(await hasWebGpu(page), "this browser has WebGPU");
  const urls = watchRequests(page);
  await page.goto("/?backend=webgpu&ui=0");
  await expect(page.locator("html")).toHaveAttribute("data-error", "gpu-unavailable");
  expect(count(urls, PRIMARY)).toBe(1);
  expect(count(urls, FALLBACK)).toBe(0);
  expect(count(urls, FALLBACK_GLUE)).toBe(0);
});

// A false positive: navigator.gpu hands out something that looks like an adapter, the WebGPU
// module is downloaded, but wgpu cannot actually get a device. `auto` must then retry once
// on the WebGL2 module and still draw, on the same canvas.
test("auto retries on WebGL2 when WebGPU turns out to be unusable", async ({ page }) => {
  test.skip(await hasWebGpu(page), "this browser has WebGPU");
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "gpu", {
      configurable: true,
      value: {
        // An adapter that looks plausible but cannot give out a device.
        requestAdapter: async () => ({
          features: new Set(),
          limits: new Proxy({}, { get: () => 65536 }),
          info: { vendor: "", architecture: "", device: "", description: "" },
          isFallbackAdapter: false,
          requestDevice: async () => {
            throw new Error("no device");
          },
        }),
        getPreferredCanvasFormat: () => "bgra8unorm",
      },
    });
  });
  const urls = watchRequests(page);
  await page.goto("/?ui=0");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await expect(page.locator("html")).toHaveAttribute("data-backend", "webgl2");
  expect(count(urls, PRIMARY)).toBe(1); // tried WebGPU first...
  expect(count(urls, FALLBACK)).toBe(1); // ...then fell back, once
});
