#!/usr/bin/env bash
# Builds crates/facetry-wasm twice and crates/facetry-diagrams-wasm once into
# packages/facetry/wasm (wasm-pack, --target web) and shrinks them with wasm-opt -Oz:
#   facetry_wasm      WebGPU only: what almost every visitor downloads (small)
#   facetry_wasm_gl   WebGL2 only: loaded on demand where WebGPU is missing (large: wgpu-core,
#                     the GL backend and the naga shader translator)
#   facetry_diagrams  the Mermaid add-on (parser and layout, no GPU code): loaded only by pages
#                     that draw diagrams (`facetry/diagrams`)
# See docs/wasm-size.md for why they are separate.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=packages/facetry/wasm
if command -v wasm-opt >/dev/null; then OPT=(wasm-opt); else OPT=(pnpm exec wasm-opt); fi

build() { # build NAME CRATE CARGO_ARGS...
  local name=$1 crate=$2
  shift 2
  wasm-pack build "crates/$crate" --target web --release --no-pack \
    --out-dir "../../$OUT" --out-name "$name" -- --no-default-features "$@"
  local wasm="$OUT/${name}_bg.wasm"
  # Features the Rust toolchain emits that wasm-opt must be told to accept.
  "${OPT[@]}" -Oz --enable-bulk-memory --enable-nontrapping-float-to-int \
    --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue \
    -o "$wasm.opt" "$wasm"
  mv "$wasm.opt" "$wasm"
}

build facetry_wasm facetry-wasm --features webgpu
build facetry_wasm_gl facetry-wasm --features webgl
build facetry_diagrams facetry-diagrams-wasm
scripts/wasm-size.sh
