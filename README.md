# Facetry

GPU-accelerated charts for JS/TS apps, written in Rust and compiled to WebAssembly.
Rendering uses [wgpu](https://wgpu.rs): WebGPU where the browser really has it, WebGL2 otherwise.

**Status: M1 (2D bar chart) plus M4 (3D bars), built ahead of stacking, lines and pies to get
feedback on the riskiest part first.** A `Chart` draws a bar chart from a declarative JSON spec:
validation, band and linear scales with "nice" ticks, layout, axes (lines, ticks, gridlines,
labels, titles), MSDF text and instanced rectangles; with `dimension: "3d"` the bars are
lit, depth-tested boxes you can orbit (see [3D charts](#3d-charts)). Bars can carry value labels,
report hover and click, and the current view can be saved as a PNG (see [Value labels, hover and
click](#value-labels-hover-and-click) and [PNG snapshots](#png-snapshots)). Lines, arcs, stacking,
legends and animation come in later milestones.

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
- `layers[]`: `{ mark: "bar" | "line" | "arc", data, encoding, position?, point?, style?, label? }`. `x` must be a categorical field (band scale),
  `y` a numeric field (linear scale, includes zero by default). `color` is `{ value: "#hex" }` or
  `{ field }` (categorical field mapped onto the theme palette). `key` is optional; the default
  key of a bar is `layer<n>/<x value>`. Several layers on the same axes overlay.
- `label` (optional, per layer): value labels on the bars, see
  [below](#value-labels-hover-and-click).
- Unknown fields are errors, so typos are caught. Empty data is valid: the axes are drawn (numeric
  domain `[0, 1]`, no categories) and there are no bars.
- Tick labels use thousands separators (`1,200`) and as many decimals as the tick step needs
  (`0.25`, never `0.30000000000000004`).

### Bar positions: stack, normalize, group

When bars share an `x` category, `layers[].position` says how. The series come from
`encoding.color.field` (one series per distinct value, in the order they first appear in the
column), so the three non-default positions need it.

```jsonc
{
  "data": { "sales": { "quarter": ["Q1", "Q1", "Q2", "Q2"], "product": ["Hardware", "Software", "Hardware", "Software"],
                       "revenue": [120, 80, 90, 95] } },
  "axes": [ { "id": "x", "orient": "bottom" }, { "id": "y", "orient": "left", "title": "Revenue" } ],
  "layers": [{
    "mark": "bar", "data": "sales",
    "position": "stack",                               // "overlay" (default) | "stack" | "normalize" | "group"
    "encoding": { "x": { "field": "quarter" }, "y": { "field": "revenue" }, "color": { "field": "product" } },
    "label": { "show": "always" }
  }]
}
```

- `overlay` (default): one bar per row, bars of the same x overlap. Nothing changes.
- `stack`: the series pile up in series order, the first at the bottom. **Negative values stack
  downward from zero**, separately from the positive ones, so a negative never cuts into the
  positive stack. The axis covers the stack totals.
- `normalize`: stacked, each x scaled to 100%. The axis is `0%` to `100%`. Negative values have no
  share, so they are a validation error (`/layers/0/encoding/y/field`, naming the first
  offending row); use `stack` for mixed signs. A percent axis cannot be shared with a layer that
  shows raw values (give one of them its own axis).
- `group`: one slot per series inside each x band (every series keeps its slot even where it has
  no row), with the width and spacing from a band scale. Bars start at zero; the axis covers the
  largest (summed) value.
- Rows with the same x, series (and z in 3D) are **summed** first for the three non-default
  positions. Hover and click then report the first of those rows with its y value replaced by the
  sum.
- Keys are `layer0/<x>/<series>` (`layer0/<x>/<series>/<z>` in 3D), so they stay stable across
  updates; an `encoding.key` field replaces the x part.
- Value labels: a stacked or normalised segment is labelled **centred inside it** with the
  segment's value (normalised: its percentage, `33.3%`), and **no label is drawn when the text
  does not fit** in the segment; grouped bars are labelled above their ends like plain bars,
  except where the text is wider than the slot. There are no totals. In 3D the same rule is
  decided at the starting camera, and a segment's label is also hidden **while another box is
  in front of the segment's middle** (the renderer tests that per frame with the picking ray,
  so stacks seen from above only show their top segment's number); `label.format.percent: true`
  prints a segment's share of its stack.
- 3D: `stack`/`normalize` pile boxes up inside each (x, z) cell, `group` splits the x cell into
  sub-cells (z stays one cell).

### Lines, and bars with lines

`mark: "line"` draws one polyline per series (`encoding.color.field`; without it, one line):

```jsonc
{
  "data": { "kpi": { "quarter": ["Q1", "Q2", "Q3", "Q4"], "revenue": [120, 90, 140, 110], "margin": [12, 8, 21, 16] } },
  "axes": [
    { "id": "x", "orient": "bottom", "title": "Quarter" },
    { "id": "revenue", "orient": "left", "title": "Revenue" },
    { "id": "margin", "orient": "right", "title": "Margin %" }
  ],
  "layers": [
    { "mark": "bar", "data": "kpi",
      "encoding": { "x": { "field": "quarter", "axis": "x" }, "y": { "field": "revenue", "axis": "revenue" }, "color": { "value": "#4C78A8" } } },
    { "mark": "line", "data": "kpi",
      "point": { "size": 9 },                // markers: true | { size } (diameter in CSS px, default 8)
      "style": { "strokeWidth": 2.5 },       // CSS px, default 2
      "label": { "show": "hover" },
      "encoding": { "x": { "field": "quarter", "axis": "x" }, "y": { "field": "margin", "axis": "margin" }, "color": { "value": "#F58518" } } }
  ]
}
```

- `x` is categorical (a band scale: points sit at the band centres, so a line lines up with bars
  of the same categories) **or numeric** (a linear scale with nice ticks, and the bottom axis
  takes `scale` overrides like a y axis; unlike y it does not include 0 unless `zero: true`).
  Whole-number data gets whole-number ticks, and year-like ticks read `2015`, not `2,015`.
  Layers sharing a bottom axis must agree on the kind of `x`.
- Each series is one polyline in the order of `x` (categorical: the axis order; numeric:
  ascending). **A missing y (`null`) breaks the line** (a series with no row at some category
  is simply connected across it). A run of one point is drawn as a dot. Values outside a fixed
  `scale.domain` are clamped to the plot edge.
- Draw order: gridlines, bars, lines, markers, then axis lines and ticks, hover marker and
  labels. A layer is bound to a left or right axis by id as for bars, so a bar and a line layer
  can use different y scales.
- `label`: the value above each point (below it where there is no room); `"hover"` shows only
  the hovered point's. Hover and click: the point nearest the pointer **within 12 CSS px** wins
  (even where a bar is behind it), otherwise the bar under the pointer. The hovered point gets
  a larger marker. Keys are `layer1/<x>` (`layer1/<x>/<series>` with a colour field).
- 3D has no lines yet: `mark: "line"` with `dimension: "3d"` is a validation error
  (`line charts are 2D only for now`).
- How the strokes are drawn: the CPU turns each polyline into triangles in device pixels
  (`facetry-render/src/stroke.rs`), with miter joins that become bevels beyond a miter limit of 4
  line widths (so a zigzag does not grow spikes), butt caps, and neighbouring segments sharing
  their inner corners so a translucent colour does not double up at a bend. One draw call per
  layer; 4x MSAA smooths the edges. Only WebGL2-compatible features are used.

### Pie and donut charts

`mark: "arc"` with `encoding.theta` (the numeric field that sizes each slice) and
`encoding.color.field` (one slice per category). A pie has no axes:

```jsonc
{
  "data": { "traffic": { "source": ["Search", "Social", "Direct", "Email"], "visits": [420, 280, 190, 60] } },
  "layers": [{
    "mark": "arc", "data": "traffic",
    "style": { "innerRadius": 0.55, "padAngle": 1.5, "startAngle": 0 },   // all optional
    "encoding": { "theta": { "field": "visits" }, "color": { "field": "source" }, "key": { "field": "source" } },
    "label": { "show": "always", "format": { "percent": true } }
  }]
}
```

- Slices follow the colour field's order, clockwise from `startAngle` (degrees, `0` = 12
  o'clock). `innerRadius` is the hole as a fraction of the outer radius (`0` pie, `> 0` donut,
  below 1); `padAngle` is the gap between slices in degrees (half is cut off each side).
  `cornerRadius` is not supported yet. Rows with the same colour value are summed into one slice
  (hover reports the first row with the summed `visits`); zero and missing values are no slice.
  Slice keys are `layer0/<colour value>` (an `encoding.key` field replaces the value).
- `theta` must be numeric and not negative, `color` a categorical field; an arc layer cannot have
  `x`/`y`/`z`, cannot be mixed with other layers (or a second arc layer), and a spec with an arc
  layer **must not declare `axes`**: each is a validation issue with its path.
- `label.format.percent: true` prints each slice's share (`42.8%`, at most one decimal; `decimals`
  overrides) instead of its value. A label goes **inside** its slice, centred in the ring (at
  0.62 of the radius for a pie), when the text fits there; otherwise it goes **outside** on a
  short leader line (the pie shrinks to leave room for those labels). A label that would touch
  an outside label already placed is left out; with `show: "hover"` there are no leader lines
  and no overlap pruning, since only one label shows at a time.
- 2D: CPU tessellation into triangles, segments of at most 2 degrees (at least 3 per slice);
  neighbouring slices share their boundary vertices, so there are no seams. Picking is a
  point-in-sector test (angle and radius); the hovered slice is drawn again a little lighter.
- **3D** (`dimension: "3d"`): an extruded pie standing on the floor, thickness `style.depth` (a
  fraction of the radius, default 0.2). It is a lit mesh with a top face, an outer wall, the hole's
  wall for a donut, and the flat faces at each slice's edges (no bottom: the camera never goes
  below the floor); curved walls use smooth radial normals, the rest flat ones. There are no
  axes or back walls. The orbit camera works as for bars. Picking intersects the ray with the
  plane of the top face and tests the slice's sector there, so **the sides of an extruded pie
  are not pickable** (from the elevations the camera allows, the top is nearly always what the
  pointer is over). 3D labels sit on the top face (outside ones centred beyond a leader line, so
  they read from any side); what fits is judged at the starting camera.
- Why a new lit-mesh pipeline for 3D: the bars' pipeline stretches one static unit cube to
  every bar, which cannot express curved walls. `mesh3d.wgsl` has the same Lambert lighting as
  `box.wgsl` and takes ready-made vertices (position, normal, colour, hover flag) from
  `facetry-render/src/arc.rs`.

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
floor edge carries each axis; both floor axes sit on the two edges at the nearest corner, which
form the lower outline of the picture) is emitted for each 45 degree sector of azimuth, tagged
with its range, and the vertex shaders drop the variants the camera is not in. Labels are
projected from their anchor and then laid out in screen pixels, so they face the viewer and keep
their size; they draw last, without depth testing, with an outline in the background colour so
they stay legible over bars.

**Axis labels follow the camera.** Which tick labels are readable depends on the viewpoint: look
along an axis and its labels land on top of each other. So every frame where the camera or size
changed, the renderer computes, for the labels of the sector the camera is in, a small table:
*draw it?* and *nudge it by how many pixels?*, and uploads it (4 KiB) for the shader to read. The
glyph instances are never rewritten and the scene is not rebuilt. The rules
(`crates/facetry-render/src/labels.rs`): a label is moved away from the plot box's projected
centre, in screen space, just far enough to clear the box's projected outline (so no axis label
is drawn over a bar); labels are then visited by priority (axis titles, y ticks, then the other
axes' ticks, each axis first label, last label, middle, the middles of the halves, ...) and a
label whose rectangle would touch one already kept is hidden. An axis seen end-on therefore keeps
its first and last label (or just one) and its title; seen broadside it keeps as many as fit. It
runs on the CPU because it takes a few dozen rectangle tests and is easy to unit-test there
(the tests sweep the camera and check that no two visible labels overlap and none touches the
box). Known limit: at very high elevations the axis lines themselves (not their labels) can pass
over bars, and an axis with more than about 100 categories is labelled every n-th category.

The TypeScript types are generated from the Rust definitions, so they cannot drift.
Example specs live in `examples/specs/` and are shared by the Rust snapshot tests, the
playground and the golden-image test.

## Value labels, hover and click

A layer can write each bar's value on it:

```jsonc
"layers": [{
  "mark": "bar", "data": "sales", "encoding": { /* ... */ },
  "label": { "show": "always", "format": { "decimals": 1 } }   // "always" | "hover"; format optional
}]
```

- `show: "always"` labels every bar; `"hover"` only the bar under the pointer. No `label` block,
  no labels. `format.decimals` (0 to 10) overrides the default, which uses as many decimals as the
  value needs (up to 4); numbers use the axis style (`1,200`).
- 2D: centred above the bar (below a negative bar); if that would leave the plot it goes inside
  the bar's end, in whichever of the theme's text and background colours contrasts more with the
  bar. 3D: hanging above the middle of the top face (below the bottom face of a negative bar),
  outlined like the axis labels.
- Bad values are validation errors with JSON-pointer paths (`/layers/0/label/show`,
  `/layers/0/label/format/decimals`).

```ts
chart.on("hover", (mark) => { /* mark: { key, layer, datum } | null */ });
chart.on("click", (mark) => console.log(mark.datum));   // { quarter: "Q3", revenue: 140 }
chart.off("hover", listener);
chart.markPosition("layer0/Q3");                        // { x, y } in CSS px, to aim at a bar
```

`hover` fires when the bar under the pointer changes, with `null` when the pointer leaves every
bar (and when the chart is laid out again: after `update` or a resize you get `null`, then the bar
that is now under the pointer). `datum` is the bar's row of the layer's table as a plain object
(`null` for a missing number). `click` fires on a press and release that moved less than 4 CSS
pixels, with `null` for a click on empty space; **a drag that orbits a 3D chart never clicks**.
A throwing listener is logged and does not affect the others.

The hovered bar is lightened (a per-instance flag, rewritten alone) and a `hover` label shows
(a handful of glyphs in a small overlay buffer); the scene is not rebuilt and nothing else is
uploaded. **Picking is done on the CPU**, with the same matrix the GPU draws with: in 2D a
point-in-rectangle test (`Scene::pick`), in 3D a ray from the camera through the pixel intersected
with every bar's box by the slab test, nearest hit wins (`facetry-render/src/pick.rs`). With at
most about a thousand bars that is microseconds, needs no GPU read-back (a colour-ID pass would
cost a frame of latency and an async buffer map per pointer move) and is covered by unit tests.
The maths lives in the render crate because it needs the camera and `glam`; the 2D test is in
core next to the scene it reads.

## PNG snapshots

```ts
const blob = await chart.snapshot();                              // PNG of the chart as it is now
const big = await chart.snapshot({ scale: 2, background: "transparent" });
await chart.downloadPNG("sales.png", { scale: 2 });                  // saves a file
```

`snapshot` captures exactly the current state: the spec and, in 3D, the current orbit camera,
without the hover highlight or hover-only labels. `scale` is pixels per CSS pixel (default: the
current `devicePixelRatio`, which reproduces the canvas; at most 4 and never beyond the GPU's
texture limit); `background` is `"theme"` (default) or `"transparent"`. The picture is not
stretched: the chart is laid out again at the snapshot's size, so text stays crisp and margins
and tick counts suit it.

How: the renderer draws the re-laid-out scene with the current camera into an offscreen texture
of its own (separate multisampled colour, resolve and depth targets, *in the same format as the
canvas* so the same pipelines and colour handling apply and the exported colours are the ones on
screen), copies it to a buffer (rows padded to 256 bytes), maps it asynchronously and un-pads the
rows, swaps BGRA to RGBA when needed and, for a transparent background, divides the colours by
alpha (in linear light for sRGB targets, where the GPU blended in linear light). Then the on-screen
layout is put back, so the canvas keeps rendering as before. TypeScript hands the RGBA bytes to
the browser's own PNG encoder (`OffscreenCanvas.convertToBlob`, or `canvas.toBlob`): no PNG
encoder is added to the wasm (see [Bundle size](#bundle-size)). Snapshots queue: concurrent calls
run one after the other, each showing the state at the time it was called.

Tested in headless Chromium on the WebGL2 backend (an sRGB RGBA surface): sizes, scales, the
transparent background, and agreement with the screen. The format conversions that matter on
WebGPU (a non-sRGB, usually BGRA, surface) are unit-tested, but that path has not been run on a
real WebGPU device.

## Layout

```
crates/facetry-core     pure Rust: spec + validation, scales, layout, Scene and Scene3d
                        (no GPU/browser deps)
crates/facetry-font     the bundled MSDF font (raw RGB atlas + metrics JSON) and text measuring
crates/facetry-render   wgpu Renderer: rect + MSDF text pipelines (2D); box, line and world-text
                        pipelines plus the glam orbit camera (3D); 4x MSAA (native + wasm32);
                        3D label placement (labels.rs), picking (pick.rs), PNG read-back
crates/facetry-wasm     wasm-bindgen glue: the `Chart` class and typed errors; features `webgpu`
                        (default) and `webgl` choose the backend, built once per module
tools/font-atlas        offline tool that generates the MSDF atlas (not part of the build)
packages/facetry        npm package (TypeScript): lazily loads one of two wasm modules, ResizeObserver + DPR
                        sizing, orbit/zoom/reset gestures, hover/click events, PNG snapshots,
                        renders only when dirty
examples/specs          example chart specs (JSON)
examples/playground     Vite app + Playwright tests (smoke, playground, golden images, orbit, hover,
                        snapshot)
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
`?backend=webgl2` (force the fallback), `?example=bar-colored` (or `bar-labels`, `bar3d-grid`,
`bar3d-labels`, `bar3d-row`, `bar3d-negative`), `?ui=0` (only the chart). 3D examples show the
current camera in the toolbar; hovering a bar shows its datum next to it; **Save PNG** downloads
the chart as it is now. `facetryChart` on `window` is the live chart for poking at from the
console.

### Snapshots and golden images

- **Scene snapshots** (`crates/facetry-core/tests/snapshots`) pin the exact layout of every
  example spec. After an intentional layout change: `INSTA_UPDATE=always cargo test -p facetry-core`
  (or `cargo insta review`), then look at the diff in git before committing. CI sets
  `INSTA_UPDATE=no`, so any difference fails.
- **Golden images** (`examples/playground/tests/__golden__/bar-basic.png`, `bar-labels.png` (value
  labels, `always`) and `bar3d-grid.png`): Playwright screenshots the basic bar example, the
  labelled one and the 3D grid example (default camera) in
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
