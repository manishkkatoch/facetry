import init from "../wasm/facetry_wasm.js";

let loading: Promise<unknown> | undefined;

/** Fetches and instantiates the wasm module once per page; later calls share the promise. */
export function loadWasm(): Promise<unknown> {
  loading ??= init().catch((e: unknown) => {
    loading = undefined; // allow a retry after a network failure
    throw e;
  });
  return loading;
}
