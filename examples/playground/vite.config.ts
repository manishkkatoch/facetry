import { fileURLToPath } from "node:url";
import { defineConfig } from "vite";

export default defineConfig({
  // Relative asset URLs, so the build works both at / (dev, preview) and under
  // /facetry/ on GitHub Pages.
  base: "./",
  build: {
    // Top-level await in main.ts.
    target: "es2022",
    // The playground, and a page of Mermaid blocks drawn by `diagram.run()`.
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("index.html", import.meta.url)),
        run: fileURLToPath(new URL("run.html", import.meta.url)),
      },
    },
  },
});
