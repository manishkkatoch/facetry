#!/usr/bin/env bash
# Prints the raw and gzipped size of the built .wasm.
set -euo pipefail
cd "$(dirname "$0")/.."
WASM=packages/facetry/wasm/facetry_wasm_bg.wasm
raw=$(wc -c <"$WASM")
gz=$(gzip -9 -c "$WASM" | wc -c)
echo "wasm raw:  $raw bytes ($((raw / 1024)) KiB)"
echo "wasm gzip: $gz bytes ($((gz / 1024)) KiB)"
