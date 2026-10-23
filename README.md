# Facetry

GPU-accelerated charts for JS/TS apps, written in Rust and compiled to WebAssembly.
Rendering uses [wgpu](https://wgpu.rs): WebGPU where the browser really has it, WebGL2 otherwise.

**Status: M0 (toolchain).** The only thing a `Chart` does today is create a GPU canvas
and clear it to a colour. Bar, line and pie charts (2D and 3D) come in later milestones.

```ts
import { Chart } from "facetry";

const chart = await Chart.create(canvas, { backend: "auto", clearColor: "#336699" });
console.log(chart.backend); // "webgpu" | "webgl2"
// ...
chart.destroy();
```

`Chart.create` rejects with a `FacetryError` whose `code` is `gpu-unavailable`,
`invalid-options` or `render-failed`.

## Layout

```
crates/facetry-core     pure Rust: Color, Theme (no GPU/browser deps)
crates/facetry-render   wgpu Renderer (native + wasm32)
crates/facetry-wasm     wasm-bindgen glue: the `Chart` class and typed errors
packages/facetry        npm package (TypeScript): loads wasm once, ResizeObserver + DPR
                        sizing, renders only when dirty
examples/playground     Vite app + Playwright smoke test
scripts/                build-wasm.sh, wasm-size.sh
```

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
cargo test --workspace

pnpm --filter playground exec playwright install chromium   # once
pnpm test:e2e              # smoke test against `vite preview` (run `pnpm -r build` first)
```

Set `CHROMIUM_PATH` to run the Playwright test with an already-installed Chromium.

## Bundle size

Release build (`opt-level = "z"`, LTO) after `wasm-opt -Oz`, measured with `scripts/wasm-size.sh`:

| | size |
|---|---|
| `facetry_wasm_bg.wasm` raw | 1,350,614 bytes (1318 KiB) |
| gzip -9 | 542,366 bytes (529 KiB) |

Most of that is wgpu (wgpu-core and the GL backend, kept so the WebGL2 fallback works).
