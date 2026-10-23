export { Chart, type ChartOptions } from "./chart.js";
export { FacetryError, type FacetryErrorCode, type SpecIssue } from "./errors.js";
// The spec types are generated from the Rust definitions (tsify), so they cannot drift.
export type {
  AxisSpec,
  ChartSpec,
  ColorEncoding,
  Dimension,
  Encoding,
  KeyEncoding,
  LayerSpec,
  Mark,
  Orient,
  PositionEncoding,
  ScaleSpec,
} from "../wasm/facetry_wasm.js";
