/**
 * The package ships two wasm modules built from the same Rust, differing only in which GPU
 * backend wgpu contains (see docs/wasm-size.md):
 *
 * - `webgpu`: WebGPU only, about a quarter of the size. Where WebGPU exists, this is all that
 *   is ever downloaded.
 * - `webgl2`: WebGL2 only, the fallback. It carries wgpu-core, the GL backend and the shader
 *   translator, so it is large and must never be fetched by a WebGPU user.
 *
 * Both are loaded with a dynamic `import()` (bundlers split each glue file and wasm asset into
 * its own chunk), at most once per page, and only when first needed.
 */
type Glue = {
  /** wasm-bindgen's init: fetches and instantiates the `.wasm` next to the glue file. */
  default(): Promise<unknown>;
  Chart: typeof import("../wasm/facetry_wasm.js").Chart;
};

export type ModuleKind = "webgpu" | "webgl2";

const importers: Record<ModuleKind, () => Promise<Glue>> = {
  webgpu: () => import("../wasm/facetry_wasm.js"),
  // Same exports (same Rust source, different out-name); `Glue` is the part we use.
  webgl2: () => import("../wasm/facetry_wasm_gl.js"),
};

const loaded = new Map<ModuleKind, Promise<Glue>>();

/** Fetches and instantiates a module once per page; later calls share the promise. */
export function loadModule(kind: ModuleKind): Promise<Glue> {
  let pending = loaded.get(kind);
  if (!pending) {
    pending = importers[kind]().then(async (glue) => {
      await glue.default();
      return glue;
    });
    pending.catch(() => loaded.delete(kind)); // allow a retry after a network failure
    loaded.set(kind, pending);
  }
  return pending;
}

let probe: Promise<boolean> | undefined;

/**
 * Whether WebGPU looks usable: `navigator.gpu` exists and hands out an adapter. A cheap
 * check made before downloading anything; it can still be a false positive (the device or
 * surface may fail later), which `Chart.create` handles by retrying on the WebGL2 module.
 */
export function webgpuLooksUsable(): Promise<boolean> {
  probe ??= (async () => {
    try {
      const gpu = (navigator as { gpu?: { requestAdapter(): Promise<unknown> } }).gpu;
      return gpu !== undefined && (await gpu.requestAdapter()) != null;
    } catch {
      return false;
    }
  })();
  return probe;
}
