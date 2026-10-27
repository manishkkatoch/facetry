// The page of examples/playground/run.html: Mermaid blocks drawn by `diagram.run()`.
import { diagram } from "facetry/diagrams";

const backend = new URLSearchParams(location.search).get("backend");
const charts = await diagram.run({
  chart: backend === "webgl2" || backend === "webgpu" ? { backend } : {},
  height: "320px",
  suppressErrors: true,
});
// For the end-to-end tests.
Object.assign(window, { facetryCharts: charts, facetryDiagram: diagram });
document.documentElement.dataset.ready = "true";
