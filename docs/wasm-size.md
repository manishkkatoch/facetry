# Facetry wasm size: what is in it and how to shrink it

Investigation of `facetry_wasm_bg.wasm` at commit `4093a7b` (3D bar charts), built with
`scripts/build-wasm.sh` (wasm-pack `--release`, then `wasm-opt -Oz`). Nothing in the product
code or the normal build was changed; the recommendations below are not applied.

Toolchain: rustc 1.97.0, wgpu 30.0.1, wasm-bindgen 0.2.129, binaryen 132 (npm), twiggy.
All sizes are bytes. "gzip" is `gzip -9`, "brotli" is brotli quality 11, window 2^24
(Node zlib; no `brotli` binary was available).

## Summary

* **The WebGL2 fallback is 75% of the file.** wgpu only needs `wgpu-core`, `wgpu-hal` (gles)
  and `naga` (WGSL parser + validator + GLSL writer) when it has to run the GL backend itself.
  On WebGPU the browser compiles the WGSL and validates the calls, and wgpu's WebGPU backend
  is a thin shim (about 25 KB of code). Dropping wgpu's `webgl` feature takes the module from
  2,338,652 to **577,999 bytes raw (-75%), 992,317 to 294,450 gzip (-70%), 791,216 to 256,484
  brotli**. Dropping `webgpu` instead only saves 65 KB raw / 28 KB gzip.
* **Biggest single fix:** ship a WebGPU-only module as the default and load a WebGL2 module
  only on browsers without WebGPU. Measured primary build: 294 KB gzip, 256 KB brotli.
* **Two cheap wins stack on top** (measured): store the font atlas as raw RGB instead of a PNG
  (-39 KB gzip) and build `log` with `release_max_level_off` (-29 KB gzip, but only in the
  WebGL build). WebGPU-only plus the atlas change is **254 KB gzip / 212 KB brotli**.
* **What caused the M0 to M1 jump (+430 KB gzip):** roughly half is naga/wgpu-core code and
  tables becoming reachable once shaders exist (`wgsl` frontend, validator, GLSL writer,
  pipeline/texture/bind-group paths), and about 120 KB gzip (28%) is the embedded 77 KB MSDF
  atlas plus the `png`/`fdeflate` decoder. The rest is serde (JSON metrics parse, derived
  deserialisers, `serde_path_to_error`) and our own code. Details below.
* Compiler knobs are already optimal: `opt-level="z"` + `wasm-opt -Oz` is the smallest
  gzipped of everything tried (`"s"` is +83 KB gzip, `"3"` +208 KB). `tsify` costs nothing at
  runtime.

## How to reproduce

```
scripts/wasm-profile.sh [TOP_N]    # ~2 min cold; output in target/wasm-profile-webgl/ (git-ignored)
                                   # PROFILE_MODULE=webgpu profiles the small module instead
```

It builds the same release wasm with its own cargo target dir and
`CARGO_PROFILE_RELEASE_STRIP=none` (so wasm-ld keeps the `name` section), runs
`wasm-opt -Oz -g` to keep names through the optimiser, and runs `twiggy top`,
`twiggy dominators` and a per-crate rollup (`scripts/wasm-rollup.py`, groups the demangled
function names by crate). It also produces a stripped copy and prints its size next to the
shipped build as a sanity check: **2,338,967 stripped vs 2,338,652 shipped (+315 bytes,
0.013%)**, so the named build is a faithful stand-in. Names add 942 KB of `name` section to
the profiling build, so shares below are of *code* (function bodies, 1,788,292 bytes), not
of the named file.

Why the earlier `twiggy` attempt failed: the shipped file has no `name` section, so every
function shows up as `code[N]`. `strip=none` at the cargo level is enough; no `debug=` is
needed.

## Where the bytes go (current build)

The shipped file is roughly 76% code (1.79 MB), 21% data segments (486 KB) and 3% structure
(types, imports/exports, tables).

### Code, by crate (function bodies = 1,788,292 B)

| Component | Bytes | % of code |
|---|---:|---:|
| naga (WGSL frontend, validator, const-evaluator, GLSL writer) | 558,915 | 31.3% |
| std / alloc / core (includes generics monomorphised for everyone else) | 300,327 | 16.8% |
| wgpu API crate (of which `wgpu::backend::wgpu_core` 234,722; `wgpu::backend::webgpu` 25,376) | 281,603 | 15.7% |
| wgpu-core | 185,799 | 10.4% |
| wgpu-hal (gles backend + `glow` WebGL bindings) | 137,339 | 7.7% |
| hashbrown / foldhash / slotmap / smallvec / bitflags etc. | 99,647 | 5.6% |
| facetry_* (incl. wasm-bindgen glue generated for our API) | 86,353 | 4.8% |
| png + fdeflate/miniz/crc/adler | 49,898 | 2.8% |
| serde-derived deserialisers/visitors for facetry types (incl. `serde_path_to_error` wrappers) | 46,236 | 2.6% |
| serde_json | 16,141 | 0.9% |
| wasm-bindgen / js-sys / web-sys | 11,702 | 0.7% |
| serde_path_to_error | 6,056 | 0.3% |
| serde-wasm-bindgen | 3,777 | 0.2% |
| serde core, other | 4,499 | 0.3% |

Second level for the big ones (same denominator): `naga::front::wgsl` 171,909 (9.6%),
`naga::proc::constant_evaluator` 102,095 (5.7%), `naga::valid` 88,634 (5.0%),
`naga::back::glsl` 60,628 (3.4%), `wgpu_hal::gles` 106,270 (5.9%).

Everything that exists only for the WebGL path (naga + `wgpu::backend::wgpu_core` +
wgpu-core + wgpu-hal) is **1,116,775 B = 62.5% of code** before counting the share of
std/hashbrown that it drags in. The ablation below shows the real number: 1.76 MB raw (75%
of the file) disappears with the `webgl` feature.

Caveats on attribution: with `lto=true` and `opt-level="z"`, wgpu-core code is inlined into
`wgpu::backend::wgpu_core::*` wrappers (e.g. `CoreCommandEncoder::finish` is one 87 KB
function), so "wgpu API" and "wgpu-core" are really one block; crate attribution is by
name prefix (first path segment; `<T as Trait>` and `drop_in_place<T>` are attributed to T).
std/alloc/core is overstated by monomorphised generics belonging to other crates.
`twiggy dominators` is not informative here: 794,636 B (24%) are retained by the function
table (`table[0]`, from wgpu's `dyn` device/queue/encoder traits), because every backend
implementation is reachable through vtables, which is also why the linker cannot drop the GL
path by itself.

### Data segments (485,674 B = 20.8% of the file)

Known items: font atlas PNG 77,352; font metrics JSON 21,452; WGSL sources 12,586.
The remaining ~375 KB is strings and tables from naga (keyword/builtin tables, parse and
validation error messages), wgpu-core/hal (error and label strings) and core (unicode and
float formatting tables). It shrinks to 198 KB in the WebGPU-only build (and that still
includes the three items above).

### Top functions

`CoreCommandEncoder::finish` 87,439; `gles::Device::create_pipeline` 36,891;
`naga::valid::Validator::validate_impl` 31,078; `naga::front::wgsl::lower::Lowerer::call`
28,118; `Chart::relayout` (our code, with the renderer inlined) 26,660;
`naga::front::wgsl::Frontend::parse` 26,137; GLSL `write_expr` 25,817;
`wgsl::error::Error::as_parse_error` 23,761; the async `Chart::create` future 20,534;
`png::decoder::StreamingDecoder::parse_u32` 15,199 (inlined decoder body).
Full table: `target/wasm-profile-webgl/twiggy-top.txt` after running the script.

## Experiments

Every row is a separate build from commit `4093a7b` with the same pipeline
(cargo release, wasm-bindgen CLI, `wasm-opt -Oz` + the feature flags from
`build-wasm.sh`), unless the "variant" says otherwise. Baseline reproduces the shipped
2,338,652 bytes exactly. Delta is gzip vs baseline.

| # | Variant | Raw | gzip | brotli | Δ gzip | Δ raw |
|---|---|---:|---:|---:|---:|---:|
| a | Baseline (webgpu + webgl) | 2,338,652 | 992,317 | 791,216 | | |
| b | **WebGPU only** (drop wgpu `webgl`) | 577,999 | 294,450 | 256,484 | **-697,867 (-70.3%)** | -1,760,653 (-75.3%) |
| c | WebGL2 only (drop wgpu `webgpu`) | 2,274,036 | 964,415 | 772,246 | -27,902 (-2.8%) | -64,616 |
| d | Atlas as raw RGB, no PNG decoder | 2,328,068 | 953,276 | 750,240 | -39,041 (-3.9%) | -10,584 |
| d' | No atlas at all, no PNG decoder (blank-atlas stub; upper bound for moving the atlas out of the wasm) | 2,167,436 | 871,693 | 681,497 | -120,624 (-12.2%) | -171,216 |
| e1 | No serde_json / metrics JSON (1-glyph stub) | 2,283,614 | 973,198 | 776,658 | -19,119 (-1.9%) | -55,038 |
| e2 | No serde_path_to_error (plain `Deserialize`) | 2,313,361 | 980,817 | 783,707 | -11,500 (-1.2%) | -25,291 |
| e3 | No tsify (`ts` feature off in facetry-wasm) | 2,339,377 | 992,703 | 791,361 | +386 (0%) | +725 |
| e4 | e1 + e2 together | 2,258,399 | 962,544 | 769,543 | -29,773 (-3.0%) | -80,253 |
| f1 | `opt-level = "s"` | 2,588,794 | 1,075,291 | 841,547 | +82,974 | +250,142 |
| f2 | `opt-level = 3` | 2,976,129 | 1,200,007 | 912,787 | +207,690 | +637,477 |
| f3 | wasm-opt off | 4,898,426 | 1,242,374 | 901,377 | +250,057 | +2,559,774 |
| f4 | wasm-opt `-O3` | 2,410,236 | 997,405 | 792,386 | +5,088 | +71,584 |
| f5 | wasm-opt `-Os` | 2,375,367 | 991,800 | 789,445 | -517 | +36,715 |
| f6 | wasm-opt `-Oz --converge` | 2,336,473 | 989,694 | 788,804 | -2,623 (-0.3%) | -2,179 |
| f7 | wasm-opt `-O4` | 2,412,278 | 998,220 | 793,711 | +5,903 | +73,626 |
| f8 | f6 + `--low-memory-unused --zero-filled-memory` | 2,323,293 | 984,650 | 785,871 | -7,667 (-0.8%) | -15,359 |
| g1 | `log` with `release_max_level_off` | 2,259,573 | 963,196 | 769,835 | -29,121 (-2.9%) | -79,079 |
| i1 | d + g1 (both cheap wins, still one build) | 2,249,281 | 923,651 | 728,955 | -68,666 (-6.9%) | -89,371 |
| h | WebGPU only + opt-level "s" | 602,879 | 298,343 | 257,848 | +3,893 vs b | +24,880 vs b |
| i2 | **WebGPU only + d + g1** (proposed primary build) | 562,312 | 254,364 | 211,630 | **-737,953 (-74.4%)** | -1,776,340 |
| i3 | **WebGL2 only + d + g1** (proposed lazy fallback) | 2,185,244 | 897,012 | 711,102 | -95,305 (-9.6%) | -153,408 |
| j | WebGPU only with PNG, JSON and serde_path_to_error removed (floor of "facetry on WebGPU without the avoidable extras") | 319,835 | 143,798 | 121,434 | -848,519 | -2,018,817 |

Brotli versus gzip on the baseline: 791,216 vs 992,317 (-20.3%); on the WebGPU-only build:
256,484 vs 294,450 (-12.9%).

Notes on what each row means:

* **b / the `wgsl` question.** With `webgl` removed, naga is not linked at all (17 bytes of
  `naga_types` remain) even with the `wgsl` wgpu feature on: `wgsl` is only a `cfg` flag that
  enables the `ShaderSource::Wgsl` enum variant, and it only pulls naga in through
  `wgpu-core`. It cannot be turned off (the crate then fails to compile because
  `ShaderSource::Wgsl` disappears), and it does not need to be.
  The WebGPU-only module also needs no changes to the Rust sources: `Backends::GL` still
  compiles and simply fails at runtime ("no adapter") if requested.
* **c.** The WebGPU backend (wgpu's `web` glue + generated `webgpu_sys` bindings) costs 65 KB
  raw / 28 KB gzip on top of the WebGL path; the JS glue also grows (see below).
* **d / d'.** d replaces the PNG decode with an `include_bytes!` of the raw 512x148 RGB
  pixels (227,328 B). wasm-opt's memory packing drops the zero runs of the MSDF background
  from the data segments, so the file barely grows and the gzip of the raw atlas
  (74,415) is slightly below the PNG (77,352). d' removes the atlas and decoder
  completely (text renders blank) and is the upper bound for shipping the atlas outside the
  wasm; it saves 121 KB gzip, of which only ~39 KB is code and the ~77 KB atlas has to come
  back as a separate file (net ~ d).
* **e1.** The stub also removes the 21 KB metrics JSON and the `Deserialize` code for the
  font types, so the true saving of a "generate the metrics as a Rust static at build time"
  change is below -19 KB gzip by the size of the glyph table that replaces the JSON
  (the table is about 95 glyphs of 8 numbers).
* **e2.** Removes `serde_path_to_error` and its per-type `Wrap<>` visitors. It also loses
  the JSON-pointer `path` on shape errors (a user-visible feature), so this is an upper
  bound on a trade-off, not a free win.
* **e3.** `tsify` only emits TypeScript declarations; it adds no runtime code. Same for
  `serde-wasm-bindgen`: 3.8 KB of code. The derived deserialisers (46 KB, 2.6% of code) are
  needed with any serde-based input path; a hand-written reader over `js_sys::Reflect`
  would be smaller but is a large change that was not tried.
* **f.** `lto = true` already means fat LTO, so `lto = "fat"` is the same build (not
  rebuilt). The wasm-opt pass is worth 250 KB gzip (f3), `-Oz` beats `-O3/-O4/-Os`, and
  `--converge` adds only 0.3%. `--low-memory-unused` (the extra 5 KB in f8) assumes the
  lowest 1 KiB of memory is never used, which is not guaranteed by Rust's wasm layout
  (the stack grows down to address 0 in the default layout), so it is not recommended.
  `-C target-feature` options were not explored (nothing in the attribution suggested a
  win).
* **g1.** Only matters when wgpu-core is linked (WebGL path): in the WebGPU-only build the
  same flag is worth about 1 KB gzip (b to i2 is -40.1 KB, of which d is -39.0 KB).
  `log` macros are already inert at runtime because no logger is installed, but their format
  strings and call sites are compiled in.
* **Other things looked at and found small:** `core::fmt` ~13 KB, panic machinery
  (`core::panicking` + `std::panicking`) ~1.6 KB (`panic = "abort"` already), `dlmalloc`
  7 KB, `hashbrown` 44 KB (by name; the whole hashbrown bucket is 4.9 KB in the WebGPU-only build, so it belongs to wgpu-core/naga), `core::slice::sort` 28 KB.
  `strict_asserts` and `fragile-send-sync-non-atomic-wasm` are not enabled and the
  `profiling` crate is inert. No extra wgpu feature is switched on that could be removed.
  `panic=immediate-abort` needs nightly `-Zbuild-std` and was not tried.

### JS glue

wasm-bindgen's `facetry_wasm.js` is also downloaded: 127,821 B (20,963 gzip) in the baseline,
78,399 B (15,014 gzip) in the WebGPU-only build, 80,871 B (13,199 gzip) in WebGL-only.

## Reference points

A minimal wgpu "hello triangle" (one WGSL triangle, `wgpu::util::new_instance_with_webgpu_detection`,
same release profile and `wasm-opt -Oz`, scratch crate, not committed):

| Variant | Raw | gzip | brotli |
|---|---:|---:|---:|
| wgpu `webgpu` + `webgl` + `wgsl` | 1,663,712 | 668,737 | 533,836 |
| wgpu `webgpu` + `wgsl` only | 62,100 | 26,673 | 22,491 |
| Facetry baseline (for comparison) | 2,338,652 | 992,317 | 791,216 |
| Facetry WebGPU-only | 577,999 | 294,450 | 256,484 |

So the "wgpu floor" with both backends is 669 KB gzip (67% of our module); with WebGPU alone
the wgpu floor is 27 KB. Facetry's own contribution on top of the floor is about 324 KB gzip
with both backends and about 268 KB gzip with WebGPU only (the atlas alone is ~77 KB of it
and the PNG decoder ~40 KB).

## What the data says about M0 to M1

Sizes at the two milestone commits, same pipeline (stripped, `wasm-opt -Oz`; rebuilt here):

| | Raw | gzip | brotli |
|---|---:|---:|---:|
| M0 `b30a853` (clear colour) | 1,350,614 | 542,356 | 431,809 |
| M1 `5b0547b` | 2,294,489 | 972,593 | 776,555 |
| Δ | +943,875 | +430,237 | +344,746 |
| current `4093a7b` | 2,338,652 | 992,317 | 791,216 |

Per-component change in raw bytes between the two named builds (twiggy, `scripts/wasm-rollup.py`
buckets):

| Component | M0 | M1 | Δ | share of Δraw |
|---|---:|---:|---:|---:|
| Data segments | 209,055 | 475,334 | +266,279 | 28% |
| naga | 332,261 | 561,532 | +229,271 | 24% |
| std/alloc/core | 204,965 | 295,132 | +90,167 | 10% |
| wgpu API | 190,732 | 279,736 | +89,004 | 9% |
| wgpu-core | 124,647 | 185,801 | +61,154 | 6% |
| facetry_* | 22,538 | 72,919 | +50,381 | 5% |
| png/inflate | 0 | 49,898 | +49,898 | 5% |
| hashbrown etc. | 57,137 | 99,337 | +42,200 | 4% |
| serde (derive, json, path, wasm-bindgen) | 0 | 59,080 | +59,080 | 6% |
| wgpu-hal | 153,307 | 137,342 | -15,965 | -2% |

Interpretation:

* **M0 already contained the whole WebGL stack** (naga 332 KB, wgpu-core, hal): wgpu with the
  `webgl` feature links the GL backend and naga's GLSL writer and validator even for a
  clear-colour pass. That is why M0 was already 529 KiB.
* **M1 made more of that stack reachable.** Adding shader modules, pipelines, bind groups,
  buffers and textures pulls in the `wgsl` frontend (+172 KB code), more validation, constant
  evaluation, wgpu-core's pipeline/binding/texture/command paths and their tables. Together
  naga + wgpu + wgpu-core (net of a 16 KB drop in wgpu-hal) account for about 55% of the raw
  jump (code +363 KB, plus roughly 155 KB of the data growth), and the std/hashbrown growth
  (+132 KB) is largely theirs too. **This part is
  scaled, not measured per component**: gzip cannot be attributed per function, so
  applying the same share to the +430 KB gzip gives roughly 235 KB gzip.
* **The atlas and PNG decoder are the largest item we own:** measured on the current commit,
  removing both is -121 KB gzip (28% of the jump); the data is 77 KB of that.
* **serde and the metrics JSON:** serde_json + metrics -19 KB gzip, `serde_path_to_error`
  -11.5 KB, and the derived deserialisers (not separable from any serde path) another
  ~46 KB raw.
* `log` call sites in wgpu-core, which became reachable with M1, are worth 29 KB gzip
  (measured on the current commit).

## Recommendations (ranked)

Savings are against the baseline's 992,317 gzip / 791,216 brotli. "Measured" means a build
was made and sized; "estimated" means it was not.

1. **Ship a WebGPU-only module and lazy-load a WebGL2 module as the fallback.**
   Measured: primary 294,450 gzip / 256,484 brotli (-70% / -68%); with recommendations 2 and
   3 below, 254,364 / 211,630 (-74% / -73%). The fallback (WebGL2 only, with 2 and 3)
   measures 897,012 gzip / 711,102 brotli (-9.6% vs today), loaded only where WebGPU is
   missing, so WebGL-only users pay what they pay now and everyone else pays a quarter of it.
   Implementation sketch (not done): make `webgl` a cargo feature of `facetry-render` and
   `facetry-wasm` (wasm-pack builds twice, e.g. `--features webgl` into `wasm-webgl/`), and
   have `packages/facetry/src/wasm.ts` choose the module: `backend: "webgl2"` loads the
   fallback; `"webgpu"` loads the primary; `"auto"` does `await navigator.gpu?.requestAdapter()`
   in JS and loads the fallback if that is null. The WebGPU module would still contain the
   `BackendPreference::WebGl2` enum arm, which returns `gpu-unavailable` at runtime.
   Trade-offs: two builds in CI and in the npm package (the fallback is about 0.9 MB
   gzip, so package size on disk roughly doubles), a JS-side adapter probe that must agree
   with wgpu's own probing, and a second wasm-bindgen glue file (15 KB gzip each).
   Instantiating the fallback after a failed WebGPU attempt costs one extra round trip.

2. **Store the atlas as raw RGB instead of a PNG.** Measured: -39,041 gzip / -40,976 brotli
   (-3.9% / -5.2%), raw -10,584. Removes `png`, `fdeflate` and friends (50 KB code) and the
   decode at start-up. The build change is small: have `tools/font-atlas` also emit a
   `.rgb` file (or have `facetry-font` expose it) and make `decode_atlas` a plain RGB to RGBA
   expansion. Trade-offs: the committed asset is 227 KB instead of 77 KB (repo size, not
   download size) and wasm-opt's zero-run packing is what keeps the binary small, so
   keep wasm-opt in the pipeline. Pixel-for-pixel equivalence of the raw dump was not
   verified in a browser here (the experiment only measures size).
   Moving the atlas out of the wasm entirely (a PNG fetched by JS, decoded with
   `createImageBitmap`, uploaded via `copyExternalImageToTexture`) removes the same code with
   an extra request and about the same total bytes; the better lever is making the atlas
   itself smaller (fewer glyphs, lower `em_px`, 6-bit channels): the atlas is now 29% of the
   WebGPU-only module's gzip. **Estimated only**; it is a visual-quality decision.

3. **Build `log` with `release_max_level_off`** (add `log = { version = "0.4",
   features = ["release_max_level_off"] }` to `facetry-wasm`; cargo unifies the feature for
   wgpu-core). Measured: -29,121 gzip / -21,381 brotli on the full build, -28.2 KB gzip on the
   WebGL-only build, ~1 KB on the WebGPU-only build. It is a three-line, low-risk change
   (no logger is installed, so nothing visible is lost), which makes it the one
   "trivially safe build-config win" here; it is **not** applied in this branch. Do it
   together with, or before, recommendation 1 if the single-module design is kept.

4. **Generate the font metrics at build time instead of parsing JSON at start-up.**
   Upper bound measured: -19,119 gzip (-1.9%) / -15 KB brotli, and in the WebGPU-only
   build it makes `serde_json` unnecessary in the wasm entirely (it is the only runtime user:
   `parse_spec_json` is not exported). Realistic saving is a few KB less than the bound,
   because the glyph table still has to be stored. Also removes a start-up JSON parse of
   21 KB.

5. **Use `wasm-opt -Oz --converge`.** Measured: -2,623 gzip (-0.3%). Free but slows the
   build; optional. Keep `opt-level = "z"` and `-Oz`; nothing else tried is smaller
   gzipped (`"s"`, `3`, `-Os`, `-O3`, `-O4` are all larger; `"s"` is +83 KB on the full build and
   +3.9 KB on the WebGPU-only one).

6. **Serve with brotli.** Measured: -20% on the baseline (791 KB vs 992 KB) and -13% on the
   WebGPU-only build. Most CDNs do this automatically for `application/wasm`. I could not
   check the behaviour of GitHub Pages (used by `pages.yml`) offline and believe it serves
   gzip only, so verify with `curl -H 'Accept-Encoding: br' -I` before counting on it for the
   playground.

7. **Not recommended unless size becomes critical:**
   * Dropping `serde_path_to_error`: -11,500 gzip (measured upper bound) but loses precise
     error paths for shape errors, which the validation UX relies on.
   * Dropping `tsify`: no saving (+386 bytes), it only affects `.d.ts` output.
   * `--low-memory-unused` in wasm-opt: -5 KB gzip, but the assumption is not guaranteed.

8. **Larger engineering options for the WebGL fallback (estimates only, nothing built):**
   * *Pre-built naga IR instead of WGSL text.* wgpu accepts `ShaderSource::Naga` behind the
     `naga-ir` feature. Serialising the five shader modules at build time would drop
     `naga::front::wgsl` (172 KB raw code plus its error tables), saving an estimated
     40-70 KB gzip in the fallback, at the cost of a build step, a serialisation format and
     a naga version pin. The validator, GLSL writer and constant evaluator must stay.
   * *A hand-written WebGL2 renderer for the fallback* (rectangles, text quads, boxes and
     lines; the GLSL ES 3.00 translations are already checked in tests). This removes
     wgpu-core, hal and naga from the fallback; I would expect the fallback to land
     near the WebGPU-only size plus GL glue, i.e. 300-400 KB gzip instead of 897 KB.
     This is a large piece of work with a second rendering path to keep in sync, so only
     worth it if WebGL2 users are a large share of the audience.

Combined, using only measured items 1 to 3: primary 254,364 gzip (211,630 brotli), fallback
897,012 gzip (711,102 brotli). Adding item 4 should bring the primary to roughly 240 KB gzip
(estimate).

## Measured versus estimated

Measured (separate builds of the same commit, sizes in the tables): a to j, i1 to i3, M0 and
M1 rebuilds, the hello-triangle reference, the per-crate rollup of the named build.
Estimated or reasoned (not built): the share of the M0 to M1 gzip jump that belongs to
naga/wgpu (scaled from raw shares), the real saving of item 4 (bound is measured, remainder
is not), the atlas-shrinking options, anything in item 8, the GitHub Pages brotli
behaviour. Not tested at runtime in a browser: every experiment variant was only built and
sized; the stubs (blank atlas, one-glyph metrics, plain `Deserialize`) change behaviour and
are not meant to be shipped.

## Appendix: how each experiment was made

All in a throwaway git worktree under the scratchpad, reset between runs; the branch was never
modified by them.

* b, c: `facetry-render/Cargo.toml` wasm32 wgpu features minus `webgl` / minus `webgpu`
  (b without `wgsl` fails to compile, as noted).
* d: `decode_atlas` replaced by `include_bytes!` of the raw RGB dump plus RGB to RGBA
  expansion. d': `decode_atlas` returns a zeroed 512x148 RGBA buffer.
* e1: `FontMetrics::inter()` returns a one-glyph struct instead of parsing the JSON.
  e2: `parse_spec` calls `ChartSpec::deserialize` directly. e3: `facetry-wasm` depends on
  `facetry-core` without `features = ["ts"]`.
* f1/f2: `CARGO_PROFILE_RELEASE_OPT_LEVEL=s|3`. f3 to f8: `wasm-opt` variants run on the
  baseline's wasm-bindgen output.
* g1: `log` with `release_max_level_off` added to `facetry-wasm`.
* i1 to i3, j: combinations of the above.
* Hello triangle: separate crate with the same `[profile.release]`, wgpu 30.0.1 with
  `["std", "wgsl", "webgpu", "webgl"]` and `["std", "wgsl", "webgpu"]`.

## Applied

Recommendations 1 to 3 were implemented afterwards (commits on this branch). The numbers
above describe the single combined module at `4093a7b`; the sections below describe what
ships now. Recommendations 4 onwards are not applied.

| Module | Built with | Raw | gzip -9 | brotli -q 11 | vs old combined module (gzip) |
|---|---|---:|---:|---:|---:|
| `facetry_wasm` | `--no-default-features --features webgpu` | 563,245 | 254,991 | 212,008 | -737,326 (-74.3%) |
| `facetry_wasm_gl` | `--no-default-features --features webgl` | 2,187,106 | 897,869 | 711,806 | -94,448 (-9.5%) |
| old combined module | both backends | 2,338,652 | 992,317 | 791,216 | |

Measured with `scripts/wasm-size.sh` after `scripts/build-wasm.sh`. They are within 1 KB of
the experiment variants i2 (562,312 / 254,364 / 211,630) and i3 (2,185,244 / 897,012 /
711,102); the small difference is the new "backend not built" error paths and the
adapter-before-surface ordering in `Renderer::new`.

What changed:

1. **Two modules** (`facetry-render`/`facetry-wasm` features `webgpu` (default) and `webgl`;
   wgpu's `webgpu`/`webgl` features are no longer unconditional). `scripts/build-wasm.sh`
   builds both into `packages/facetry/wasm`. The TypeScript loader imports each lazily
   (`import()`), once per page. `"webgpu"` uses the small module, `"webgl2"` the large one,
   `"auto"` uses the small one when `navigator.gpu.requestAdapter()` yields an adapter and
   the large one otherwise, and retries once on the large one if the WebGPU module then
   fails to start with anything but a bad spec or bad options. A module asked for a backend
   it does not contain rejects with `gpu-unavailable` and a message naming what it has.
   To keep the canvas usable for that retry, the WebGPU-only module requests adapter and
   device *before* creating the surface, because `getContext("webgpu")` locks a canvas to
   that context type for good. In the playground build (Vite), the two wasm files and their
   glue files are separate chunks and the entry chunk is 12 KB.
2. **Raw atlas.** `tools/font-atlas` writes `inter-regular-msdf.rgb` next to the PNG;
   `facetry-font` embeds the raw bytes and the `png` crate is gone from the shipped
   dependency tree. A test in the tool crate decodes the PNG and requires it to equal the
   committed raw bytes; re-running the tool regenerates both images and the metrics
   byte for byte.
3. **`log` with `release_max_level_off`** in `facetry-wasm`.

(`scripts/wasm-profile.sh` now profiles the WebGL module by default; its stripped output is within 0.1% of the shipped file, 1.9 KB smaller because it skips wasm-pack's extra flags.)

Not verified here: the WebGPU module has never run against a real WebGPU device in this
environment (headless Chromium without a GPU has none). What was exercised: it builds, its
glue and wasm load, and requesting WebGPU in headless fails with `gpu-unavailable`
(Playwright). The adapter-before-surface ordering and the JS probe are untested on real
WebGPU; open the Pages playground in a WebGPU browser and check `chart.backend === "webgpu"`
before releasing.

## Later growth: axis labels, PNG snapshots, value labels and hover

Measured with the same pipeline (`scripts/build-wasm.sh`: wasm-pack, then `wasm-opt -Oz`); the
WebGPU-only module per commit of that branch (each commit built from a clean checkout):

| Commit | WebGPU raw | WebGPU gzip | Δ gzip |
|---|---:|---:|---:|
| before (main at the shrink) | 563,245 | 254,991 | |
| 2D top/right margin | 563,546 | 255,107 | +116 |
| 3D axis labels placed per frame (label planner, 4 KiB state table, shader slot) | 575,761 | 260,630 | +5,523 |
| `chart.snapshot()` (offscreen target, read-back, un-padding, un-premultiply) | 582,771 | 264,456 | +3,826 |
| value labels, picking, hover, events | 601,511 | 272,819 | +8,363 |

The WebGL2 module went from 897,869 to 917,046 gzip (+19,177); the same code, about the same
size, since the wgpu GL backend itself does not change.

Where it goes (`scripts/wasm-profile.sh` on a WebGPU build with all four commits): the new code
is spread thin, not one big item. The snapshot commit (+3.8 KB gzip) is `snapshotStart` and
`snapshotPoll` with their wasm-bindgen shims, the GPU copy and map calls and the read-back
conversion. The label planner (+5.5 KB) is the placement maths plus a rebuilt text pipeline
layout. The last commit (+8.4 KB) is serde code for the new spec types, validation, label
layout in both builders, picking, the hover paths and `markInfo`/`datum` building through
`js_sys::Reflect`. Three trims were applied after the first measurement (together -3.3 KB
gzip, 276,164 to 272,819): no hull sort in the label planner (the box's silhouette is tested
face by face), no `HashMap`/`VecDeque` there, and no `Debug` formatting of texture formats in
the snapshot error path. Not done: replacing the serde-based spec reader (see e3 above) would
remove more than all of this at once.

PNG encoding is deliberately not in the wasm (that was the point of removing `png`); the
browser's encoder is used from TypeScript.


## M2: stacking, lines, pies and the legend

Same pipeline (`scripts/build-wasm.sh`), one measurement per phase of the `m2-chart-types`
branch (each phase built after its tests passed):

| Phase | WebGPU raw | WebGPU gzip | Δ gzip | WebGL2 gzip | Δ gzip |
|---|---:|---:|---:|---:|---:|
| before (main) | 601,511 | 272,819 | | 917,046 | |
| 1 bar positions: stack, normalize, group (2D + 3D) | 621,833 | 281,421 | +8,602 | 924,730 | +7,684 |
| 2 line mark, numeric x axis, tessellator, mesh pipeline | 668,163 | 299,964 | +18,543 | 944,403 | +19,673 |
| 3 arc mark, 3D lit-mesh pipeline, label occlusion | 709,636 | 316,928 | +16,964 | 960,811 | +16,408 |
| 4 legend (2D and 3D overlay, second pass) | 727,543 | 324,277 | +7,349 | 967,304 | +6,493 |

Total: +51,458 B gzip for the WebGPU module (+18.9%), +50,258 for the WebGL2 one (+5.5%).
Brotli: 227,037 to 266,299 (WebGPU), 725,367 to 762,029 (WebGL2). No new dependencies: `glam` and
`bytemuck` were already there, `proptest` is a native-only dev-dependency of the render crate, and
the tessellator is about 150 lines (see `crates/facetry-render/src/stroke.rs`) instead of lyon,
which would have added a generic path-flattening and tessellation stack for a job that is
"extrude a polyline with miter joins".

Where it goes (`PROFILE_MODULE=webgpu scripts/wasm-profile.sh` at the end): the layout code is the
single biggest piece. `build_scene_with_theme` is 26 KB raw (it now holds the bar, line and linear
x paths, with `layer_bars` and the legend inlined around it), `build_scene_3d_with_theme` 16 KB,
`Renderer::set_scene_3d` 12 KB (slices, overlay, label planning), and the new legend layout 5 KB.
The serde-derived deserialisers for the spec types are the second block: `ChartSpec` 16 KB,
`LayerSpec` 12 KB (it gained `position`, `point`, `style`), `Encoding` 6 KB, ~61 KB raw for all
spec types together, as before the single largest thing that would shrink if the spec were read
by hand instead of derived (idea e3 above, not done). The rest of the growth is spread thin:
the mesh and slice pipelines, ray tests for picking and label occlusion, and the extra
`format!`/`f64` formatting calls for keys and labels.

Not done, in order of expected gain: a hand-written spec reader; sharing the key/label code of
the bar and line builders; and dropping `serde_path_to_error` for a cheaper error path.


## Legend toggle and lines in 3D

Same pipeline (`scripts/build-wasm.sh`), one measurement per feature of the
`legend-toggle-3d-lines` branch:

| Phase | WebGPU raw | WebGPU gzip | Δ gzip | WebGL2 gzip | Δ gzip |
|---|---:|---:|---:|---:|---:|
| before (end of M2) | 727,543 | 324,277 | | 967,304 | |
| 1 click a legend entry to hide a series (hit rectangles, hidden set, `setSeriesVisible`) | 739,038 | 329,662 | +5,385 | 971,651 | +4,347 |
| 2 lines in 3D (polylines and points in `Scene3d`, numeric 3D x axis, disc and curtain pipelines, picking and label rules) | 769,096 | 339,694 | +10,032 | 982,766 | +11,115 |

Total: +15,417 B gzip for the WebGPU module (+4.8%), +15,462 for the WebGL2 one (+1.6%). Brotli:
266,299 to 279,010 (WebGPU), 762,029 to 774,598 (WebGL2). Nothing new in the dependency tree.

Where it goes: the hiding feature is mostly the `Cow`-masked columns and per-layer category
filtering in `resolve_layer`/`resolve_arc` (plain code, no new generics worth mentioning), the
legend's hit rectangles, and four small wasm-bindgen exports (`setSeriesVisible`, `hiddenSeries`,
`legendPick`, `legendPosition`). The 3D lines are the larger part: the 3D builder grew a line
branch and a numeric x axis (it now matches the 2D builder's two x kinds), the renderer two more
pipelines (billboard discs and the curtain, each with a ~40-line WGSL shader that on the WebGL2
module also pays for naga's translation of it) and the label planner an overlap rule. The shader
count is why the WebGL2 delta of the second phase is a little above the WebGPU one.

## Spec reader by hand, font metrics as data

Branch `spec-reader`. Same pipeline (`scripts/build-wasm.sh`); WebGPU profile from
`PROFILE_MODULE=webgpu scripts/wasm-profile.sh`.

### What serde cost, measured first

Profile of the WebGPU module before any change (`target/wasm-profile-webgpu/rollup.txt`):

| component | code bytes | share of code (475,687) |
|---|---:|---:|
| serde-derived code for facetry types (`Deserialize` impls, visitors, field identifiers, `deny_unknown_fields` errors) | 62,280 | 13.1% |
| serde_json (the font metrics parse; `parse_spec_json` is not in the wasm) | 17,685 | 3.7% |
| serde_path_to_error | 10,702 | 2.2% |
| serde core / other | 6,635 | 1.4% |
| serde-wasm-bindgen | 6,025 | 1.3% |
| **serde total** | **103,327** | **21.7%** |

Plus 21,452 B of embedded metrics JSON in the data segments. So about 125 KB of the 769 KB raw
module was "read JSON into our types". The expected saving was the 103 KB of code and the JSON
minus what replaces them (a hand-written reader over a plain value tree, estimated at 25 to
35 KB raw): roughly 90 to 100 KB raw, 30 to 35 KB gzip. The stop rule asked for 10 KB gzip from
the reader; the measurements below are well past it.

### What changed

1. **Font metrics are Rust data** (`crates/facetry-font/assets/inter-regular-metrics.rs`,
   written by `tools/font-atlas`, `include!`d into a `static`). No JSON, no `OnceLock`, no
   `serde`/`serde_json` in `facetry-font`. The numbers are the f32 values the JSON held (checked
   token for token) and the atlas bytes did not change.
2. **The spec is read by hand.** `facetry_core::value` has a `Value` tree (null, bool, `Int`,
   `Uint`, `Float`, string, array, object with insertion-ordered keys) and a small JSON parser
   for it, with `serde_json`'s error wording and line/column. `facetry_core::spec_read` turns a
   `Value` into a `ChartSpec`: a small `object!` macro reads an object in input order and
   reports unknown fields, duplicates and missing fields; the rest is a function per type.
   `facetry-wasm` walks the JS object into a `Value` with `js-sys` (`value_from_js`) and builds
   the `issues` array by hand. `parse_spec_json` (tests, tools) is `parse_json` then
   `parse_spec_value`.
3. The spec types lost their `Deserialize` derives and the `Column`/`Cell` visitors.

### Results

| Step | WebGPU raw | WebGPU gzip | Δ gzip | WebGPU brotli | WebGL2 raw | WebGL2 gzip | Δ gzip | WebGL2 brotli |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| before (`1e9eeb0`) | 769,096 | 339,694 | | 279,010 | 2,383,259 | 982,766 | | 774,598 |
| 1 font metrics as Rust data | 714,710 | 321,248 | -18,446 | 264,148 | 2,331,150 | 964,919 | -17,847 | 760,532 |
| 2 hand-written spec reader | 669,204 | 302,702 | -18,546 | 251,204 | 2,284,087 | 944,713 | -20,206 | 747,655 |

Total: WebGPU -99,892 raw (-13.0%), -36,992 gzip (-10.9%), -27,806 brotli (-10.0%); WebGL2
-99,172 raw, -38,053 gzip (-3.9%), -26,943 brotli. The reader alone is -18.5 KB gzip on WebGPU
(the stop rule needed 10 KB). The font change alone clears that too, but the rule is about the
reader and it is measured on top of the font change.

Profile after (same rollup): the five serde buckets are gone and `twiggy` finds no symbol with
`serde` in its name. Code 475,687 to 404,772 B, data 263,827 to 237,509 B. `facetry_*` code
grew by 44 KB (the reader, the `Value` walk, and generics that used to be counted under serde),
which is why the net is not the full 103 KB.

### What is still in the dependency tree

`cargo tree -p facetry-wasm --target wasm32-unknown-unknown -e normal` still lists `serde`,
`serde_core`, `serde_derive` and `serde-wasm-bindgen`, for two reasons, neither of which puts a
byte in the wasm:

* `tsify` (the TypeScript declaration generator) depends on `serde` and `serde-wasm-bindgen`
  for helpers we do not use. The `Tsify` derive reads `#[serde(...)]` attributes
  (`rename_all`, `rename`, `untagged`, `skip`) to name things, and registers that attribute
  itself, so the spec types keep those attributes under `cfg_attr(feature = "ts", ...)` but
  derive nothing from serde. `serde_json` and `serde_path_to_error` are gone from the tree.
* `facetry-core` still derives `Serialize` for `Scene`/`Scene3d` (insta snapshots). Nothing
  serialises at run time, so it is dead code.

The generated TypeScript (`facetry_wasm.d.ts`, `facetry_wasm_gl.d.ts` and the `_bg.wasm.d.ts`
files) is byte-identical to before (`diff` on the `scripts/build-wasm.sh` output). tsify was kept
rather than generating the `.d.ts` in CI because it needs no new moving part; only the
declarations are generated, no code.

### Behaviour

Held by `crates/facetry-core/tests/parse_differential.rs`, which runs a frozen copy of the old
serde parser (`tests/serde_reference`) next to the new one on every example spec, hand-written
edge cases, mutations of the examples, damaged text and proptest-composed mutations: same
`ChartSpec` (compared through `Debug`), same error path, equivalent message, and for malformed
JSON `serde_json`'s own message and position. The existing validation tests, insta snapshots
and Playwright suite pass unchanged. Deliberate differences, all listed in that test file:

* A JSON **array where an object is expected** (`["x", "left"]` as an axis, `[]` as a whole
  spec) is now an error. serde derives read it positionally; the browser path took any JS
  array as an empty object.
* **Unknown fields in a JS object are now rejected.** `serde-wasm-bindgen` only looks at the
  field names it knows, so in the browser a typo like `"colour"` was silently ignored although
  `deny_unknown_fields` rejected it in JSON text. Measured on 25,935 spec objects fed to the old
  and new wasm in Node: no spec the old module rejected is accepted now; 472 are newly rejected
  for an unknown field, 153 for an array in place of an object, 2 for an over-long `domain`
  (`[1, 2, 3]`). The old module also threw a raw `TypeError` for `null` where an object was
  expected (49 cases); that is now an `invalid-spec` issue.
* Wording: messages lose JSON text's ` at line L column C` (syntax errors keep it); the browser
  path's `Error: ` prefix and `unit value` become `null`; floats print as Rust does (`1e100`).
  Enum fields given a number or `{}` say `invalid type: ..., expected a string`
  (serde_json said `expected value`).
* Text after the first JSON value is still ignored by `parse_spec_json`, as before.
* Floats parse correctly rounded; `serde_json`'s default can be one ULP off for inputs with many
  digits (the browser path never had that).

## M5: animated transitions

Gzip -9 sizes at each step (baseline is main at 1765804):

| step | WebGPU | WebGL2 |
|---|---:|---:|
| baseline | 302,702 | 944,713 |
| phase 1 (2D bars, axes, text, legend) | 317,317 (+14,615) | 959,160 (+14,447) |
| phase 2 (2D lines and arcs, all 3D marks) | 343,092 (+25,775) | 985,206 (+26,046) |
| phase 3 (camera tweens, legend toggle) | 347,025 (+3,933) | 988,566 (+3,360) |

Final: WebGPU raw 775,644 / gzip 347,025 / brotli 284,781; WebGL2 raw 2,389,848 / gzip 988,566 /
brotli 780,084. Total growth +44,323 B gzip (+14.6%) WebGPU and +43,853 (+4.6%) WebGL2.

Where the bytes go (twiggy/rollup, WebGPU build): about 49 KB raw is the transition code of
`facetry-core` (`Plan::diff` about 14 KB, `Plan3::diff` about 13.7 KB, the rest sampling, seams,
easing and path morphing) and about 17 KB is wasm glue for the new `Chart` methods. The shaders
are WGSL text and grow the data segment a little (the easing functions are shared through
`ease.wgsl`). Micro-optimisations tried (splitting `diff_keys` into a non-generic
`diff_str_keys`, scratch buffers) saved 250 B or less each; a second `wasm-opt` pass would save
about 1.2 KB gzip but `scripts/build-wasm.sh` was left alone.

Per-frame CPU cost (`examples/playground/tests/transition-cost.spec.ts`, WebGL2 in headless
Chromium; `seek` is the morph, tessellation and buffer upload; render time on software GL is not
meaningful):

| chart | CPU per frame |
|---|---|
| 1000 bars, 250 x 4 stacked | under 0.01 ms |
| 200-slice donut | about 0.1 ms |
| 3D, 30 x 30 boxes | about 0.07 ms |
| 4 x 250 lines with 1000 markers | 3.6 to 7 ms (p95 up to about 16 ms), nearly all marker discs |
| the same lines without markers | about 0.2 ms |
