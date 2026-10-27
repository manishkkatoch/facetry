import { Chart, FacetryError, type ChartOptions, type ChartSpec } from "facetry";
import { diagram, type DiagramStyle } from "facetry/diagrams";

// The same files the Rust snapshot tests and the golden-image test use.
const modules = import.meta.glob<ChartSpec>("../../specs/*.json", { eager: true, import: "default" });
// Mermaid diagrams (the layout corpus of crates/facetry-diagrams), listed as `mermaid/<name>`.
const mermaid = import.meta.glob<string>("../../diagrams/*/*.mmd", {
  eager: true,
  query: "?raw",
  import: "default",
});
type Example = ChartSpec | string;
const examples = new Map<string, Example>([
  ...Object.entries(modules).map(
    ([path, spec]) => [path.replace(/^.*\/(.+)\.json$/, "$1"), spec] as [string, Example],
  ),
  ...Object.entries(mermaid).map(
    ([path, text]) => [path.replace(/^.*\/(.+)\.mmd$/, "mermaid/$1"), text] as [string, Example],
  ),
]);

const styleSelect = document.querySelector<HTMLSelectElement>("#style")!;

/** A spec as it is, or Mermaid text laid out by the diagrams add-on in the chosen style. */
const toSpec = (example: Example): Promise<ChartSpec> =>
  typeof example === "string"
    ? diagram.fromMermaid(example, { style: styleSelect.value as DiagramStyle })
    : Promise.resolve(example);

const params = new URLSearchParams(location.search);
// ?style=glossy draws Mermaid diagrams in the glossy style.
styleSelect.value = params.get("style") === "glossy" ? "glossy" : "flat";
// ?backend=webgl2 forces the fallback; ?backend=webgpu forces WebGPU.
const backend = (params.get("backend") ?? "auto") as NonNullable<ChartOptions["backend"]>;
// ?example=<name> picks the starting spec; ?ui=0 hides everything but the chart.
const initial = examples.has(params.get("example") ?? "") ? params.get("example")! : "bar-basic";
if (params.get("ui") === "0") document.body.classList.add("embed");

const canvas = document.querySelector<HTMLCanvasElement>("#chart")!;
const status = document.querySelector<HTMLElement>("#status")!;
const cameraReadout = document.querySelector<HTMLElement>("#camera")!;
const select = document.querySelector<HTMLSelectElement>("#example")!;
const textarea = document.querySelector<HTMLTextAreaElement>("#json")!;
const applyButton = document.querySelector<HTMLButtonElement>("#apply")!;
const saveButton = document.querySelector<HTMLButtonElement>("#save-png")!;
const hoverReadout = document.querySelector<HTMLElement>("#hover")!;
const issuesList = document.querySelector<HTMLUListElement>("#issues")!;
const root = document.documentElement;

for (const name of examples.keys()) select.append(new Option(name, name));
select.value = initial;

const show = (example: Example) =>
  (textarea.value = typeof example === "string" ? example : JSON.stringify(example, null, 2));
/** Problems in the spec or in the Mermaid text, as `{ path, message }`. */
function issuesOf(err: FacetryError) {
  if (err.diagramIssues.length) {
    return err.diagramIssues.map((i) => ({ path: `line ${i.line}:${i.column}`, message: i.message }));
  }
  return err.issues.length ? err.issues : [{ path: "", message: err.message }];
}
/** Lists problems under the editor; no argument clears them. */
function showIssues(items: { path: string; message: string }[] = []) {
  issuesList.replaceChildren(
    ...items.map(({ path, message }) => {
      const li = document.createElement("li");
      const code = document.createElement("code");
      code.textContent = path || "(spec)";
      li.append(code, `: ${message}`);
      return li;
    }),
  );
  root.dataset.issues = String(items.length);
}

/** Resolves once the frame after the update has been drawn. */
const nextFrames = () =>
  new Promise<void>((resolve) => requestAnimationFrame(() => requestAnimationFrame(() => resolve())));

/** Shows the 3D camera (azimuth / elevation / distance); hidden for 2D charts. */
function showCamera(spec: ChartSpec) {
  const is3d = spec.dimension === "3d";
  cameraReadout.hidden = !is3d;
  if (!is3d) {
    delete root.dataset.camera;
    return;
  }
  const c = chart.camera;
  cameraReadout.textContent = `camera: azimuth ${c.azimuth.toFixed(0)}°, elevation ${c.elevation.toFixed(0)}°, distance ${c.distance.toFixed(2)}`;
  root.dataset.camera = JSON.stringify(c); // the e2e tests read this
}
let currentSpec: ChartSpec;

let updates = 0;
async function apply(example: unknown) {
  try {
    const spec = await toSpec(example as Example);
    chart.update(spec);
    currentSpec = spec;
    showCamera(currentSpec);
    showIssues();
  } catch (e) {
    showIssues(issuesOf(FacetryError.from(e)));
    return;
  }
  await nextFrames();
  root.dataset.updates = String(++updates); // the e2e tests wait on this
}

show(examples.get(initial)!);
let chart: Chart;
try {
  currentSpec = await toSpec(examples.get(initial)!);
  chart = await Chart.create(canvas, currentSpec, { backend });
  await chart.firstFrame;
  status.textContent = `backend: ${chart.backend}, ${chart.msaaSamples}x MSAA`;
  root.dataset.msaa = String(chart.msaaSamples);
  root.dataset.backend = chart.backend;
  // For poking at it from the devtools console (and the e2e tests): `facetryChart.setCamera(...)`.
  Object.assign(window, { facetryChart: chart });
  showCamera(currentSpec);
  // Hover and click events: show the bar under the pointer, log clicks.
  chart.on("hover", (mark) => {
    hoverReadout.textContent = mark ? `${mark.key} ${JSON.stringify(mark.datum)}` : "";
  });
  chart.on("click", (mark) => console.log("click", mark));
  // The chart handles the gestures itself; we only mirror the camera it ends up with.
  for (const type of ["pointermove", "wheel", "dblclick"]) {
    canvas.addEventListener(type, () => requestAnimationFrame(() => showCamera(currentSpec)), {
      passive: true,
    });
  }
  root.dataset.ready = "true"; // the e2e tests wait for this
} catch (e) {
  const err = FacetryError.from(e);
  status.textContent = `Error (${err.code}): ${err.message}`;
  root.dataset.error = err.code;
  showIssues(issuesOf(err));
  throw e;
}

saveButton.addEventListener("click", () => {
  chart.downloadPNG(`facetry-${select.value}.png`).catch((e: unknown) => {
    const err = FacetryError.from(e);
    status.textContent = `Error (${err.code}): ${err.message}`;
  });
});

select.addEventListener("change", () => {
  const spec = examples.get(select.value)!;
  show(spec);
  void apply(spec);
});

// A new style lays the Mermaid text in the editor out again.
styleSelect.addEventListener("change", () => {
  if (!textarea.value.trimStart().startsWith("{")) void apply(textarea.value);
});

applyButton.addEventListener("click", () => {
  // JSON is a chart spec; anything else is taken as Mermaid.
  if (!textarea.value.trimStart().startsWith("{")) {
    void apply(textarea.value);
    return;
  }
  let spec: unknown;
  try {
    spec = JSON.parse(textarea.value);
  } catch (e) {
    showIssues([{ path: "", message: `not valid JSON: ${(e as Error).message}` }]);
    return;
  }
  void apply(spec);
});
