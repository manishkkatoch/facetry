# Facetry

GPU-accelerated charts for JS/TS apps, written in Rust and compiled to WebAssembly.
Rendering uses [wgpu](https://wgpu.rs): WebGPU where the browser really has it, WebGL2 otherwise.

**Status: M1 (2D bar chart).** A `Chart` draws a bar chart from a declarative JSON spec:
validation, band and linear scales with "nice" ticks, layout, axes (lines, ticks, gridlines,
labels, titles), MSDF text and instanced rectangles. Lines, arcs, stacking, legends, tooltips,
animation and 3D come in later milestones.

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

The TypeScript types are generated from the Rust definitions, so they cannot drift.
Example specs live in `examples/specs/` and are shared by the Rust snapshot tests, the
playground and the golden-image test.

## Layout

```
crates/facetry-core     pure Rust: spec + validation, scales, layout, Scene (no GPU/browser deps)
crates/facetry-font     the bundled MSDF font (atlas PNG + metrics JSON) and text measuring
crates/facetry-render   wgpu Renderer: rect + MSDF text pipelines, 4x MSAA (native + wasm32)
crates/facetry-wasm     wasm-bindgen glue: the `Chart` class and typed errors
tools/font-atlas        offline tool that generates the MSDF atlas (not part of the build)
packages/facetry        npm package (TypeScript): loads wasm once, ResizeObserver + DPR
                        sizing, renders only when dirty
examples/specs          example chart specs (JSON)
examples/playground     Vite app + Playwright tests (smoke, playground, golden image)
scripts/                build-wasm.sh, wasm-size.sh
```

How a frame is made: `spec` -> `validate` -> `build_scene(spec, viewport, dpr, font metrics)` ->
`Scene` (flat lists of keyed rects, rules and texts in device pixels) -> `Renderer::set_scene`
(instance buffers) -> one render pass: gridlines, bars, axis rules, then text. Resizing rebuilds
the scene, because margins, tick counts and label thinning depend on the size.

## Prerequisites

- Rust stable (the `wasm32-unknown-unknown` target is listed in `rust-toolchain.toml`)
- [wasm-pack](https://rustwasm.github.io/wasm-pack/) (`cargo install wasm-pack`)
- Node 22 and pnpm 10 (`wasm-opt` comes from the `binaryen` npm package)

## Commands

```sh
pnpm install
pnpm build:wasm            # wasm-pack + wasm-opt -Oz -> packages/facetry/wasm, prints size
pnpm -r build              # tsc for the package, tsc + vite build for the playground
pnpm dev                   # playground dev server; add ?backend=webgl2 to force the fallback

cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p facetry-core -p facetry-render -p facetry-wasm --target wasm32-unknown-unknown --all-targets -- -D warnings
cargo test --workspace     # unit, property (proptest) and snapshot (insta) tests

pnpm --filter playground exec playwright install chromium   # once
pnpm test:e2e              # Playwright against `vite preview` (run `pnpm -r build` first)
```

Set `CHROMIUM_PATH` to run the Playwright tests with an already-installed Chromium.

### Playground

`pnpm dev` (or `pnpm -r build && pnpm --filter playground preview`) serves a page with the chart,
a dropdown of the example specs, and a JSON editor: edit the spec and press **Apply** to call
`chart.update`; validation issues are listed with their JSON-pointer paths. URL parameters:
`?backend=webgl2` (force the fallback), `?example=bar-colored`, `?ui=0` (only the chart).

### Snapshots and golden images

- **Scene snapshots** (`crates/facetry-core/tests/snapshots`) pin the exact layout of every
  example spec. After an intentional layout change: `INSTA_UPDATE=always cargo test -p facetry-core`
  (or `cargo insta review`), then look at the diff in git before committing. CI sets
  `INSTA_UPDATE=no`, so any difference fails.
- **Golden image** (`examples/playground/tests/__golden__/bar-basic.png`): Playwright screenshots
  the basic bar example in Chromium at 800x500, devicePixelRatio 1, `?backend=webgl2`, and allows
  1% of pixels to differ. To regenerate after an intentional visual change:
  `pnpm -r build && pnpm --filter playground exec playwright test golden --update-snapshots`,
  then **look at the PNG** before committing it.

### Regenerating the font atlas

The text atlas is generated offline and committed; nothing at build or run time needs the font.
Only needed if you change the font, glyph set, atlas size or distance range (constants in
`tools/font-atlas/src/main.rs`):

```sh
cargo run -p font-atlas --release
```

This rewrites `crates/facetry-font/assets/inter-regular-msdf.png` and
`inter-regular-metrics.json` from `tools/font-atlas/fonts/Inter-Regular.ttf` (Inter 4.1, SIL Open
Font License 1.1, licence next to it). Rebuild the wasm afterwards (the files are embedded with
`include_bytes!`) and update the scene snapshots, since layout uses the glyph widths.

## Bundle size

Release build (`opt-level = "z"`, LTO) after `wasm-opt -Oz`, measured with `scripts/wasm-size.sh`:

| | size |
|---|---|
| | M0 | M1 |
|---|---|---|
| `facetry_wasm_bg.wasm` raw | 1,350,614 bytes (1318 KiB) | 2,294,489 bytes (2240 KiB) |
| gzip -9 | 542,366 bytes (529 KiB) | 972,603 bytes (949 KiB) |

Most of that is wgpu (wgpu-core and the GL backend, kept so the WebGL2 fallback works). M1 roughly
doubled it: it now ships shaders, so the wasm includes the shader translator (WGSL parser,
validator, GLSL writer; wgpu needs it at runtime to compile our WGSL for WebGL2), plus the
embedded font atlas (~77 KB PNG), serde and a PNG decoder. (Not yet profiled symbol by symbol.)
