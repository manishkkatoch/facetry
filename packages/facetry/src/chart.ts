import type {
  Chart as WasmChart,
  CameraState,
  CameraUpdate,
  ChartSpec,
} from "../wasm/facetry_wasm.js";
import { FacetryError } from "./errors.js";
import { loadModule, webgpuLooksUsable, type ModuleKind } from "./wasm.js";

export interface ChartOptions {
  /**
   * GPU API. `"auto"` (default) uses WebGPU when the browser has it, else WebGL2; if WebGPU
   * then fails to start, it retries once on WebGL2. `"webgpu"` and `"webgl2"` never fall back.
   * Only the module for the chosen API is downloaded.
   */
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
  /** Releases the pointer/wheel listeners added in the constructor. */
  readonly #removeListeners: () => void;
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
    this.#removeListeners = this.#attachOrbitControls();
    this.#syncInteraction();
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
    const wanted = options.backend ?? "auto";
    try {
      const first: ModuleKind =
        wanted === "webgl2" || (wanted === "auto" && !(await webgpuLooksUsable()))
          ? "webgl2"
          : "webgpu";
      try {
        return await Chart.#createOn(first, canvas, spec, options);
      } catch (e) {
        // A WebGPU false positive (adapter or device creation fails after `navigator.gpu`
        // looked fine). Bad specs and options would fail the same way on WebGL2.
        const code = FacetryError.from(e).code;
        if (wanted !== "auto" || first !== "webgpu" || code === "invalid-spec" || code === "invalid-options") {
          throw e;
        }
        return await Chart.#createOn("webgl2", canvas, spec, options);
      }
    } catch (e) {
      throw FacetryError.from(e);
    }
  }

  static async #createOn(
    kind: ModuleKind,
    canvas: HTMLCanvasElement,
    spec: ChartSpec,
    options: ChartOptions,
  ): Promise<Chart> {
    const glue = await loadModule(kind);
    // The GPU surface is configured from the canvas's pixel size, so set it first.
    const { dpr } = sizeCanvas(canvas);
    // The module only contains one backend, so it must be told which; "auto" means "yours".
    const inner = await glue.Chart.create(canvas, spec, { ...options, backend: kind }, dpr);
    return new Chart(canvas, inner);
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
    this.#syncInteraction();
    this.requestRender();
  }

  /**
   * The 3D camera: `{ type, azimuth, elevation, distance }` (degrees; distance in plot-box
   * radii). For a 2D chart these are the defaults and have no effect.
   */
  get camera(): CameraState {
    return this.#inner.camera;
  }

  /**
   * Moves the 3D camera; any subset of `type`, `azimuth`, `elevation`, `distance`. Values are
   * clamped (elevation 5-85 degrees). Does not rebuild the chart, only redraws. Does nothing
   * in a 2D chart.
   */
  setCamera(update: CameraUpdate): void {
    if (this.#destroyed) return;
    try {
      this.#inner.setCamera(update);
    } catch (e) {
      throw FacetryError.from(e);
    }
    this.requestRender();
  }

  /** Returns the 3D camera to the spec's `camera` (or the defaults). */
  resetCamera(): void {
    if (this.#destroyed) return;
    this.#inner.resetCamera();
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
    this.#removeListeners();
    cancelAnimationFrame(this.#raf);
    this.#raf = 0;
    this.#rejectFirstFrame(new FacetryError("destroyed", "chart destroyed before first frame"));
    this.#inner.free();
  }

  /** Cursor and touch behaviour follow the spec: orbiting may be switched on or off by `update`. */
  #syncInteraction(): void {
    const on = this.#inner.orbitEnabled;
    this.#canvas.style.touchAction = on ? "none" : "";
    this.#canvas.style.cursor = on ? "grab" : "";
  }

  /**
   * Drag to orbit, wheel to zoom, double-click to reset. The handlers are always attached
   * but do nothing unless the chart is 3D with `interaction.orbit` on. They only change the
   * camera inside Rust and ask for a frame; the scene is not rebuilt.
   */
  #attachOrbitControls(): () => void {
    const canvas = this.#canvas;
    let drag: { id: number; x: number; y: number } | null = null;

    const down = (e: PointerEvent) => {
      if (!this.#inner.orbitEnabled || e.button !== 0 || drag) return;
      drag = { id: e.pointerId, x: e.clientX, y: e.clientY };
      // Capture keeps the drag going when the pointer leaves the canvas.
      canvas.setPointerCapture(e.pointerId);
      canvas.style.cursor = "grabbing";
    };
    const move = (e: PointerEvent) => {
      if (!drag || e.pointerId !== drag.id) return;
      // CSS pixels, so a drag turns the chart by the same angle on any screen density.
      this.#inner.orbit(e.clientX - drag.x, e.clientY - drag.y);
      drag.x = e.clientX;
      drag.y = e.clientY;
      this.requestRender();
    };
    const up = (e: PointerEvent) => {
      if (!drag || e.pointerId !== drag.id) return;
      drag = null;
      if (canvas.hasPointerCapture(e.pointerId)) canvas.releasePointerCapture(e.pointerId);
      canvas.style.cursor = this.#inner.orbitEnabled ? "grab" : "";
    };
    const wheel = (e: WheelEvent) => {
      if (!this.#inner.orbitEnabled) return;
      // Keep the page from scrolling while the pointer is over an interactive chart.
      e.preventDefault();
      // Normalise to pixels: some mice report lines or pages.
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? canvas.clientHeight : 1;
      this.#inner.zoom(e.deltaY * unit);
      this.requestRender();
    };
    const dblclick = () => {
      if (!this.#inner.orbitEnabled) return;
      this.#inner.resetCamera();
      this.requestRender();
    };

    canvas.addEventListener("pointerdown", down);
    canvas.addEventListener("pointermove", move);
    canvas.addEventListener("pointerup", up);
    canvas.addEventListener("pointercancel", up);
    // Not passive: we call preventDefault to stop the page scrolling.
    canvas.addEventListener("wheel", wheel, { passive: false });
    canvas.addEventListener("dblclick", dblclick);
    return () => {
      canvas.removeEventListener("pointerdown", down);
      canvas.removeEventListener("pointermove", move);
      canvas.removeEventListener("pointerup", up);
      canvas.removeEventListener("pointercancel", up);
      canvas.removeEventListener("wheel", wheel);
      canvas.removeEventListener("dblclick", dblclick);
    };
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
