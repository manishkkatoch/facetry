export { Chart, type ChartOptions, type SnapshotOptions } from "./chart.js";
export { FacetryError, type FacetryErrorCode, type SpecIssue } from "./errors.js";
// The spec types are generated from the Rust definitions (tsify), so they cannot drift.
export type {
  AxisSpec,
  CameraKind,
  CameraSpec,
  CameraState,
  CameraUpdate,
  ChartSpec,
  ColorEncoding,
  Dimension,
  Encoding,
  InteractionSpec,
  KeyEncoding,
  LayerSpec,
  Mark,
  Orient,
  PositionEncoding,
  ScaleSpec,
} from "../wasm/facetry_wasm.js";
