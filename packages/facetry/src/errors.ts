export type FacetryErrorCode =
  | "gpu-unavailable"
  | "invalid-options"
  | "render-failed"
  | "destroyed"
  | "unknown";

/** Every failure from this library is a `FacetryError`; switch on `code`. */
export class FacetryError extends Error {
  readonly code: FacetryErrorCode;

  constructor(code: FacetryErrorCode, message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "FacetryError";
    this.code = code;
  }

  /** Converts whatever the wasm module threw (an `Error` with a `code` property). */
  static from(e: unknown): FacetryError {
    if (e instanceof FacetryError) return e;
    const code = (e as { code?: unknown } | null)?.code;
    const known: FacetryErrorCode[] = ["gpu-unavailable", "invalid-options", "render-failed"];
    return new FacetryError(
      known.find((k) => k === code) ?? "unknown",
      e instanceof Error ? e.message : String(e),
      { cause: e },
    );
  }
}
