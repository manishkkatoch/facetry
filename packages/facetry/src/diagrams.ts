/**
 * `facetry/diagrams`: Mermaid diagrams drawn in 3D. A separate entry point backed by its own
 * small wasm module (the Mermaid parser and layout, no GPU code), which is downloaded the
 * first time a diagram is laid out, never by pages that only draw charts.
 *
 * ```ts
 * import { Chart } from "facetry";
 * import { diagram } from "facetry/diagrams";
 *
 * const spec = await diagram.fromMermaid("flowchart LR\n  a[Write] --> b{Valid?}");
 * const chart = await Chart.create(canvas, spec);
 * ```
 *
 * The result is an ordinary `ChartSpec` (`dimension: "3d"` with a `diagram`), so it can be
 * stored, edited or shipped as JSON and drawn later without this module.
 */
// Types only: nothing from the chart modules is loaded by this entry point (`run` imports the
// chart module when it is called).
import type { Block, ChartSpec, DiagramStyle, DiagramTheme } from "../wasm/facetry_wasm.js";
import type { Chart, ChartOptions } from "./chart.js";
import type { DiagramIssue } from "../wasm/facetry_diagrams.js";
import { FacetryError } from "./errors.js";
import { iconParts, type IconParts, type IconSource } from "./icons.js";
import { iconPackNames, packIcons, registerIconPacks, type IconPack } from "./iconPacks.js";

export type { Block, BlockPart } from "../wasm/facetry_wasm.js";
export type { DiagramIssue, DiagramStyle, DiagramTheme, IconSource };
export type { FillRule, IconFillPath, IconParts, IconPath } from "./icons.js";
export { iconFromSvg } from "./icons.js";
export type { IconifyIcon, IconifyJSON, IconPack, IconPackIcons } from "./iconPacks.js";

export interface DiagramOptions {
  /**
   * How the diagram looks. `"flat"` (default): light blocks with thin edges, seen square on.
   * `"glossy"`: glossy slabs coloured by shape (or by `classDef`), thick square-cornered
   * arrows on the floor, soft shadows and labels printed on the slabs, seen at an angle.
   * `"isometric"`: layered pedestals in palette colours joined by roads with chevrons, long
   * soft shadows, printed labels, in the classic isometric view. `"schematic"`: a plant
   * schematic in the same view, nodes as equipment by shape (building, silo, tank, valve,
   * reactor, drum), pipes on supports with flow marks, labels on standing signs. The
   * style also decides how edges are routed, so it is chosen here rather than in the spec.
   */
  style?: DiagramStyle;
  /**
   * The colours, after Mermaid's themes: `"default"`, `"neutral"` (greys), `"dark"` (light
   * lines and text on a dark page) or `"forest"` (greens), each in the chosen style's own
   * manner. A theme set in the Mermaid text itself (`%%{init: {"theme": "dark"}}%%`, or
   * `config: theme:` in front matter) wins, as in Mermaid. Default `"default"`.
   */
  theme?: DiagramTheme;
  /**
   * Animate every edge: marks move along it the way it runs (beads on the flat style's lines,
   * light pulses on glossy arrows, chevrons driving along isometric roads, flow marks along
   * schematic pipes). `true` or `"fast"`, or `"slow"`; default `false`. Edges the Mermaid text
   * animates itself (`a e1@--> b` and `e1@{ animate: true }` or `animation: slow`) move either
   * way, and one it sets `animate: false` stays still. See `Chart.flow` to stop and start them.
   */
  flow?: boolean | "fast" | "slow";
  /**
   * Blocks of your own, by name, for `@{ shape: "<name>" }` in the Mermaid text: models the
   * schematic and isometric styles draw in place of their equipment and pedestals (the other
   * styles draw the node as a rectangle). Each is a list of parts, footprints raised between
   * two heights and fitted to the node's box; see the README. They win over blocks of the same
   * name from {@link diagram.registerBlocks}. Only the blocks the diagram uses go into the spec,
   * where `Chart.create` checks them.
   */
  blocks?: Record<string, Block>;
  /**
   * Isometric style only: lay a card on the floor beside each node, alternating sides from
   * rank to rank. The card shows the node's label (its first line as the title, the rest as
   * wrapped text) and its icon; the node itself then shows its number. The layout makes room
   * for the cards. Default `false`.
   */
  cards?: boolean;
  /**
   * Icons of your own, by name, for `@{ icon: "name" }` in the Mermaid text (they win over a
   * built-in icon of the same name): SVG markup as an icon set ships it, bare path data of a
   * filled shape in a 24 by 24 box, or `{ stroke?, fill?, fillRule?, viewBox? }`. Filled shapes
   * are filled and stroked ones drawn as lines, in the style's ink. They travel inside the
   * returned spec (`diagram.icons`), so it still draws on its own.
   *
   * ```ts
   * await diagram.fromMermaid('flowchart LR\n  a[Pump]@{ icon: "pump" }', {
   *   icons: { pump: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="9"/><path d="M12 3v18"/></svg>' },
   * });
   * ```
   */
  icons?: Record<string, IconSource>;
}

/** Options of {@link diagram.run}. */
export interface RunOptions extends DiagramOptions {
  /** Which elements hold Mermaid text: a CSS selector. Default `"pre.mermaid"`, as Mermaid's. */
  querySelector?: string;
  /** The elements to draw instead (then `querySelector` is not used). */
  nodes?: ArrayLike<HTMLElement>;
  /** The canvas's CSS height (it is as wide as the element). Default `"400px"`. */
  height?: string;
  /** Options for every chart, such as the `backend`. */
  chart?: ChartOptions;
  /**
   * Resolve even if a diagram fails (its message is shown in its place either way). Default
   * `false`: the others are still drawn, then `run` rejects with the first failure.
   */
  suppressErrors?: boolean;
}

/** Marks an element `run` has drawn, so a second `run` leaves it alone (Mermaid's attribute). */
const PROCESSED = "data-processed";

/** `text` with the indentation its lines share removed (Mermaid text indented in the HTML). */
function dedent(text: string): string {
  const lines = text.replace(/\r\n?/g, "\n").split("\n");
  const indents = lines.filter((l) => l.trim() !== "").map((l) => /^[ \t]*/.exec(l)![0].length);
  const cut = indents.length > 0 ? Math.min(...indents) : 0;
  return lines.map((l) => l.slice(cut)).join("\n");
}

/** Shows why `element`'s diagram could not be drawn, in its place. */
function showError(element: HTMLElement, error: FacetryError): void {
  const box = document.createElement("div");
  box.className = "facetry-error";
  box.setAttribute("role", "alert");
  box.style.cssText = "font: 13px/1.5 system-ui, sans-serif; color: #b42318; white-space: pre-wrap";
  const lines =
    error.diagramIssues.length > 0
      ? error.diagramIssues.map((i) => `line ${i.line}, column ${i.column}: ${i.message}`)
      : [error.message];
  box.textContent = ["Diagram error:", ...lines].join("\n");
  element.replaceChildren(box);
}

type Glue = typeof import("../wasm/facetry_diagrams.js");

/** Blocks registered for the page (see `diagram.registerBlocks`). */
const registeredBlocks = new Map<string, Block>();

let loading: Promise<Glue> | undefined;

/** Fetches and instantiates the diagrams module once per page. */
function load(): Promise<Glue> {
  loading ??= import("../wasm/facetry_diagrams.js").then(async (glue) => {
    await glue.default();
    return glue;
  });
  loading.catch(() => (loading = undefined)); // allow a retry after a network failure
  return loading;
}

export const diagram = {
  /**
   * Registers icon sets for every diagram on the page, as Mermaid's `registerIconPacks` does:
   * `@{ icon: "<pack>:<icon>" }` then draws icon `<icon>` of pack `<pack>`, in any style. A pack
   * holds an Iconify JSON set (an `@iconify-json/*` package's `icons.json`, aliases followed)
   * or icons by name as `DiagramOptions.icons` takes them; or has a `loader` that fetches them
   * the first time a diagram names one of its icons. A pack of a name already registered
   * replaces it. `options.icons` given to a call win over pack icons of the same name.
   *
   * ```ts
   * diagram.registerIconPacks([
   *   { name: "lucide", icons: lucideJson },
   *   { name: "logos", loader: () => fetch(url).then((r) => r.json()) },
   * ]);
   * ```
   */
  registerIconPacks(packs: IconPack[]): void {
    registerIconPacks(packs);
  },

  /** The names of the registered icon packs. */
  iconPacks(): string[] {
    return iconPackNames();
  },

  /**
   * Registers blocks for every diagram on the page: `@{ shape: "<name>" }` then draws block
   * `<name>` in the schematic and isometric styles. A name already registered is replaced;
   * names must be letters, digits, `-` or `_`. Blocks given to a call in `blocks` win.
   *
   * ```ts
   * diagram.registerBlocks({
   *   pump: { parts: [
   *     { name: "base", from: 0, to: 0.6, color: "frame" },
   *     { name: "casing", shape: "circle", size: [0.7, 0.7], from: 0.6, to: 2.2, color: "steel" },
   *   ] },
   * });
   * ```
   */
  registerBlocks(blocks: Record<string, Block>): void {
    for (const [name, block] of Object.entries(blocks)) {
      if (!/^[A-Za-z0-9_-]+$/.test(name)) {
        throw new FacetryError(
          "invalid-options",
          `block name ${JSON.stringify(name)} must be letters, digits, \`-\` or \`_\``,
        );
      }
      registeredBlocks.set(name, block);
    }
  },

  /** The names of the registered blocks. */
  blockNames(): string[] {
    return [...registeredBlocks.keys()];
  },

  /**
   * Parses Mermaid text and lays it out in `options.style`. Supported: `flowchart` (also
   * written `graph`), with `classDef`, `class`, `:::` and `style` for colours; see
   * the README for the exact subset. Rejects with a `FacetryError` whose `code` is
   * `"invalid-diagram"` and whose `diagramIssues` lists every problem (line, column, the
   * source line and a message), including Mermaid features that are not supported: a diagram
   * is never drawn with parts missing. A broken icon in `options.icons` rejects with code
   * `"invalid-options"`, naming the icon and what is wrong with it; so does an icon missing from
   * the registered pack its name points into, or a pack that fails to load. Only the icons the
   * diagram uses go into the spec.
   */
  async fromMermaid(source: string, options: DiagramOptions = {}): Promise<ChartSpec> {
    // Icons of registered packs the text names (`pack:icon`); `options.icons` win over them.
    const given = options.icons ?? {};
    const fromPacks = await packIcons(source, given);
    const icons = Object.entries({ ...fromPacks, ...given }).map(
      ([name, source]): [string, IconParts] => [name, iconParts(name, source)],
    );
    const blocks = { ...Object.fromEntries(registeredBlocks), ...(options.blocks ?? {}) };
    const glue = await load();
    try {
      const json = glue.layoutMermaid(
        source,
        options.style,
        options.cards,
        options.theme,
        options.flow === true ? "fast" : options.flow === false ? undefined : options.flow,
        icons.map(([name]) => name),
        icons.map(([, icon]) => icon.stroke ?? ""),
        new Uint32Array(icons.map(([, icon]) => icon.fill.length)),
        icons.flatMap(([, icon]) => icon.fill.map((f) => f.d)),
        new Uint8Array(icons.flatMap(([, icon]) => icon.fill.map((f) => (f.fillRule === "evenodd" ? 1 : 0)))),
        new Float64Array(icons.flatMap(([, icon]) => icon.viewBox ?? [NaN, NaN, NaN, NaN])),
        Object.keys(blocks),
      );
      const spec = JSON.parse(json) as ChartSpec;
      // The blocks the nodes name travel with the diagram (the module only knew their names).
      const used = new Set(spec.diagram?.nodes.flatMap((n) => (n.block ? [n.block] : [])));
      if (spec.diagram && used.size > 0) {
        spec.diagram.blocks = Object.fromEntries([...used].map((name) => [name, blocks[name]!]));
      }
      return spec;
    } catch (e) {
      throw FacetryError.from(e);
    }
  },

  /**
   * Draws every diagram on the page, as `mermaid.run()` does: the Mermaid text of each
   * `<pre class="mermaid">` (or `options.querySelector`, or `options.nodes`) is replaced by a
   * canvas drawing it, as wide as the element and `options.height` tall. An element can choose
   * its own style with `data-style` (`"glossy"`, ...), theme with `data-theme`, cards with
   * `data-cards` and moving edges with `data-flow` (`"slow"`, `"fast"`, or present for fast). Elements
   * already drawn (`data-processed`) are skipped, so `run` can be called again after adding
   * more. A diagram that fails shows its message in its place; the others are still drawn, then
   * `run` rejects with the first failure, a `FacetryError` (with `options.suppressErrors` it
   * resolves instead). Resolves to the charts drawn, in document order; `destroy` them when the
   * page no longer shows them.
   */
  async run(options: RunOptions = {}): Promise<Chart[]> {
    const nodes = Array.from(
      options.nodes ?? document.querySelectorAll<HTMLElement>(options.querySelector ?? "pre.mermaid"),
    ).filter((n) => !n.hasAttribute(PROCESSED));
    if (nodes.length === 0) return [];
    // Loaded only now: the chart module and its wasm are not needed to lay a diagram out.
    const { Chart } = await import("./chart.js");
    const charts: Chart[] = [];
    let first: FacetryError | undefined;
    for (const element of nodes) {
      element.setAttribute(PROCESSED, "true");
      const source = dedent(element.textContent ?? "");
      const style = (element.dataset.style as DiagramStyle | undefined) ?? options.style;
      const theme = (element.dataset.theme as DiagramTheme | undefined) ?? options.theme;
      const flowAttr = element.dataset.flow;
      const flow =
        flowAttr === undefined
          ? options.flow
          : flowAttr === "slow" || flowAttr === "fast"
            ? flowAttr
            : flowAttr !== "false";
      const cards = element.dataset.cards !== undefined ? element.dataset.cards !== "false" : options.cards;
      try {
        const spec = await diagram.fromMermaid(source, { ...options, style, theme, cards, flow });
        const canvas = document.createElement("canvas");
        canvas.style.cssText = `display: block; width: 100%; height: ${options.height ?? "400px"}`;
        canvas.setAttribute("role", "img");
        canvas.setAttribute("aria-label", source.trim());
        element.replaceChildren(canvas);
        charts.push(await Chart.create(canvas, spec, options.chart));
      } catch (e) {
        const error = FacetryError.from(e);
        showError(element, error);
        first ??= error;
      }
    }
    if (first && !options.suppressErrors) throw first;
    return charts;
  },
};
