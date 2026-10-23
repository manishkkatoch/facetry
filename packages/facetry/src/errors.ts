import type { Issue } from "../wasm/facetry_wasm.js";

/** One problem with a chart spec; `path` is a JSON pointer such as `/layers/0/encoding/y/axis`. */
export type SpecIssue = Issue;

export type FacetryErrorCode =
  | "gpu-unavailable"
  | "invalid-options"
  | "invalid-spec"
  | "render-failed"
  | "destroyed"
  | "unknown";

/** Every failure from this library is a `FacetryError`; switch on `code`. */
export class FacetryError extends Error {
  readonly code: FacetryErrorCode;
  /** For `code === "invalid-spec"`: every problem found, not just the first. */
  readonly issues: SpecIssue[];

  constructor(code: FacetryErrorCode, message: string, options?: ErrorOptions & { issues?: SpecIssue[] }) {
    super(message, options);
    this.name = "FacetryError";
    this.code = code;
    this.issues = options?.issues ?? [];
  }

  /** Converts whatever the wasm module threw (an `Error` with a `code` property). */
  static from(e: unknown): FacetryError {
    if (e instanceof FacetryError) return e;
    const { code, issues } = (e ?? {}) as { code?: unknown; issues?: unknown };
    const known: FacetryErrorCode[] = [
      "gpu-unavailable",
      "invalid-options",
      "invalid-spec",
      "render-failed",
    ];
    return new FacetryError(
      known.find((k) => k === code) ?? "unknown",
      e instanceof Error ? e.message : String(e),
      { cause: e, issues: Array.isArray(issues) ? (issues as SpecIssue[]) : [] },
    );
  }
}
