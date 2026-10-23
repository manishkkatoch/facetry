import { defineConfig } from "vite";

export default defineConfig({
  // Top-level await in main.ts.
  build: { target: "es2022" },
});
