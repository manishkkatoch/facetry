import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests",
  reporter: "list",
  // Golden images live next to the tests, one file per name (no per-OS suffix: we only
  // generate them on Linux/Chromium).
  snapshotPathTemplate: "{testDir}/__golden__/{arg}{ext}",
  expect: {
    toHaveScreenshot: {
      // Allow 1% of pixels to differ: anti-aliasing of glyph edges and bar edges varies a
      // little between Chromium builds and software-GL (SwiftShader) versions, and CI
      // installs its own Chromium. A real regression (a moved bar, a missing label, a
      // wrong colour) changes far more than 1% of the 400k pixels...
      maxDiffPixelRatio: 0.01,
      // ...and each pixel may be off by a few levels before it counts as different.
      threshold: 0.2,
    },
  },
  use: {
    baseURL: "http://localhost:4173",
    // Updates animate by default. Most tests are about what the chart looks like after an
    // update, not how it gets there, so they run as a viewer who asked for reduced motion
    // (every transition is then a jump, exactly as before animation existed). The animation
    // tests (transition*.spec.ts) opt back in with `test.use({ reducedMotion: "no-preference" })`.
    reducedMotion: "reduce",
    launchOptions: {
      // CI installs the matching browser; locally set CHROMIUM_PATH to reuse another one.
      executablePath: process.env.CHROMIUM_PATH || undefined,
      // Headless CI has no GPU: software WebGL via SwiftShader. WebGPU is flag-gated.
      args: ["--use-angle=swiftshader", "--enable-unsafe-swiftshader", "--ignore-gpu-blocklist"],
    },
  },
  // Serves the production build; run `pnpm -r build` first.
  webServer: {
    command: "pnpm preview",
    url: "http://localhost:4173",
    reuseExistingServer: !process.env.CI,
  },
});
