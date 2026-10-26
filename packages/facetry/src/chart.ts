import type {
  Chart as WasmChart,
  CameraState,
  CameraTweenOptions,
  CameraUpdate,
  ChartSpec,
  MarkInfo,
  UpdateOptions,
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
  /**
   * Animate the first frame too: the marks enter (bars grow from their baseline, text fades in)
   * with the spec's `transition`. Default `false`: the chart is simply there. Never animates
   * when the viewer prefers reduced motion.
   */
  animateIn?: boolean;
}

/** What hover and click events report: the mark (bar, line point or pie slice), and the row of data it was drawn from. */
export type MarkEvent = MarkInfo;

/** The events {@link Chart.on} takes. `null` means "no bar" (the pointer left every bar). */
export interface ChartEvents {
  /** The bar under the pointer changed. */
  hover: MarkEvent | null;
  /**
   * The pointer was pressed and released on a bar (or on empty space: `null`) without
   * dragging. A drag that orbits a 3D chart never clicks.
   */
  click: MarkEvent | null;
  /**
   * A legend entry was clicked and its series toggled; fired after the chart has been laid
   * out again. `visible` is the series' new state. Not fired for a click that changed nothing
   * (the last visible series cannot be hidden), nor for {@link Chart.setSeriesVisible}.
   */
  legendclick: LegendClickEvent;
}

/** What a `legendclick` event reports. */
export interface LegendClickEvent {
  /** The series (colour category) that was toggled, as the legend spells it. */
  series: string;
  /** Whether it is shown now. */
  visible: boolean;
}

export interface SnapshotOptions {
  /**
   * Pixels per CSS pixel of the picture: 1 gives the canvas's size in CSS pixels, 2 twice
   * that. Default: the current `devicePixelRatio`, which reproduces the canvas exactly. At
   * most 4, and never beyond the GPU's texture size limit. The chart is laid out again at
   * this scale, so text stays crisp.
   */
  scale?: number;
  /** `"theme"` (default): the chart's background colour. `"transparent"`: no background. */
  background?: "theme" | "transparent";
}

/** The CSS media query that asks for less movement. */
const REDUCED_MOTION = "(prefers-reduced-motion: reduce)";

/** Pointer movement (CSS pixels) beyond which a press is a drag, not a click. */
const CLICK_SLOP = 4;
/** Give up on a snapshot the GPU never finishes copying after this long. */
const SNAPSHOT_TIMEOUT_MS = 15_000;

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
  readonly #listeners: { [K in keyof ChartEvents]: Set<(e: ChartEvents[K]) => void> } = {
    hover: new Set(),
    click: new Set(),
    legendclick: new Set(),
  };
  /** The bar under the pointer (index into the chart's bars), -1 for none. */
  #hover = -1;
  /** Where the pointer last was over the canvas, in CSS pixels; null once it left. */
  #pointer: { x: number; y: number } | null = null;
  /** True while a drag is orbiting the chart. */
  #dragging = false;
  /** The last queued snapshot, so the next one starts after it (the GPU copy is not reentrant). */
  #snapshotTail: Promise<void> | null = null;
  /** `prefers-reduced-motion: reduce`, live; null where `matchMedia` does not exist. */
  readonly #reducedMotion: MediaQueryList | null;
  readonly #onReducedMotion: () => void;
  /** Frames drawn so far (see {@link renderCount}). */
  #renderCount = 0;
  /** While not null the animation clock is held by {@link __animationClock}, at this many ms. */
  #manualClock: number | null = null;

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
    this.#reducedMotion = typeof matchMedia === "function" ? matchMedia(REDUCED_MOTION) : null;
    // A viewer who asks for reduced motion in the middle of a transition gets its end at once.
    this.#onReducedMotion = () => {
      if (this.#reducedMotion?.matches && this.#inner.animating) {
        this.#inner.finishTransition();
        this.requestRender();
      }
    };
    this.#reducedMotion?.addEventListener("change", this.#onReducedMotion);
    this.#removeListeners = this.#attachPointerControls();
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
    const reduced = typeof matchMedia === "function" && matchMedia(REDUCED_MOTION).matches;
    const inner = await glue.Chart.create(
      canvas,
      spec,
      { ...options, backend: kind, animateIn: (options.animateIn ?? false) && !reduced },
      dpr,
    );
    return new Chart(canvas, inner);
  }

  /**
   * Replaces the chart's spec and moves to the new picture. Marks are matched by key: new ones
   * enter, ones in both move and change colour, ones that are gone leave (see the README's
   * "Animated transitions"). The movement follows the spec's `transition` (default 400 ms,
   * `cubicInOut`), or `options.transition` for this call; `false` jumps. An update that comes
   * while a transition is running continues from where the marks are. Reduced motion
   * (`prefers-reduced-motion: reduce`) always jumps.
   *
   * Throws a {@link FacetryError} with code `invalid-spec` and a list of `issues` if the spec
   * is invalid (the previous spec stays on screen), or `invalid-options` for a bad
   * `options.transition`.
   */
  update(spec: ChartSpec, options?: UpdateOptions): void {
    if (this.#destroyed) return;
    try {
      this.#inner.update(spec, this.#motion(options));
    } catch (e) {
      throw FacetryError.from(e);
    }
    this.#syncInteraction();
    this.#resetHover();
    this.requestRender();
  }

  /**
   * Calls `listener` when the bar under the pointer changes (`"hover"`, with `null` when the
   * pointer leaves every bar) or when a bar is clicked (`"click"`). Events carry
   * `{ key, layer, datum }`: the mark's key, its layer index and the data row it was drawn
   * from as a plain object. A click on empty space reports `null`.
   */
  on<K extends keyof ChartEvents>(type: K, listener: (event: ChartEvents[K]) => void): void {
    this.#listeners[type].add(listener);
  }

  /** Removes a listener added with {@link on}. */
  off<K extends keyof ChartEvents>(type: K, listener: (event: ChartEvents[K]) => void): void {
    this.#listeners[type].delete(listener);
  }

  /**
   * Hides (`visible: false`) or shows a series: the category of the colour field that the
   * legend lists. Its marks leave every layer that colours by it (bars, stacked segments,
   * lines, pie slices, in 2D and 3D) and the chart is laid out again without them: stacks
   * restack, shares renormalise, pies divide the circle among what is left and axes refit.
   * Colours do not change, the legend entry stays (dimmed) and the 3D camera is kept.
   *
   * Returns whether anything changed. Nothing happens for a name that is not a series, for a
   * series already in that state, or when hiding would leave no series visible. Does not
   * fire `legendclick` (that is for the viewer's clicks).
   */
  setSeriesVisible(name: string, visible: boolean, options?: UpdateOptions): boolean {
    if (this.#destroyed) return false;
    let changed: boolean;
    try {
      changed = this.#inner.setSeriesVisible(name, visible, this.#motion(options));
    } catch (e) {
      throw FacetryError.from(e);
    }
    if (changed) {
      this.#resetHover();
      this.requestRender();
    }
    return changed;
  }

  /**
   * The series that are hidden now, in legend order: the spec's `legend.hidden` at first,
   * then whatever the viewer or {@link setSeriesVisible} changed. `chart.update(spec)` keeps
   * it for series that still exist, unless the new spec has `legend.hidden`.
   */
  get hiddenSeries(): string[] {
    return this.#inner.hiddenSeries;
  }

  /**
   * Where the legend entry of `series` is, `{ x, y }` in CSS pixels from the canvas's
   * top-left, or `undefined` when there is no such entry or the legend is not interactive.
   * Handy for tests and for aiming your own annotations.
   */
  legendPosition(series: string): { x: number; y: number } | undefined {
    return this.#inner.legendPosition(series);
  }

  /**
   * Where the middle of the bar with this `key` is, in CSS pixels from the canvas's top-left
   * (in 3D: where its centre projects with the camera as it is now), or `undefined` for an
   * unknown key. Handy for placing your own annotations and for tests.
   */
  markPosition(key: string): { x: number; y: number } | undefined {
    return this.#inner.markPosition(key);
  }

  /**
   * A PNG of the chart exactly as it is now: the current spec and, in 3D, the current orbit
   * camera. The chart is drawn again into an offscreen texture (so the canvas is not
   * disturbed and keeps rendering), without any hover highlight or hover-only label. See
   * {@link SnapshotOptions}.
   */
  snapshot(options: SnapshotOptions = {}): Promise<Blob> {
    let scale: number;
    let transparent: boolean;
    try {
      if (this.#destroyed) throw new FacetryError("destroyed", "chart is destroyed");
      scale = options.scale ?? (window.devicePixelRatio || 1);
      if (typeof scale !== "number" || !Number.isFinite(scale) || scale <= 0) {
        throw new FacetryError("invalid-options", "`scale` must be a positive number");
      }
      const background = options.background ?? "theme";
      if (background !== "theme" && background !== "transparent") {
        throw new FacetryError(
          "invalid-options",
          `\`background\` must be "theme" or "transparent", got ${JSON.stringify(background)}`,
        );
      }
      transparent = background === "transparent";
    } catch (e) {
      return Promise.reject(e);
    }
    // The first snapshot starts right now (so it shows the state at the time of the call);
    // later ones wait their turn.
    const job = () => this.#takeSnapshot(scale, transparent);
    const result = this.#snapshotTail ? this.#snapshotTail.then(job) : job();
    const tail = result.then(
      () => {},
      () => {},
    );
    this.#snapshotTail = tail;
    void tail.then(() => {
      if (this.#snapshotTail === tail) this.#snapshotTail = null;
    });
    return result;
  }

  /**
   * Saves {@link snapshot} as a file through the browser's download mechanism. `options` are
   * those of `snapshot`.
   */
  async downloadPNG(filename = "facetry.png", options?: SnapshotOptions): Promise<void> {
    const blob = await this.snapshot(options);
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = filename;
    link.style.display = "none";
    document.body.append(link);
    link.click();
    link.remove();
    // Not at once: some browsers start the download after the click returns.
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  async #takeSnapshot(scale: number, transparent: boolean): Promise<Blob> {
    let meta: { width: number; height: number };
    try {
      meta = this.#inner.snapshotStart(scale, transparent);
    } catch (e) {
      throw FacetryError.from(e);
    }
    const started = performance.now();
    for (;;) {
      if (this.#destroyed) throw new FacetryError("destroyed", "chart destroyed during a snapshot");
      let pixels: Uint8ClampedArray | undefined;
      try {
        pixels = this.#inner.snapshotPoll();
      } catch (e) {
        throw FacetryError.from(e);
      }
      if (pixels) return encodePng(pixels, meta.width, meta.height);
      if (performance.now() - started > SNAPSHOT_TIMEOUT_MS) {
        throw new FacetryError("render-failed", "the GPU did not finish the snapshot in time");
      }
      // Yield to the event loop: in browsers, that is where the GPU copy completes.
      await new Promise((resolve) => setTimeout(resolve, 0));
    }
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
   *
   * With `options` the camera turns there over time instead of jumping: `{ duration, easing }`
   * as for a transition (`{}` for 400 ms, `cubicInOut`); azimuth takes the shortest way round
   * (350 to 10 degrees passes through 0), the lens (`type`) changes at once. Dragging or zooming
   * stops a tween where it is; viewers who prefer reduced motion get the jump.
   */
  setCamera(update: CameraUpdate, options?: CameraTweenOptions): void {
    if (this.#destroyed) return;
    try {
      this.#inner.setCamera(update, this.#tween(options));
    } catch (e) {
      throw FacetryError.from(e);
    }
    this.#repick();
    this.requestRender();
  }

  /** Returns the 3D camera to the spec's `camera` (or the defaults); `options` as for {@link setCamera}. */
  resetCamera(options?: CameraTweenOptions): void {
    if (this.#destroyed) return;
    try {
      this.#inner.resetCamera(this.#tween(options));
    } catch (e) {
      throw FacetryError.from(e);
    }
    this.#repick();
    this.requestRender();
  }

  /**
   * Whether a transition is running. While it is, a frame is drawn every animation frame;
   * when it ends the last frame is drawn and the loop stops.
   */
  get animating(): boolean {
    return !this.#destroyed && this.#inner.animating;
  }

  /**
   * How many frames this chart has drawn. An idle chart draws none, which is how tests (and
   * the curious) can check that rendering stops after a transition.
   */
  get renderCount(): number {
    return this.#renderCount;
  }

  /**
   * Test hook: holds the animation clock still and sets the running transition to `ms` since
   * it began, drawing that frame at once. While held, a transition that starts later also sits
   * at its time 0 until the clock is moved again, and no animation frames are requested.
   * `null` lets the clock run again from where it is. Not part of the supported API.
   */
  __animationClock(ms: number | null): void {
    if (this.#destroyed) return;
    if (ms === null) {
      if (this.#manualClock !== null) {
        this.#manualClock = null;
        this.#inner.rebase(performance.now());
        this.requestRender();
      }
      return;
    }
    this.#manualClock = ms;
    this.#inner.seek(ms);
    this.#draw();
    this.#repick();
  }

  /**
   * Test hook: like {@link __animationClock} but reports what the frame cost, in milliseconds on
   * this page's clock: `seek` is the CPU work of moving the transition (morphing the marks,
   * tessellating lines and slices, uploading buffers) and `render` the time `render` took to
   * record and submit the frame (the GPU's own time is not in it). Not part of the supported API.
   */
  __timeFrame(ms: number, repeats = 1): { seek: number; render: number } {
    this.#manualClock = ms;
    const t0 = performance.now();
    for (let i = 0; i < repeats; i++) this.#inner.seek(ms);
    const t1 = performance.now();
    this.#draw();
    // `seek` is the mean of `repeats` runs (the timer is coarse); `render` is one run.
    return { seek: (t1 - t0) / repeats, render: performance.now() - t1 };
  }

  /**
   * Schedules a redraw on the next animation frame. Calls before that frame coalesce into
   * one. We draw only when something changed (resize, an update, a hover), never in a
   * continuous loop, so an idle chart costs no CPU/GPU and no battery. While a transition
   * runs, each frame asks for the next one until it ends.
   */
  requestRender(): void {
    if (this.#destroyed || this.#raf) return;
    this.#raf = requestAnimationFrame((now) => {
      this.#raf = 0;
      const wasAnimating = this.#inner.animating;
      if (this.#manualClock === null) this.#inner.tick(now);
      this.#draw();
      // The camera may have moved under a pointer that did not: look again at what is under it.
      if (wasAnimating) this.#repick();
      if (this.#manualClock === null && this.#inner.animating) this.requestRender();
    });
  }

  /** Draws a frame now. */
  #draw(): void {
    try {
      this.#inner.render();
      this.#renderCount++;
      this.#resolveFirstFrame();
    } catch (e) {
      const err = FacetryError.from(e);
      this.#rejectFirstFrame(err);
      console.error(err);
    }
  }

  /** The options for the wasm call: reduced motion turns every transition into a jump. */
  #motion(options?: UpdateOptions): UpdateOptions | undefined {
    return this.#reducedMotion?.matches ? { transition: false } : options;
  }

  /** Camera tween options for the wasm call: none (a jump) under reduced motion. */
  #tween(options?: CameraTweenOptions): CameraTweenOptions | undefined {
    return this.#reducedMotion?.matches ? undefined : options;
  }

  /** Stops observing and drawing, and frees the GPU resources. Safe to call twice. */
  destroy(): void {
    if (this.#destroyed) return;
    this.#destroyed = true;
    this.#observer.disconnect();
    this.#reducedMotion?.removeEventListener("change", this.#onReducedMotion);
    this.#removeListeners();
    this.#listeners.hover.clear();
    this.#listeners.click.clear();
    this.#listeners.legendclick.clear();
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
   * Pointer handling: drag to orbit, wheel to zoom, double-click to reset (3D with
   * `interaction.orbit` on), and for every chart hover and click on bars. The handlers only
   * change the camera or the hover inside Rust and ask for a frame; the scene is not rebuilt.
   *
   * A press becomes a drag once the pointer has moved more than a few pixels; until then it
   * may still be a click. The orbit then starts from where the press was, so no movement is
   * lost, and the release does not click.
   */
  #attachPointerControls(): () => void {
    const canvas = this.#canvas;
    let press: {
      id: number;
      x: number;
      y: number;
      lastX: number;
      lastY: number;
      /** The legend entry the press started on: it is a legend click, never an orbit or a chart click. */
      legend?: string;
    } | null = null;

    /** Pointer position in CSS pixels from the canvas's top-left. */
    const at = (e: MouseEvent) => {
      const box = canvas.getBoundingClientRect();
      // Divides out any CSS transform that scales the canvas.
      return {
        x: (e.clientX - box.left) * (canvas.clientWidth / (box.width || 1)),
        y: (e.clientY - box.top) * (canvas.clientHeight / (box.height || 1)),
      };
    };
    const endDrag = () => {
      this.#dragging = false;
      this.#look();
    };

    const down = (e: PointerEvent) => {
      if (e.button !== 0 || press) return;
      press = { id: e.pointerId, x: e.clientX, y: e.clientY, lastX: e.clientX, lastY: e.clientY };
      this.#pointer = at(e);
      press.legend = this.#inner.legendPick(this.#pointer.x, this.#pointer.y);
      // Capture keeps the drag going when the pointer leaves the canvas (not for a legend
      // press: that never drags).
      if (this.#inner.orbitEnabled && press.legend === undefined) canvas.setPointerCapture(e.pointerId);
    };
    const move = (e: PointerEvent) => {
      this.#pointer = at(e);
      if (!press) {
        this.#look();
        return;
      }
      if (e.pointerId !== press.id || !this.#inner.orbitEnabled || press.legend !== undefined) return;
      if (!this.#dragging) {
        if (Math.hypot(e.clientX - press.x, e.clientY - press.y) <= CLICK_SLOP) return;
        this.#dragging = true;
        canvas.style.cursor = "grabbing";
        // The chart is about to move under the pointer: nothing is hovered while it does.
        this.#setHover(-1);
        press.lastX = press.x;
        press.lastY = press.y;
      }
      // CSS pixels, so a drag turns the chart by the same angle on any screen density.
      this.#inner.orbit(e.clientX - press.lastX, e.clientY - press.lastY);
      press.lastX = e.clientX;
      press.lastY = e.clientY;
      this.requestRender();
    };
    const up = (e: PointerEvent) => {
      if (!press || e.pointerId !== press.id) return;
      const wasDrag = this.#dragging;
      const legend = press.legend;
      const moved = Math.hypot(e.clientX - press.x, e.clientY - press.y);
      press = null;
      if (canvas.hasPointerCapture(e.pointerId)) canvas.releasePointerCapture(e.pointerId);
      endDrag();
      this.#pointer = at(e);
      if (e.type === "pointercancel") return;
      if (legend !== undefined) {
        // A legend click: pressed and released on the same entry. It toggles the series and
        // is not a chart click.
        if (moved <= CLICK_SLOP && this.#inner.legendPick(this.#pointer.x, this.#pointer.y) === legend) {
          this.#toggleSeries(legend);
        }
        return;
      }
      if (wasDrag) {
        this.#repick();
        return;
      }
      // A click: it hits what the pointer is over (looked up now, so it is right even if no
      // move event came first, as with touch).
      const mark = this.#inner.pick(this.#pointer.x, this.#pointer.y);
      this.#emit("click", mark >= 0 ? this.#inner.markInfo(mark) : null);
      if (e.pointerType !== "mouse") {
        this.#pointer = null;
        this.#setHover(-1);
      }
    };
    const leave = () => {
      if (press) return;
      this.#pointer = null;
      this.#look();
    };
    const wheel = (e: WheelEvent) => {
      if (!this.#inner.orbitEnabled) return;
      // Keep the page from scrolling while the pointer is over an interactive chart.
      e.preventDefault();
      // Normalise to pixels: some mice report lines or pages.
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? canvas.clientHeight : 1;
      this.#inner.zoom(e.deltaY * unit);
      this.#repick();
      this.requestRender();
    };
    const dblclick = () => {
      if (!this.#inner.orbitEnabled) return;
      this.#inner.resetCamera(undefined);
      this.#repick();
      this.requestRender();
    };

    canvas.addEventListener("pointerdown", down);
    canvas.addEventListener("pointermove", move);
    canvas.addEventListener("pointerup", up);
    canvas.addEventListener("pointercancel", up);
    canvas.addEventListener("pointerleave", leave);
    // Not passive: we call preventDefault to stop the page scrolling.
    canvas.addEventListener("wheel", wheel, { passive: false });
    canvas.addEventListener("dblclick", dblclick);
    return () => {
      canvas.removeEventListener("pointerdown", down);
      canvas.removeEventListener("pointermove", move);
      canvas.removeEventListener("pointerup", up);
      canvas.removeEventListener("pointercancel", up);
      canvas.removeEventListener("pointerleave", leave);
      canvas.removeEventListener("wheel", wheel);
      canvas.removeEventListener("dblclick", dblclick);
    };
  }

  /** A viewer's click on a legend entry: toggles that series and tells `legendclick` listeners. */
  #toggleSeries(series: string): void {
    const visible = this.#inner.hiddenSeries.includes(series);
    if (!this.setSeriesVisible(series, visible)) return;
    this.#emit("legendclick", { series, visible });
  }

  /**
   * Looks at what is under the pointer: over a legend entry the cursor becomes a pointer
   * and no bar is hovered; otherwise the bar there (if any) is.
   */
  #look(): void {
    const p = this.#pointer;
    const entry = p ? this.#inner.legendPick(p.x, p.y) : undefined;
    this.#canvas.style.cursor = entry !== undefined ? "pointer" : this.#inner.orbitEnabled ? "grab" : "";
    this.#setHover(p && entry === undefined ? this.#inner.pick(p.x, p.y) : -1);
  }

  /** Makes `mark` the hovered bar (-1: none), redraws, and tells `hover` listeners. */
  #setHover(mark: number): void {
    if (mark === this.#hover || this.#destroyed) return;
    this.#hover = mark;
    this.#inner.setHover(mark);
    this.requestRender();
    this.#emit("hover", mark >= 0 ? this.#inner.markInfo(mark) : null);
  }

  /** Looks again at what is under the last known pointer position (the chart moved, not the pointer). */
  #repick(): void {
    if (this.#pointer && !this.#dragging) this.#look();
  }

  /** The chart was laid out again: its bars are new, so nothing is hovered until we look again. */
  #resetHover(): void {
    if (this.#hover !== -1) {
      this.#hover = -1;
      this.#emit("hover", null);
    }
    this.#repick();
  }

  #emit<K extends keyof ChartEvents>(type: K, event: ChartEvents[K]): void {
    for (const listener of [...this.#listeners[type]]) {
      try {
        listener(event);
      } catch (e) {
        // A throwing listener must not break the chart or the other listeners.
        console.error(e);
      }
    }
  }

  #syncSize(): void {
    if (this.#destroyed) return;
    const { width, height, dpr } = sizeCanvas(this.#canvas);
    // Layout depends on the size (margins, tick counts, label thinning), not just the surface.
    this.#inner.resize(width, height, dpr);
    this.#resetHover();
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

/** Encodes RGBA pixels (not premultiplied) as a PNG with the browser's own encoder. */
async function encodePng(pixels: Uint8ClampedArray, width: number, height: number): Promise<Blob> {
  const image = new ImageData(pixels as Uint8ClampedArray<ArrayBuffer>, width, height);
  if (typeof OffscreenCanvas !== "undefined") {
    const canvas = new OffscreenCanvas(width, height);
    canvas.getContext("2d")!.putImageData(image, 0, 0);
    return canvas.convertToBlob({ type: "image/png" });
  }
  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  canvas.getContext("2d")!.putImageData(image, 0, 0);
  return new Promise((resolve, reject) =>
    canvas.toBlob(
      (blob) => (blob ? resolve(blob) : reject(new FacetryError("render-failed", "PNG encoding failed"))),
      "image/png",
    ),
  );
}
