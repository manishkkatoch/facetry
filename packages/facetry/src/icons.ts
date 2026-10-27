import { FacetryError } from "./errors.js";

export type FillRule = "nonzero" | "evenodd";

/** One filled shape of an icon: path data and its fill rule (default `"nonzero"`, as in SVG). */
export interface IconFillPath {
  d: string;
  fillRule?: FillRule;
}

/**
 * An icon as SVG path data drawn in `viewBox` (`[minX, minY, width, height]`, default
 * `[0, 0, 24, 24]`): `stroke`, path data drawn as lines, and `fill`, filled shapes (one path
 * data string, or a list of shapes each filled on its own). At least one of the two.
 */
export interface IconPath {
  stroke?: string;
  fill?: string | IconFillPath[];
  /** For a `fill` given as one string. */
  fillRule?: FillRule;
  viewBox?: [number, number, number, number];
}

/**
 * An icon for `DiagramOptions.icons`: SVG markup (`"<svg ...>...</svg>"`, as an icon set ships
 * it), bare path data (the `d` of a filled `<path>` in a 24 by 24 box, as Material Design
 * Icons, Simple Icons or Font Awesome publish it), or an {@link IconPath}. Filled shapes are
 * filled and stroked ones drawn as lines, both in the style's ink.
 */
export type IconSource = string | IconPath;

/** An icon in the form the diagrams module takes: lines, and filled shapes one by one. */
export interface IconParts {
  stroke?: string;
  fill: IconFillPath[];
  viewBox?: [number, number, number, number];
}

const SKIPPED = new Set(["defs", "title", "desc", "metadata", "clipPath", "mask", "symbol", "style"]);

/**
 * Converts SVG markup into path data: every `path`, `circle`, `ellipse`, `rect`, `line`,
 * `polyline` and `polygon`, filled where SVG would fill it (`fill` and `fill-rule`, inherited,
 * from attributes or the `style` attribute) and drawn as lines where it would stroke it, with
 * the `viewBox` (or `width` and `height`) of the `<svg>`. Shapes painted neither way (an icon
 * set's invisible bounding box) are left out. Throws a `FacetryError` (`"invalid-options"`)
 * for what it cannot place exactly: `transform`s, `<use>`, `<text>` and `<image>`.
 */
export function iconFromSvg(markup: string, name = "icon"): IconParts {
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
  // A presentation property: from the `style` attribute, else the attribute of that name.
  const paint = (el: Element, prop: string): string | undefined => {
    const style = el.getAttribute("style") ?? "";
    const m = new RegExp(`(?:^|;)\\s*${prop}\\s*:\\s*([^;]+)`).exec(style);
    return m ? m[1]!.trim() : (el.getAttribute(prop) ?? undefined);
  };
  type Paint = { fill: string; stroke: string; rule: string };
  const strokes: string[] = [];
  const fills: IconFillPath[] = [];
  const visit = (el: Element, inherited: Paint) => {
    const tag = el.localName;
    if (SKIPPED.has(tag)) return;
    if (el.hasAttribute("transform")) {
      fail("transforms are not supported; flatten them into the paths first (SVGO's convertTransform does)");
    }
    if (tag === "use" || tag === "text" || tag === "image") fail(`<${tag}> is not supported`);
    // Paint is inherited: `fill="none"` on the <svg> of a line icon, `stroke="none"` on a
    // filled icon, and both on a bounding box that is not meant to show.
    const p: Paint = {
      fill: paint(el, "fill") ?? inherited.fill,
      stroke: paint(el, "stroke") ?? inherited.stroke,
      rule: paint(el, "fill-rule") ?? inherited.rule,
    };
    const d = shape(el, tag, num);
    if (d) {
      // A line has no inside to fill.
      if (p.fill !== "none" && tag !== "line") {
        fills.push(p.rule === "evenodd" ? { d, fillRule: "evenodd" } : { d });
      }
      if (p.stroke !== "none") strokes.push(d);
    }
    for (const child of Array.from(el.children)) visit(child, p);
  };
  visit(svg, { fill: "black", stroke: "none", rule: "nonzero" });
  if (strokes.length === 0 && fills.length === 0) fail("the SVG draws no shapes");
  const box = svg.getAttribute("viewBox");
  const out: IconParts = { fill: fills };
  if (strokes.length > 0) out.stroke = strokes.join(" ");
  if (box) {
    const v = box.trim().split(/[\s,]+/).map(Number);
    if (v.length !== 4 || v.some((x) => !Number.isFinite(x))) fail(`bad viewBox "${box}"`);
    out.viewBox = v as [number, number, number, number];
  } else if (svg.hasAttribute("width") && svg.hasAttribute("height")) {
    out.viewBox = [0, 0, num(svg, "width"), num(svg, "height")];
  }
  return out;
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

/** `source` in the form the diagrams module takes. */
export function iconParts(name: string, source: IconSource): IconParts {
  if (typeof source === "string") {
    return source.trimStart().startsWith("<") ? iconFromSvg(source, name) : { fill: [{ d: source }] };
  }
  const fill =
    typeof source.fill === "string"
      ? [source.fillRule === "evenodd" ? { d: source.fill, fillRule: source.fillRule } : { d: source.fill }]
      : (source.fill ?? []);
  const out: IconParts = { fill };
  if (source.stroke !== undefined) out.stroke = source.stroke;
  if (source.viewBox !== undefined) out.viewBox = source.viewBox;
  return out;
}
