#!/usr/bin/env bash
# Builds crates/facetry-wasm twice into packages/facetry/wasm (wasm-pack, --target web) and
# shrinks both with wasm-opt -Oz:
#   facetry_wasm     WebGPU only: what almost every visitor downloads (small)
#   facetry_wasm_gl  WebGL2 only: loaded on demand where WebGPU is missing (large: wgpu-core,
#                    the GL backend and the naga shader translator)
# See docs/wasm-size.md for why they are separate.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=packages/facetry/wasm
if command -v wasm-opt >/dev/null; then OPT=(wasm-opt); else OPT=(pnpm exec wasm-opt); fi

build() { # build NAME CARGO_ARGS...
  local name=$1
  shift
  wasm-pack build crates/facetry-wasm --target web --release --no-pack \
    --out-dir "../../$OUT" --out-name "$name" -- --no-default-features "$@"
  local wasm="$OUT/${name}_bg.wasm"
  # Features the Rust toolchain emits that wasm-opt must be told to accept.
  "${OPT[@]}" -Oz --enable-bulk-memory --enable-nontrapping-float-to-int \
    --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue \
    -o "$wasm.opt" "$wasm"
  mv "$wasm.opt" "$wasm"
}

build facetry_wasm --features webgpu
build facetry_wasm_gl --features webgl
scripts/wasm-size.sh
