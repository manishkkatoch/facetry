import { Chart, FacetryError, type ChartOptions, type ChartSpec } from "facetry";

// The same files the Rust snapshot tests and the golden-image test use.
const modules = import.meta.glob<ChartSpec>("../../specs/*.json", { eager: true, import: "default" });
const examples = new Map(
  Object.entries(modules).map(([path, spec]) => [path.replace(/^.*\/(.+)\.json$/, "$1"), spec]),
);

const params = new URLSearchParams(location.search);
// ?backend=webgl2 forces the fallback; ?backend=webgpu forces WebGPU.
const backend = (params.get("backend") ?? "auto") as NonNullable<ChartOptions["backend"]>;
// ?example=<name> picks the starting spec; ?ui=0 hides everything but the chart.
const initial = examples.has(params.get("example") ?? "") ? params.get("example")! : "bar-basic";
if (params.get("ui") === "0") document.body.classList.add("embed");

const canvas = document.querySelector<HTMLCanvasElement>("#chart")!;
const status = document.querySelector<HTMLElement>("#status")!;
const select = document.querySelector<HTMLSelectElement>("#example")!;
const textarea = document.querySelector<HTMLTextAreaElement>("#json")!;
const applyButton = document.querySelector<HTMLButtonElement>("#apply")!;
const issuesList = document.querySelector<HTMLUListElement>("#issues")!;
const root = document.documentElement;

for (const name of examples.keys()) select.append(new Option(name, name));
select.value = initial;

const show = (spec: ChartSpec) => (textarea.value = JSON.stringify(spec, null, 2));
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

let updates = 0;
async function apply(spec: unknown) {
  try {
    chart.update(spec as ChartSpec);
    showIssues();
  } catch (e) {
    const err = FacetryError.from(e);
    showIssues(err.issues.length ? err.issues : [{ path: "", message: err.message }]);
    return;
  }
  await nextFrames();
  root.dataset.updates = String(++updates); // the e2e tests wait on this
}

show(examples.get(initial)!);
let chart: Chart;
try {
  chart = await Chart.create(canvas, examples.get(initial)!, { backend });
  await chart.firstFrame;
  status.textContent = `backend: ${chart.backend}, ${chart.msaaSamples}x MSAA`;
  root.dataset.msaa = String(chart.msaaSamples);
  root.dataset.backend = chart.backend;
  root.dataset.ready = "true"; // the e2e tests wait for this
} catch (e) {
  const err = FacetryError.from(e);
  status.textContent = `Error (${err.code}): ${err.message}`;
  root.dataset.error = err.code;
  showIssues(err.issues);
  throw e;
}

select.addEventListener("change", () => {
  const spec = examples.get(select.value)!;
  show(spec);
  void apply(spec);
});

applyButton.addEventListener("click", () => {
  let spec: unknown;
  try {
    spec = JSON.parse(textarea.value);
  } catch (e) {
    showIssues([{ path: "", message: `not valid JSON: ${(e as Error).message}` }]);
    return;
  }
  void apply(spec);
});
