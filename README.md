# Facetry

GPU-accelerated charts for JS/TS apps, written in Rust and compiled to WebAssembly.
Rendering uses [wgpu](https://wgpu.rs): WebGPU where the browser really has it, WebGL2 otherwise.

**Status: M1 (2D bar chart) plus M4 (3D bars), built ahead of stacking, lines and pies to get
feedback on the riskiest part first.** A `Chart` draws a bar chart from a declarative JSON spec:
validation, band and linear scales with "nice" ticks, layout, axes (lines, ticks, gridlines,
labels, titles), MSDF text and instanced rectangles; with `dimension: "3d"` the bars are
lit, depth-tested boxes you can orbit (see [3D charts](#3d-charts)). Lines, arcs, stacking,
legends, tooltips and animation come in later milestones.

```ts
import { Chart, FacetryError, type ChartSpec } from "facetry";

const spec: ChartSpec = {
  dimension: "2d",
  data: { sales: { quarter: ["Q1", "Q2", "Q3", "Q4"], revenue: [120, 90, 140, 110] } },
  axes: [
    { id: "x", orient: "bottom", title: "Quarter" },
    { id: "y", orient: "left", title: "Revenue" },
  ],
  layers: [
    {
      mark: "bar",
      data: "sales",
      encoding: {
        x: { field: "quarter", axis: "x" },
        y: { field: "revenue", axis: "y" },
        color: { value: "#4C78A8" },
        key: { field: "quarter" },
      },
    },
  ],
};

const chart = await Chart.create(canvas, spec, { backend: "auto" });
console.log(chart.backend); // "webgpu" | "webgl2"

try {
  chart.update(newSpec); // re-layout and redraw (no animation yet)
} catch (e) {
  if (e instanceof FacetryError && e.code === "invalid-spec") {
    for (const { path, message } of e.issues) console.log(path, message); // e.g. /layers/0/encoding/y/axis
  }
}
chart.destroy();
```

`Chart.create` / `chart.update` reject with a `FacetryError` whose `code` is `invalid-spec`
(with `issues`, every problem found, each a JSON-pointer `path` plus a `message`; nothing is
drawn until the spec is valid), `gpu-unavailable`, `invalid-options` or `render-failed`.

`backend: "auto"` (the default) uses WebGPU when `navigator.gpu` exists and yields an adapter,
otherwise WebGL2; if WebGPU then fails to start (adapter, device or surface), it retries once on
WebGL2. `"webgpu"` and `"webgl2"` never fall back and reject with `gpu-unavailable`. The package
ships **two wasm modules** and downloads only the one it needs: a small WebGPU-only module and a
larger WebGL2-only one that is fetched lazily where WebGPU is missing (see [Bundle size](#bundle-size)).
Asking a module for a backend it was not built with (only possible when using the wasm directly)
also rejects with `gpu-unavailable`, with a message saying which backend the build contains.

### The spec

- `data`: named, columnar tables. A column is an array of numbers (`null` = missing) or an array
  of strings (categorical). All columns of a table must be the same length.
- `axes`: `{ id, orient: "bottom" | "left" | "right", title?, scale? }`. `scale` (numeric axes
  only) takes `{ domain: [min, max], nice, zero }`. At most one axis per side. A layer binds a
  channel to an axis by `id`, so a layer can use a `right` axis that has its own domain.
- `layers[]`: `{ mark: "bar", data, encoding }`. `x` must be a categorical field (band scale),
  `y` a numeric field (linear scale, includes zero by default). `color` is `{ value: "#hex" }` or
  `{ field }` (categorical field mapped onto the theme palette). `key` is optional; the default
  key of a bar is `layer<n>/<x value>`. Several layers on the same axes overlay.
- Unknown fields are errors, so typos are caught. Empty data is valid: the axes are drawn (numeric
  domain `[0, 1]`, no categories) and there are no bars.
- Tick labels use thousands separators (`1,200`) and as many decimals as the tick step needs
  (`0.25`, never `0.30000000000000004`).

### 3D charts

```ts
const spec: ChartSpec = {
  dimension: "3d",
  data: { sales: { quarter: [/* ... */], region: [/* ... */], revenue: [/* ... */] } },
  axes: [
    { id: "x", orient: "bottom", title: "Quarter" },
    { id: "y", orient: "left", title: "Revenue" },
    { id: "z", orient: "depth", title: "Region" },
  ],
  layers: [
    {
      mark: "bar",
      data: "sales",
      encoding: {
        x: { field: "quarter", axis: "x" },
        y: { field: "revenue", axis: "y" },
        z: { field: "region", axis: "z" }, // optional: without it the bars form one row at the front
        color: { field: "region" },
      },
    },
  ],
  camera: { type: "perspective", azimuth: 35, elevation: 25, distance: 2.7 }, // all optional
  interaction: { orbit: true }, // the default in 3D
};
```

- `z` is a categorical field (band scale), bound to an `orient: "depth"` axis. The first category
  is the front row. Bars stand on square cells (`x` categories by `z` categories) and are 1 unit
  high; `y` maps the value domain onto that height, with negative values hanging below zero.
- `camera`: `type` is `perspective` (default, 40 degree field of view) or `orthographic`;
  `azimuth` degrees around the chart (0 = front, positive = camera moves right); `elevation`
  degrees above the floor, clamped to 5-85; `distance` in box radii (half the plot box's diagonal,
  at least 1), clamped to 1.3-8. Defaults: 35, 25, 2.7.
- `dimension: "3d"` allows `depth` axes, the `z` channel, `camera` and `interaction`; these are
  validation errors in 2D (with paths such as `/layers/0/encoding/z`). Right axes are not
  supported in 3D yet.

**Camera controls.** In a 3D chart (with `interaction.orbit` not `false`) drag to orbit, use the
wheel to zoom and double-click to reset. From code:

```ts
chart.setCamera({ azimuth: 120, elevation: 40 }); // any subset of type/azimuth/elevation/distance
chart.camera;                                      // { type, azimuth, elevation, distance }
chart.resetCamera();                               // back to the spec's camera
```

The camera lives in Rust and moving it only rewrites one uniform and asks for a frame: the
scene is **not** rebuilt, and nothing renders until something changes. `chart.update(spec)`
keeps the orbited camera unless the spec's `camera` changed.

How 3D differs under the hood: `build_scene_3d` produces a `Scene3d` in world units (boxes, lines,
labels anchored at world points). What depends on the viewpoint (back-wall gridlines; which
floor edge carries each axis, so labels stay on the near or outer side) is emitted for each
45 degree sector of azimuth, tagged with its range, and the vertex shaders drop the variants the
camera is not in. Labels are projected from their anchor and then laid out in screen pixels, so
they face the viewer and keep their size; they draw last, without depth testing, with an
outline in the background colour so they stay legible over bars. Known limits: tick labels are
thinned for the starting camera and not again when zooming; an axis that points straight at the
camera (azimuth near 0/90/180/270 for one of the two floor axes) has its labels stacked tightly,
and some labels can be drawn over a bar that stands in front of them.

The TypeScript types are generated from the Rust definitions, so they cannot drift.
Example specs live in `examples/specs/` and are shared by the Rust snapshot tests, the
playground and the golden-image test.

## Layout

```
crates/facetry-core     pure Rust: spec + validation, scales, layout, Scene and Scene3d
                        (no GPU/browser deps)
crates/facetry-font     the bundled MSDF font (raw RGB atlas + metrics JSON) and text measuring
crates/facetry-render   wgpu Renderer: rect + MSDF text pipelines (2D); box, line and world-text
                        pipelines plus the glam orbit camera (3D); 4x MSAA (native + wasm32)
crates/facetry-wasm     wasm-bindgen glue: the `Chart` class and typed errors; features `webgpu`
                        (default) and `webgl` choose the backend, built once per module
tools/font-atlas        offline tool that generates the MSDF atlas (not part of the build)
packages/facetry        npm package (TypeScript): lazily loads one of two wasm modules, ResizeObserver + DPR
                        sizing, orbit/zoom/reset gestures, renders only when dirty
examples/specs          example chart specs (JSON)
examples/playground     Vite app + Playwright tests (smoke, playground, golden images, orbit)
scripts/                build-wasm.sh, wasm-size.sh
```

How a frame is made: `spec` -> `validate` -> `build_scene(spec, viewport, dpr, font metrics)` ->
`Scene` (flat lists of keyed rects, rules and texts in device pixels) -> `Renderer::set_scene`
(instance buffers) -> one render pass: gridlines, bars, axis rules, then text. Resizing rebuilds
the scene, because margins, tick counts and label thinning depend on the size. A 3D frame is
`build_scene_3d` -> `Scene3d` -> `Renderer::set_scene_3d`; each render pass clears colour and
depth, draws bars (depth-tested, culled back faces, Lambert lighting), then lines (tested against
the bars), then text.

## Prerequisites

- Rust stable (the `wasm32-unknown-unknown` target is listed in `rust-toolchain.toml`)
- [wasm-pack](https://rustwasm.github.io/wasm-pack/) (`cargo install wasm-pack`)
- Node 22 and pnpm 10 (`wasm-opt` comes from the `binaryen` npm package)

## Commands

```sh
pnpm install
pnpm build:wasm            # both modules: wasm-pack + wasm-opt -Oz -> packages/facetry/wasm, prints sizes
pnpm -r build              # tsc for the package, tsc + vite build for the playground
pnpm dev                   # playground dev server; add ?backend=webgl2 to force the fallback

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p facetry-core -p facetry-render -p facetry-wasm --target wasm32-unknown-unknown --all-targets -- -D warnings
cargo clippy -p facetry-render -p facetry-wasm --target wasm32-unknown-unknown --all-targets --no-default-features --features webgl -- -D warnings
cargo test --workspace     # unit, property (proptest) and snapshot (insta) tests

pnpm --filter playground exec playwright install chromium   # once
pnpm test:e2e              # Playwright against `vite preview` (run `pnpm -r build` first)
```

Set `CHROMIUM_PATH` to run the Playwright tests with an already-installed Chromium.

### Playground

`pnpm dev` (or `pnpm -r build && pnpm --filter playground preview`) serves a page with the chart,
a dropdown of the example specs, and a JSON editor: edit the spec and press **Apply** to call
`chart.update`; validation issues are listed with their JSON-pointer paths. URL parameters:
`?backend=webgl2` (force the fallback), `?example=bar-colored` (or `bar3d-grid`, `bar3d-row`,
`bar3d-negative`), `?ui=0` (only the chart). 3D examples show the current camera in the toolbar;
`facetryChart` on `window` is the live chart for poking at from the console.

### Snapshots and golden images

- **Scene snapshots** (`crates/facetry-core/tests/snapshots`) pin the exact layout of every
  example spec. After an intentional layout change: `INSTA_UPDATE=always cargo test -p facetry-core`
  (or `cargo insta review`), then look at the diff in git before committing. CI sets
  `INSTA_UPDATE=no`, so any difference fails.
- **Golden images** (`examples/playground/tests/__golden__/bar-basic.png` and `bar3d-grid.png`):
  Playwright screenshots the basic bar example and the 3D grid example (default camera) in
  Chromium at 800x500, devicePixelRatio 1, `?backend=webgl2`, and allows 1% of pixels to differ. To regenerate after an intentional visual change:
  `pnpm -r build && pnpm --filter playground exec playwright test golden --update-snapshots`,
  then **look at the PNG** before committing it.

### Regenerating the font atlas

The text atlas is generated offline and committed; nothing at build or run time needs the font.
Only needed if you change the font, glyph set, atlas size or distance range (constants in
`tools/font-atlas/src/main.rs`):

```sh
cargo run -p font-atlas --release
```

This rewrites `crates/facetry-font/assets/inter-regular-msdf.rgb` (raw RGB, embedded),
`inter-regular-msdf.png` (the same pixels, for viewing; a test checks they agree) and
`inter-regular-metrics.json` from `tools/font-atlas/fonts/Inter-Regular.ttf` (Inter 4.1, SIL Open
Font License 1.1, licence next to it). Rebuild the wasm afterwards (the files are embedded with
`include_bytes!`) and update the scene snapshots, since layout uses the glyph widths.

## Bundle size

The package ships two wasm modules built from the same source (`scripts/build-wasm.sh`, release
profile `opt-level = "z"` + LTO, then `wasm-opt -Oz`); `scripts/wasm-size.sh` prints these numbers:

| | raw | gzip -9 | brotli -q 11 | loaded |
|---|---:|---:|---:|---|
| `facetry_wasm` (WebGPU only) | 563,245 B (550 KiB) | 254,991 B (249 KiB) | 212,008 B (207 KiB) | always, where WebGPU works |
| `facetry_wasm_gl` (WebGL2 only) | 2,187,106 B (2136 KiB) | 897,869 B (877 KiB) | 711,806 B (695 KiB) | only on browsers without WebGPU, or `backend: "webgl2"` |

History of the single combined module (both backends), for comparison:

| | M0 | M1 | M4 (3D) | combined, before splitting |
|---|---|---|---|---|
| raw | 1,350,614 | 2,294,489 | 2,338,652 | 2,338,652 |
| gzip -9 | 542,366 | 972,603 | 992,327 | 992,327 |

Why two modules: wgpu only needs wgpu-core, its GL backend and the naga shader translator (WGSL
parser, validator, GLSL writer) when it has to run WebGL2 itself; on WebGPU the browser compiles
the WGSL. That machinery was 75% of the combined module. `docs/wasm-size.md` has the profile
(`scripts/wasm-profile.sh`), the experiments and the remaining ideas.
