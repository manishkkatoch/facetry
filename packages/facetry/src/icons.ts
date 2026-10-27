import { FacetryError } from "./errors.js";

/** An icon as SVG path data drawn in `viewBox` (`[minX, minY, width, height]`, default `[0, 0, 24, 24]`). */
export interface IconPath {
  d: string;
  viewBox?: [number, number, number, number];
}

/**
 * An icon for `DiagramOptions.icons`: SVG markup (`"<svg ...>...</svg>"`, as an icon set ships
 * it), bare path data (the `d` of a `<path>`, in a 24 by 24 box), or path data with its
 * viewBox. Icons are drawn as lines, like the built-in ones: line icons (Lucide, Tabler,
 * Feather, ...) look as designed, filled ones show their outlines.
 */
export type IconSource = string | IconPath;

const SKIPPED = new Set(["defs", "title", "desc", "metadata", "clipPath", "mask", "symbol", "style"]);

/**
 * Converts SVG markup into one path: every `path`, `circle`, `ellipse`, `rect`, `line`,
 * `polyline` and `polygon`, with the `viewBox` (or `width` and `height`) of the `<svg>`.
 * Shapes painted neither filled nor stroked (an icon set's invisible bounding box) are left
 * out. Throws a `FacetryError` (`"invalid-options"`) for what it cannot place exactly:
 * `transform`s, `<use>`, `<text>` and `<image>`.
 */
export function iconFromSvg(markup: string, name = "icon"): IconPath {
  const fail = (why: string): never => {
    throw new FacetryError("invalid-options", `icon \`${name}\`: ${why}`);
  };
  const doc = new DOMParser().parseFromString(markup, "image/svg+xml");
  const svg = doc.documentElement;
  if (svg.nodeName !== "svg" || doc.getElementsByTagName("parsererror").length > 0) {
    fail("not SVG markup (expected an <svg> element)");
  }
  const num = (el: Element, attr: string): number => {
    const v = parseFloat(el.getAttribute(attr) ?? "0");
    return Number.isFinite(v) ? v : 0;
  };
  const parts: string[] = [];
  const visit = (el: Element, fill: string, stroke: string) => {
    const tag = el.localName;
    if (SKIPPED.has(tag)) return;
    if (el.hasAttribute("transform")) {
      fail("transforms are not supported; flatten them into the paths first (SVGO's convertTransform does)");
    }
    if (tag === "use" || tag === "text" || tag === "image") fail(`<${tag}> is not supported`);
    // Paint is inherited: `fill="none"` on the <svg> of a line icon, `stroke="none"` on a
    // filled icon, and both on a bounding box that is not meant to show.
    fill = el.getAttribute("fill") ?? fill;
    stroke = el.getAttribute("stroke") ?? stroke;
    const painted = fill !== "none" || stroke !== "none";
    const d = painted ? shape(el, tag, num) : undefined;
    if (d) parts.push(d);
    for (const child of Array.from(el.children)) visit(child, fill, stroke);
  };
  visit(svg, "black", "none");
  if (parts.length === 0) fail("the SVG draws no shapes");
  const box = svg.getAttribute("viewBox");
  let viewBox: [number, number, number, number] | undefined;
  if (box) {
    const v = box.trim().split(/[\s,]+/).map(Number);
    if (v.length !== 4 || v.some((x) => !Number.isFinite(x))) fail(`bad viewBox "${box}"`);
    viewBox = v as [number, number, number, number];
  } else if (svg.hasAttribute("width") && svg.hasAttribute("height")) {
    viewBox = [0, 0, num(svg, "width"), num(svg, "height")];
  }
  return viewBox ? { d: parts.join(" "), viewBox } : { d: parts.join(" ") };
}

/** One shape element as path data; undefined for anything that is not a shape. */
function shape(el: Element, tag: string, num: (el: Element, a: string) => number): string | undefined {
  const points = () =>
    (el.getAttribute("points") ?? "")
      .trim()
      .split(/[\s,]+/)
      .filter((s) => s !== "")
      .map(Number);
  switch (tag) {
    case "path":
      return el.getAttribute("d") ?? undefined;
    case "circle":
    case "ellipse": {
      const [cx, cy] = [num(el, "cx"), num(el, "cy")];
      const rx = tag === "circle" ? num(el, "r") : num(el, "rx");
      const ry = tag === "circle" ? rx : num(el, "ry");
      if (rx <= 0 || ry <= 0) return undefined;
      return `M${cx - rx} ${cy} A${rx} ${ry} 0 1 0 ${cx + rx} ${cy} A${rx} ${ry} 0 1 0 ${cx - rx} ${cy} Z`;
    }
    case "rect": {
      const [x, y, w, h] = [num(el, "x"), num(el, "y"), num(el, "width"), num(el, "height")];
      if (w <= 0 || h <= 0) return undefined;
      let rx = el.hasAttribute("rx") ? num(el, "rx") : num(el, "ry");
      let ry = el.hasAttribute("ry") ? num(el, "ry") : rx;
      rx = Math.min(Math.max(rx, 0), w / 2);
      ry = Math.min(Math.max(ry, 0), h / 2);
      if (rx === 0 || ry === 0) return `M${x} ${y} H${x + w} V${y + h} H${x} Z`;
      const a = (tx: number, ty: number) => `A${rx} ${ry} 0 0 1 ${tx} ${ty}`;
      return (
        `M${x + rx} ${y} H${x + w - rx} ${a(x + w, y + ry)} V${y + h - ry} ${a(x + w - rx, y + h)} ` +
        `H${x + rx} ${a(x, y + h - ry)} V${y + ry} ${a(x + rx, y)} Z`
      );
    }
    case "line":
      return `M${num(el, "x1")} ${num(el, "y1")} L${num(el, "x2")} ${num(el, "y2")}`;
    case "polyline":
    case "polygon": {
      const p = points();
      if (p.length < 4) return undefined;
      return `M${p[0]} ${p[1]} L${p.slice(2).join(" ")}${tag === "polygon" ? " Z" : ""}`;
    }
    default:
      return undefined;
  }
}

/** `source` as path data and viewBox. */
export function iconPath(name: string, source: IconSource): IconPath {
  if (typeof source !== "string") return source;
  return source.trimStart().startsWith("<") ? iconFromSvg(source, name) : { d: source };
}
