#!/usr/bin/env bash
# Prints the raw, gzip -9 and brotli -q 11 size of both built .wasm files.
# (brotli comes from the `brotli` CLI if installed, else Node's zlib.)
set -euo pipefail
cd "$(dirname "$0")/.."
for name in facetry_wasm facetry_wasm_gl; do
  wasm=packages/facetry/wasm/${name}_bg.wasm
  raw=$(wc -c <"$wasm")
  gz=$(gzip -9 -c "$wasm" | wc -c)
  if command -v brotli >/dev/null; then
    br=$(brotli -q 11 -c "$wasm" | wc -c)
  else
    br=$(node -e 'const z=require("zlib"),b=require("fs").readFileSync(process.argv[1]);console.log(z.brotliCompressSync(b,{params:{[z.constants.BROTLI_PARAM_QUALITY]:11,[z.constants.BROTLI_PARAM_LGWIN]:24,[z.constants.BROTLI_PARAM_SIZE_HINT]:b.length}}).length)' "$wasm")
  fi
  printf '%-16s raw %9d  gzip %8d  brotli %8d bytes\n' "$name" "$raw" "$gz" "$br"
done
