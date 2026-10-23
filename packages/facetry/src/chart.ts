import { Chart as WasmChart, type ChartSpec } from "../wasm/facetry_wasm.js";
import { FacetryError } from "./errors.js";
import { loadWasm } from "./wasm.js";

export interface ChartOptions {
  /** GPU API. `"auto"` (default) uses WebGPU when available, else WebGL2. */
  backend?: "auto" | "webgpu" | "webgl2";
  /** Overrides the theme's background colour (`#rrggbb` or `#rrggbbaa`). */
  clearColor?: string;
}

/**
 * A chart bound to a canvas. Create with {@link Chart.create}; call {@link destroy} when done.
 * Give the canvas a CSS size (the chart fills it); the pixel size follows automatically.
 */
export class Chart {
  readonly #canvas: HTMLCanvasElement;
  readonly #inner: WasmChart;
  readonly #observer: ResizeObserver;
  #raf = 0;
  #destroyed = false;
  #resolveFirstFrame!: () => void;
  #rejectFirstFrame!: (e: unknown) => void;

  /** Resolves after the first frame has been drawn; rejects if drawing fails first. */
  readonly firstFrame: Promise<void>;
  /** `"webgpu"` or `"webgl2"`. */
  readonly backend: string;
  /** Samples per pixel: 4 (4x MSAA) normally, 1 if the device cannot multisample. */
  readonly msaaSamples: number;

  private constructor(canvas: HTMLCanvasElement, inner: WasmChart) {
    this.#canvas = canvas;
    this.#inner = inner;
    this.backend = inner.backend;
    this.msaaSamples = inner.msaaSamples;
    this.firstFrame = new Promise((resolve, reject) => {
      this.#resolveFirstFrame = resolve;
      this.#rejectFirstFrame = reject;
    });
    // Avoid an unhandled-rejection warning when nobody awaits firstFrame.
    this.firstFrame.catch(() => {});
    this.#observer = new ResizeObserver(() => this.#syncSize());
    this.#observer.observe(canvas);
    this.requestRender();
  }

  /**
   * Rejects with a {@link FacetryError}: `invalid-spec` (see its `issues`; nothing is drawn),
   * `invalid-options`, or `gpu-unavailable`.
   */
  static async create(
    canvas: HTMLCanvasElement,
    spec: ChartSpec,
    options: ChartOptions = {},
  ): Promise<Chart> {
    try {
      await loadWasm();
      // The GPU surface is configured from the canvas's pixel size, so set it first.
      const { dpr } = sizeCanvas(canvas);
      return new Chart(canvas, await WasmChart.create(canvas, spec, options, dpr));
    } catch (e) {
      throw FacetryError.from(e);
    }
  }

  /**
   * Replaces the chart's spec and redraws (no animation yet). Throws a {@link FacetryError}
   * with code `invalid-spec` and a list of `issues` if the spec is invalid; the previous
   * spec stays on screen.
   */
  update(spec: ChartSpec): void {
    if (this.#destroyed) return;
    try {
      this.#inner.update(spec);
    } catch (e) {
      throw FacetryError.from(e);
    }
    this.requestRender();
  }

  /**
   * Schedules a redraw on the next animation frame. Calls before that frame coalesce into
   * one. We draw only when something changed (resize, new data later), never in a
   * continuous loop, so an idle chart costs no CPU/GPU and no battery.
   */
  requestRender(): void {
    if (this.#destroyed || this.#raf) return;
    this.#raf = requestAnimationFrame(() => {
      this.#raf = 0;
      try {
        this.#inner.render();
        this.#resolveFirstFrame();
      } catch (e) {
        const err = FacetryError.from(e);
        this.#rejectFirstFrame(err);
        console.error(err);
      }
    });
  }

  /** Stops observing and drawing, and frees the GPU resources. Safe to call twice. */
  destroy(): void {
    if (this.#destroyed) return;
    this.#destroyed = true;
    this.#observer.disconnect();
    cancelAnimationFrame(this.#raf);
    this.#raf = 0;
    this.#rejectFirstFrame(new FacetryError("destroyed", "chart destroyed before first frame"));
    this.#inner.free();
  }

  #syncSize(): void {
    if (this.#destroyed) return;
    const { width, height, dpr } = sizeCanvas(this.#canvas);
    // Layout depends on the size (margins, tick counts, label thinning), not just the surface.
    this.#inner.resize(width, height, dpr);
    this.requestRender();
  }
}

/** Sets the canvas backing store to its CSS size times devicePixelRatio (crisp on HiDPI). */
function sizeCanvas(canvas: HTMLCanvasElement): { width: number; height: number; dpr: number } {
  const dpr = window.devicePixelRatio || 1;
  const width = Math.round(canvas.clientWidth * dpr);
  const height = Math.round(canvas.clientHeight * dpr);
  if (canvas.width !== width) canvas.width = width;
  if (canvas.height !== height) canvas.height = height;
  return { width, height, dpr };
}
