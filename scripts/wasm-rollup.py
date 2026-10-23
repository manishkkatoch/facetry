"""Groups `twiggy top --format csv` rows by crate, as a share of the code section.

Usage: wasm-rollup.py twiggy-all.csv

Names are demangled Rust paths, so the crate is a heuristic: the first path segment, looking
inside `<T as Trait>` (T's crate) and `drop_in_place<T>` (T's crate). Generic std code that
is monomorphised for a crate's types (Vec<T>, hashbrown, fmt) is attributed to std/alloc/
core/hashbrown, not to the user of the generic, so those buckets overstate "std" a little.
"""
import csv
import re
import sys
from collections import defaultdict

GROUPS = [
    ("wgpu-core", r"wgpu_core"),
    ("wgpu-hal (incl. glow = WebGL bindings)", r"wgpu_hal|glow"),
    ("wgpu (api)", r"wgpu$|wgpu_types|wgpu_naga_bridge"),
    ("naga", r"naga"),
    ("serde_json", r"serde_json|zmij|itoa|ryu"),
    ("serde_path_to_error", r"serde_path_to_error"),
    ("serde-wasm-bindgen", r"serde_wasm_bindgen"),
    ("serde core/other", r"serde|serde_core|tsify"),
    ("png/inflate", r"png|miniz_oxide|fdeflate|crc32fast|adler2?|flate2"),
    ("facetry_* (incl. wasm-bindgen glue for our API)", r"facetry|chart_[a-zA-Z]+|__wbg_chart.*"),
    ("wasm-bindgen/js-sys/web-sys", r"wasm_bindgen|js_sys|web_sys|__wbg|__wbindgen|console_error"),
    ("std/alloc/core", r"std|alloc|core|compiler_builtins|dlmalloc|panic_abort|unwind|libm|str|char|[fiu](8|16|32|64|128|size)|slice|bool|__rustc"),
    ("hashbrown/indexmap/smallvec etc.", r"foldhash|slotmap|lock_api|simd_adler32|hashbrown|indexmap|smallvec|arrayvec|bit_set|bit_vec|bitflags|hexf|rustc_hash|fxhash|ahash|once_cell|parking_lot|log|thiserror|profiling|glam|bytemuck|ordered_float|unicode|half|num_traits|document_features|raw_window_handle|codespan|termcolor|hashbrown"),
]


def crate_of(name: str) -> str:
    n = re.sub(r"\[[0-9a-f]{8,16}\]", "", name.strip())  # drop crate disambiguators
    m = re.match(r"(?:core::ptr::)?drop_in_place<(.*)", n)
    if m:
        n = m.group(1)
    n = n.lstrip("<&*(")
    n = re.sub(r"^(dyn |mut |const )", "", n)
    m = re.match(r"([A-Za-z_][A-Za-z0-9_]*)", n)
    return m.group(1) if m else n


def bucket(name: str) -> str:
    if name.startswith('"') and name.endswith("subsection"):
        return "CUSTOM"
    if name.startswith("data[") or name.startswith("data segment"):
        return "DATA"
    if name.startswith("custom section"):
        return "CUSTOM"
    if name.startswith("elem[") or name.startswith("table["):
        return "TABLE"
    if name.startswith("type[") or name.startswith("import ") or name.startswith("export ") or name.startswith("memory[") or name.startswith("global[") or name.startswith("func[") or name.startswith("code[") or name.startswith("\"function names\"") or name.startswith("\"function signatures\""):
        return "STRUCT:" + name.split("[")[0].strip()
    c = crate_of(name)
    mine = "facetry_" in name
    if c in ("js_sys", "wasm_bindgen", "wasm_bindgen_futures") and "facetry_wasm" in name:
        return "facetry_* (incl. wasm-bindgen glue for our API)"
    if c.startswith("serde") and "facetry_" in name:
        return "serde-derived code for facetry types"
    for label, pat in GROUPS:
        if re.fullmatch(pat, c) or re.match(r"(?:%s)" % pat, c) and label != "std/alloc/core":
            return label
    if c in ("std", "alloc", "core", "compiler_builtins", "dlmalloc", "panic_abort"):
        return "std/alloc/core"
    return "other: " + c


def main(path: str) -> None:
    rows = []
    with open(path, newline="") as f:
        r = csv.reader(f)
        header = next(r)
        for row in r:
            rows.append(dict(zip(header, row)))
    key = "Shallow Bytes" if "Shallow Bytes" in rows[0] else list(rows[0].keys())[1]
    nm = "Item" if "Item" in rows[0] else list(rows[0].keys())[0]
    tot = defaultdict(int)
    cnt = defaultdict(int)
    other = defaultdict(int)
    for row in rows:
        b = bucket(row[nm])
        sz = int(row[key])
        tot[b] += sz
        cnt[b] += 1
        if b.startswith("other: "):
            other[b] += sz
    code = sum(v for k, v in tot.items() if not (k in ("DATA", "CUSTOM", "TABLE") or k.startswith("STRUCT:")))
    total = sum(tot.values())
    print(f"total attributed bytes: {total}  (code incl. names-free estimate: {code})")
    print(f"{'component':44} {'bytes':>9} {'% code':>7} {'items':>6}")
    for k, v in sorted(tot.items(), key=lambda kv: -kv[1]):
        if k.startswith("other: "):
            continue
        pct = f"{100 * v / code:6.1f}%" if code and not (k in ("DATA", "CUSTOM", "TABLE") or k.startswith("STRUCT:")) else "      -"
        print(f"{k:44} {v:9d} {pct} {cnt[k]:6d}")
    # Second level for the big crates: first two path segments after the crate name.
    sub = defaultdict(int)
    for row in rows:
        b = bucket(row[nm])
        if b.split(" ")[0] in ("naga", "wgpu", "wgpu-core", "wgpu-hal"):
            n = re.sub(r"\[[0-9a-f]{8,16}\]", "", row[nm].strip())
            n = re.sub(r"^(core::ptr::)?drop_in_place<", "", n).lstrip("<&*(")
            n = re.sub(r" as .*", "", n)  # "<T as Trait>::f" -> T
            parts = re.findall(r"[A-Za-z_][A-Za-z0-9_]*", n)
            # keep module path only (stop at the first CamelCase type)
            keep = []
            for p_ in parts:
                if p_[0].isupper():
                    break
                keep.append(p_)
            sub["::".join(keep[:3]) or "::".join(parts[:3])] += int(row[key])
    print("\nsecond level (naga / wgpu / wgpu-core / wgpu-hal), first three module segments:")
    for k, v in sorted(sub.items(), key=lambda kv: -kv[1])[:30]:
        print(f"  {k:52} {v:9d} {100 * v / code:5.1f}%")
    o = sum(other.values())
    print(f"{'other (see below)':44} {o:9d} {100 * o / code:6.1f}%")
    print("\ntop 'other' crates:")
    for k, v in sorted(other.items(), key=lambda kv: -kv[1])[:25]:
        print(f"  {k[7:]:40} {v:9d} {100 * v / code:5.1f}%")


if __name__ == "__main__":
    main(sys.argv[1])
