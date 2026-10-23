//! Pure-Rust chart model: the spec, validation, scales, layout and the scene. Deliberately
//! free of GPU and browser dependencies so it can be unit-tested natively and reused by any
//! renderer.

mod build;
mod color;
pub mod scale;
pub mod scene;
pub mod spec;
mod text;
mod theme;
mod validate;

pub use build::{build_scene, build_scene_with_theme};
pub use color::{Color, ParseColorError};
pub use scene::Scene;
pub use spec::ChartSpec;
pub use text::TextMeasure;
pub use theme::Theme;
pub use validate::{Issue, parse_spec, parse_spec_json, validate};
