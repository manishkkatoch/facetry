import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "tests",
  reporter: "list",
  use: {
    baseURL: "http://localhost:4173",
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
