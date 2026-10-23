#!/usr/bin/env bash
# Builds crates/facetry-wasm into packages/facetry/wasm (wasm-pack, --target web), then
# shrinks the .wasm with wasm-opt -Oz.
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=packages/facetry/wasm
wasm-pack build crates/facetry-wasm --target web --release --no-pack \
  --out-dir "../../$OUT" --out-name facetry_wasm

WASM="$OUT/facetry_wasm_bg.wasm"
if command -v wasm-opt >/dev/null; then OPT=(wasm-opt); else OPT=(pnpm exec wasm-opt); fi
# Features the Rust toolchain emits that wasm-opt must be told to accept.
"${OPT[@]}" -Oz --enable-bulk-memory --enable-nontrapping-float-to-int \
  --enable-sign-ext --enable-mutable-globals --enable-reference-types --enable-multivalue \
  -o "$WASM.opt" "$WASM"
mv "$WASM.opt" "$WASM"
scripts/wasm-size.sh
