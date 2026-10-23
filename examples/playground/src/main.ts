import { Chart, FacetryError, type ChartOptions } from "facetry";

/** Also asserted by tests/smoke.spec.ts. */
const CLEAR_COLOR = "#336699";

const canvas = document.querySelector<HTMLCanvasElement>("#chart")!;
const status = document.querySelector<HTMLElement>("#status")!;
const root = document.documentElement;

// ?backend=webgl2 forces the fallback; ?backend=webgpu forces WebGPU.
const backend = (new URLSearchParams(location.search).get("backend") ?? "auto") as NonNullable<
  ChartOptions["backend"]
>;

try {
  const chart = await Chart.create(canvas, { backend, clearColor: CLEAR_COLOR });
  await chart.firstFrame;
  status.textContent = `Facetry — backend: ${chart.backend}`;
  root.dataset.backend = chart.backend;
  root.dataset.ready = "true"; // the e2e test waits for this
} catch (e) {
  const err = FacetryError.from(e);
  status.textContent = `Error (${err.code}): ${err.message}`;
  root.dataset.error = err.code;
}
