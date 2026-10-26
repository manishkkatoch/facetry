import { expect, test, type Page } from "@playwright/test";
import fs from "node:fs";

// What a frame of a transition costs on the CPU, for the biggest charts we expect (about a
// thousand marks). `seek` is the work of moving the transition to a time (morphing every mark,
// tessellating the lines and slices again, uploading buffers); `render` is recording and
// submitting the frame. The browser here draws with software GL, so only `seek` says anything
// about the library. The numbers are written to test-results/frame-cost.json and printed.

test.use({ viewport: { width: 800, height: 500 }, deviceScaleFactor: 1, reducedMotion: "no-preference" });
test.setTimeout(180_000);

type Spec = any;
const results: Record<string, { marks: number; seekMedian: number; seekP95: number; renderMedian: number }> = {};

function bars(n: number, seed: number): Spec {
  let s = seed;
  const rand = () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
  const names = Array.from({ length: n }, (_, i) => `c${i}`);
  return {
    data: { t: { name: names, value: names.map(() => Math.round(rand() * 100)) } },
    axes: [{ id: "x", orient: "bottom" }, { id: "y", orient: "left" }],
    layers: [{ mark: "bar", data: "t", encoding: { x: { field: "name" }, y: { field: "value" } } }],
  };
}

function stacked(n: number, series: number, seed: number): Spec {
  let s = seed;
  const rand = () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
  const q: string[] = [], p: string[] = [], v: number[] = [];
  for (let i = 0; i < n; i++) for (let k = 0; k < series; k++) { q.push(`c${i}`); p.push(`s${k}`); v.push(Math.round(rand() * 50)); }
  return {
    data: { t: { q, p, v } },
    axes: [{ id: "x", orient: "bottom" }, { id: "y", orient: "left" }],
    layers: [{ mark: "bar", data: "t", position: "stack", encoding: { x: { field: "q" }, y: { field: "v" }, color: { field: "p" } } }],
  };
}

function lines(points: number, series: number, seed: number): Spec {
  let s = seed;
  const rand = () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
  const x: number[] = [], y: number[] = [], g: string[] = [];
  for (let k = 0; k < series; k++) for (let i = 0; i < points; i++) { x.push(i); y.push(Math.round(rand() * 100)); g.push(`s${k}`); }
  return {
    data: { t: { x, y, g } },
    axes: [{ id: "x", orient: "bottom" }, { id: "y", orient: "left" }],
    layers: [{ mark: "line", data: "t", point: true, encoding: { x: { field: "x" }, y: { field: "y" }, color: { field: "g" } } }],
  };
}

function bars3d(nx: number, nz: number, seed: number): Spec {
  let s = seed;
  const rand = () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
  const x: string[] = [], z: string[] = [], v: number[] = [];
  for (let i = 0; i < nx; i++) for (let k = 0; k < nz; k++) { x.push(`x${i}`); z.push(`z${k}`); v.push(Math.round(rand() * 100)); }
  return {
    dimension: "3d",
    data: { t: { x, z, v } },
    axes: [{ id: "x", orient: "bottom" }, { id: "y", orient: "left" }, { id: "z", orient: "depth" }],
    layers: [{ mark: "bar", data: "t", encoding: { x: { field: "x" }, y: { field: "v" }, z: { field: "z" }, color: { field: "z" } } }],
  };
}

function pie(n: number, seed: number): Spec {
  let s = seed;
  const rand = () => ((s = (s * 1664525 + 1013904223) >>> 0) / 4294967296);
  const names = Array.from({ length: n }, (_, i) => `c${i}`);
  return {
    data: { t: { name: names, value: names.map(() => 1 + Math.round(rand() * 100)) } },
    layers: [{ mark: "arc", data: "t", style: { innerRadius: 0.5 }, encoding: { theta: { field: "value" }, color: { field: "name" } } }],
    legend: false,
  };
}

async function measure(page: Page, name: string, first: Spec, second: Spec, marks: number) {
  await page.goto("/?backend=webgl2&ui=0&clock=manual");
  await expect(page.locator("html")).toHaveAttribute("data-ready", "true");
  await page.evaluate((s) => (window as any).facetryChart.update(s, { transition: false }), first);
  const out = await page.evaluate((second) => {
    const chart = (window as any).facetryChart;
    chart.update(second, { transition: { duration: 400 } });
    const seek: number[] = [];
    const render: number[] = [];
    // Warm up (shader and buffer allocation), then 30 times across the transition, `seek` the
    // mean of 20 repeats (the timer is coarse: a frame of bars is far below its resolution).
    chart.__timeFrame(10);
    for (let i = 1; i <= 30; i++) {
      const t = chart.__timeFrame((i / 31) * 400, 20);
      seek.push(t.seek);
      render.push(t.render);
    }
    const med = (a: number[]) => [...a].sort((x, y) => x - y)[Math.floor(a.length / 2)]!;
    const p95 = (a: number[]) => [...a].sort((x, y) => x - y)[Math.floor(a.length * 0.95)]!;
    return { seekMedian: med(seek), seekP95: p95(seek), renderMedian: med(render) };
  }, second);
  results[name] = { marks, ...out };
  console.log(`${name.padEnd(34)} ${String(marks).padStart(5)} marks   seek ${out.seekMedian.toFixed(3)} ms (p95 ${out.seekP95.toFixed(3)})   render ${out.renderMedian.toFixed(3)} ms`);
  return out;
}

test("what a frame of a transition costs", async ({ page }) => {
  const rows: [string, Spec, Spec, number][] = [
    ["2D bars, 1000 categories", bars(1000, 1), bars(1000, 2), 1000],
    ["2D stacked bars, 250 x 4", stacked(250, 4, 1), stacked(250, 4, 2), 1000],
    ["2D lines, 4 x 250 points + markers", lines(250, 4, 1), lines(250, 4, 2), 2000],
    ["2D donut, 200 slices", pie(200, 1), pie(200, 2), 200],
    ["3D bars, 30 x 30 grid", bars3d(30, 30, 1), bars3d(30, 30, 2), 900],
  ];
  for (const [name, a, b, marks] of rows) {
    const out = await measure(page, name, a, b, marks);
    // A frame must leave the rest of the 16 ms for the browser: generous bound for a slow CI.
    expect(out.seekMedian, name).toBeLessThan(30);
  }
  fs.mkdirSync("test-results", { recursive: true });
  fs.writeFileSync("test-results/frame-cost.json", JSON.stringify(results, null, 2));
});
