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
import type { ChartSpec } from "../wasm/facetry_wasm.js";
import type { DiagramIssue } from "../wasm/facetry_diagrams.js";
import { FacetryError } from "./errors.js";

export type { DiagramIssue };

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
   * Parses Mermaid text and lays it out. Supported: `flowchart` (also written `graph`); see
   * the README for the exact subset. Rejects with a `FacetryError` whose `code` is
   * `"invalid-diagram"` and whose `diagramIssues` lists every problem (line, column, the
   * source line and a message), including Mermaid features that are not supported: a diagram
   * is never drawn with parts missing.
   */
  async fromMermaid(source: string): Promise<ChartSpec> {
    const glue = await load();
    try {
      return JSON.parse(glue.layoutMermaid(source)) as ChartSpec;
    } catch (e) {
      throw FacetryError.from(e);
    }
  },
};
