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
// Types only: nothing from the chart modules is loaded by this entry point.
import type { ChartSpec, DiagramStyle } from "../wasm/facetry_wasm.js";
import type { DiagramIssue } from "../wasm/facetry_diagrams.js";
import { FacetryError } from "./errors.js";

export type { DiagramIssue, DiagramStyle };

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
   * Isometric style only: lay a card on the floor beside each node, alternating sides from
   * rank to rank. The card shows the node's label (its first line as the title, the rest as
   * wrapped text) and its icon; the node itself then shows its number. The layout makes room
   * for the cards. Default `false`.
   */
  cards?: boolean;
}

type Glue = typeof import("../wasm/facetry_diagrams.js");

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
   * Parses Mermaid text and lays it out in `options.style`. Supported: `flowchart` (also
   * written `graph`), with `classDef`, `class`, `:::` and `style` for colours; see
   * the README for the exact subset. Rejects with a `FacetryError` whose `code` is
   * `"invalid-diagram"` and whose `diagramIssues` lists every problem (line, column, the
   * source line and a message), including Mermaid features that are not supported: a diagram
   * is never drawn with parts missing.
   */
  async fromMermaid(source: string, options: DiagramOptions = {}): Promise<ChartSpec> {
    const glue = await load();
    try {
      return JSON.parse(glue.layoutMermaid(source, options.style, options.cards)) as ChartSpec;
    } catch (e) {
      throw FacetryError.from(e);
    }
  },
};
