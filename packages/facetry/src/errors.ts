import type { Issue } from "../wasm/facetry_wasm.js";
import type { DiagramIssue } from "../wasm/facetry_diagrams.js";

/** One problem with a chart spec; `path` is a JSON pointer such as `/layers/0/encoding/y/axis`. */
export type SpecIssue = Issue;
export type { DiagramIssue };

export type FacetryErrorCode =
  | "gpu-unavailable"
  | "invalid-options"
  | "invalid-spec"
  | "invalid-diagram"
  | "render-failed"
  | "destroyed"
  | "unknown";

/** Every failure from this library is a `FacetryError`; switch on `code`. */
export class FacetryError extends Error {
  readonly code: FacetryErrorCode;
  /** For `code === "invalid-spec"`: every problem found, not just the first. */
  readonly issues: SpecIssue[];
  /**
   * For `code === "invalid-diagram"` (from `facetry/diagrams`): every problem found in the
   * Mermaid text, each with its line and column.
   */
  readonly diagramIssues: DiagramIssue[];

  constructor(
    code: FacetryErrorCode,
    message: string,
    options?: ErrorOptions & { issues?: SpecIssue[]; diagramIssues?: DiagramIssue[] },
  ) {
    super(message, options);
    this.name = "FacetryError";
    this.code = code;
    this.issues = options?.issues ?? [];
    this.diagramIssues = options?.diagramIssues ?? [];
  }

  /** Converts whatever the wasm module threw (an `Error` with a `code` property). */
  static from(e: unknown): FacetryError {
    if (e instanceof FacetryError) return e;
    const { code, issues, diagramIssues } = (e ?? {}) as {
      code?: unknown;
      issues?: unknown;
      diagramIssues?: unknown;
    };
    const known: FacetryErrorCode[] = [
      "gpu-unavailable",
      "invalid-options",
      "invalid-spec",
      "invalid-diagram",
      "render-failed",
    ];
    return new FacetryError(
      known.find((k) => k === code) ?? "unknown",
      e instanceof Error ? e.message : String(e),
      {
        cause: e,
        issues: Array.isArray(issues) ? (issues as SpecIssue[]) : [],
        diagramIssues: Array.isArray(diagramIssues) ? (diagramIssues as DiagramIssue[]) : [],
      },
    );
  }
}
