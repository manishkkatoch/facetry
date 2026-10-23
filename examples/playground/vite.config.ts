import { defineConfig } from "vite";

export default defineConfig({
  // Relative asset URLs, so the build works both at / (dev, preview) and under
  // /facetry/ on GitHub Pages.
  base: "./",
  // Top-level await in main.ts.
  build: { target: "es2022" },
});
