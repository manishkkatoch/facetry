#!/usr/bin/env bash
# Where do the wasm bytes go? Builds the same release wasm as scripts/build-wasm.sh but
# keeps the `name` section (and nothing else), then summarises it with twiggy.
#
#   scripts/wasm-profile.sh [TOP_N]      TOP_N = rows in the `twiggy top` table (default 40)
#
# Outputs, under target/wasm-profile/ (git-ignored):
#   facetry_wasm_bg.named.wasm   wasm-opt -Oz -g output (function names kept; larger)
#   facetry_wasm_bg.wasm         same bytes with names stripped: must equal the normal build
#   twiggy-top.txt, twiggy-dominators.txt, rollup.txt
#
# The normal build is untouched: this uses its own cargo target dir and only overrides the
# release profile through environment variables (strip=none keeps wasm-ld's name section).
# Needs: cargo, the wasm32 target, the wasm-bindgen CLI matching Cargo.lock (wasm-pack
# installs it under ~/.cache/.wasm-pack), wasm-opt (binaryen), twiggy, python3.
set -euo pipefail
cd "$(dirname "$0")/.."

TOP=${1:-40}
OUT=target/wasm-profile
mkdir -p "$OUT"

if command -v wasm-bindgen >/dev/null; then BINDGEN=wasm-bindgen
else BINDGEN=$(ls -d "$HOME"/.cache/.wasm-pack/wasm-bindgen-cargo-install-*/wasm-bindgen | tail -n1); fi
if command -v wasm-opt >/dev/null; then OPT=(wasm-opt); else OPT=(pnpm exec wasm-opt); fi
FEATURES=(--enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext
  --enable-mutable-globals --enable-reference-types --enable-multivalue)

CARGO_TARGET_DIR="$OUT/cargo" CARGO_PROFILE_RELEASE_STRIP=none \
  cargo build -p facetry-wasm --target wasm32-unknown-unknown --release
"$BINDGEN" --target web --out-dir "$OUT/pkg" --out-name facetry_wasm \
  "$OUT/cargo/wasm32-unknown-unknown/release/facetry_wasm.wasm"

# -g keeps the name section through the optimiser; without it names are dropped.
"${OPT[@]}" -Oz -g "${FEATURES[@]}" -o "$OUT/facetry_wasm_bg.named.wasm" "$OUT/pkg/facetry_wasm_bg.wasm"
"${OPT[@]}" -Oz "${FEATURES[@]}" -o "$OUT/facetry_wasm_bg.wasm" "$OUT/pkg/facetry_wasm_bg.wasm"

echo "named (with name section): $(wc -c <"$OUT/facetry_wasm_bg.named.wasm") bytes"
echo "stripped (comparable to the shipped build): $(wc -c <"$OUT/facetry_wasm_bg.wasm") bytes"
echo "shipped build (scripts/build-wasm.sh), if present: $(wc -c <packages/facetry/wasm/facetry_wasm_bg.wasm 2>/dev/null || echo n/a) bytes"

NAMED="$OUT/facetry_wasm_bg.named.wasm"
twiggy top -n "$TOP" "$NAMED" | tee "$OUT/twiggy-top.txt"
twiggy dominators -d 3 -r 40 "$NAMED" >"$OUT/twiggy-dominators.txt"
twiggy top -n 1000000 --format csv "$NAMED" >"$OUT/twiggy-all.csv"
python3 -I scripts/wasm-rollup.py "$OUT/twiggy-all.csv" | tee "$OUT/rollup.txt"
