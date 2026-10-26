import { Chart, FacetryError, type ChartOptions, type ChartSpec, type UpdateOptions } from "facetry";

// The same files the Rust snapshot tests and the golden-image test use.
const modules = import.meta.glob<ChartSpec>("../../specs/*.json", { eager: true, import: "default" });
const examples = new Map(
  Object.entries(modules).map(([path, spec]) => [path.replace(/^.*\/(.+)\.json$/, "$1"), spec]),
);

const params = new URLSearchParams(location.search);
// ?backend=webgl2 forces the fallback; ?backend=webgpu forces WebGPU.
const backend = (params.get("backend") ?? "auto") as NonNullable<ChartOptions["backend"]>;
// ?animateIn=1 animates the first frame (the marks enter). ?clock=manual holds the animation
// clock from the start (for tests: `facetryChart.__animationClock(ms)` then moves it).
const animateIn = params.get("animateIn") === "1";
const manualClock = params.get("clock") === "manual";
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
const randomizeButton = document.querySelector<HTMLButtonElement>("#randomize")!;
const animateBox = document.querySelector<HTMLInputElement>("#animate")!;
const spinButton = document.querySelector<HTMLButtonElement>("#spin")!;
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

/** Shows the 3D camera (azimuth / elevation / distance); hidden for 2D charts. */
function showCamera(spec: ChartSpec) {
  const is3d = spec.dimension === "3d";
  cameraReadout.hidden = !is3d;
  spinButton.hidden = !is3d;
  if (!is3d) {
    delete root.dataset.camera;
    return;
  }
  const c = chart.camera;
  cameraReadout.textContent = `camera: azimuth ${c.azimuth.toFixed(0)}°, elevation ${c.elevation.toFixed(0)}°, distance ${c.distance.toFixed(2)}`;
  root.dataset.camera = JSON.stringify(c); // the e2e tests read this
}
let currentSpec: ChartSpec;

/** The "Animate" checkbox: off, every update jumps (`transition: false`); on, the spec's own transition applies. */
const updateOptions = (): UpdateOptions | undefined => (animateBox.checked ? undefined : { transition: false });

let updates = 0;
async function apply(spec: unknown) {
  try {
    chart.update(spec as ChartSpec, updateOptions());
    currentSpec = spec as ChartSpec;
    showCamera(currentSpec);
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
currentSpec = examples.get(initial)!;
let chart: Chart;
try {
  chart = await Chart.create(canvas, examples.get(initial)!, { backend, animateIn });
  if (manualClock) chart.__animationClock(0);
  await chart.firstFrame;
  status.textContent = `backend: ${chart.backend}, ${chart.msaaSamples}x MSAA`;
  root.dataset.msaa = String(chart.msaaSamples);
  root.dataset.backend = chart.backend;
  // For poking at it from the devtools console (and the e2e tests): `facetryChart.setCamera(...)`.
  // `playgroundSpec()` hands out a copy of the spec on screen, for tests to modify and apply.
  Object.assign(window, {
    facetryChart: chart,
    playgroundSpec: () => structuredClone(currentSpec),
    // A copy of one of the example specs (`examples/specs/<name>.json`).
    playgroundExample: (name: string) => structuredClone(examples.get(name)),
  });
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
  showIssues(err.issues);
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

// --- Randomize data ----------------------------------------------------------------------------

/** A small seeded generator (mulberry32), so a run of clicks can be reproduced with `?seed=`. */
let seed = Number(params.get("seed") ?? Date.now()) >>> 0;
function random(): number {
  seed = (seed + 0x6d2b79f5) >>> 0;
  let t = seed;
  t = Math.imul(t ^ (t >>> 15), t | 1);
  t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}

/**
 * Perturbs a spec's data the way fresh data would differ from the old: every value a bit
 * different, and now and then a category gone or a new one in its place (so bars enter and
 * leave, lines gain or lose a point, a pie a slice). Works on any example: the value columns
 * are the layers' `y` and `theta` fields, the categories their `x` (or, for a pie, `color`) field.
 */
function randomize(spec: ChartSpec): ChartSpec {
  const next = structuredClone(spec);
  const layers = next.layers ?? [];
  // The value columns to perturb, and the column that holds the categories, per table.
  const values = new Map<string, Set<string>>();
  const categories = new Map<string, string>();
  for (const layer of layers) {
    const enc = layer.encoding;
    const table = layer.data;
    const fields = values.get(table) ?? new Set<string>();
    for (const f of [enc.y?.field, enc.theta?.field]) if (f) fields.add(f);
    values.set(table, fields);
    const cat = layer.mark === "arc" ? enc.color?.field : enc.x?.field;
    if (cat && !categories.has(table)) categories.set(table, cat);
  }
  for (const [name, table] of Object.entries(next.data ?? {})) {
    const columns = table as Record<string, (number | string | null)[]>;
    for (const field of values.get(name) ?? []) {
      const column = columns[field];
      if (!column || typeof column.find((v) => v !== null) !== "number") continue;
      for (let i = 0; i < column.length; i++) {
        const v = column[i];
        if (typeof v !== "number") continue;
        const scaled = v * (0.5 + random());
        column[i] = Number.isInteger(v) ? Math.max(0, Math.round(scaled)) : Number(scaled.toFixed(2));
      }
    }
    const cat = categories.get(name);
    const names = cat ? [...new Set(columns[cat] ?? [])] : [];
    if (!cat || names.length === 0 || typeof names[0] !== "string") continue;
    const roll = random();
    if (roll < 0.35 && names.length > 2) {
      // Remove a category: every row that has it.
      const gone = names[Math.floor(random() * names.length)];
      const keep = columns[cat]!.map((c) => c !== gone);
      for (const key of Object.keys(columns)) columns[key] = columns[key]!.filter((_, i) => keep[i]);
    } else if (roll < 0.7) {
      // Add one: the rows of an existing category under a new name, at the end.
      const from = names[Math.floor(random() * names.length)];
      const fresh = `New ${++added}`;
      const rows = columns[cat]!.flatMap((c, i) => (c === from ? [i] : []));
      for (const i of rows) {
        for (const key of Object.keys(columns)) {
          const v = columns[key]![i]!;
          columns[key]!.push(key === cat ? fresh : v);
        }
      }
    }
  }
  return next;
}
let added = 0;

randomizeButton.addEventListener("click", () => {
  const spec = randomize(currentSpec);
  show(spec);
  void apply(spec);
});

// A quarter turn of the camera as a tween (shortest way round, like any setCamera with a duration).
spinButton.addEventListener("click", () => {
  chart.setCamera({ azimuth: chart.camera.azimuth + 90 }, animateBox.checked ? { duration: 700 } : undefined);
});
