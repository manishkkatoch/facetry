//! Pure-Rust chart model: the spec, validation, scales, layout and the scene. Deliberately
//! free of GPU and browser dependencies so it can be unit-tested natively and reused by any
//! renderer.

mod build;
mod build3d;
mod color;
pub mod scale;
pub mod scene;
pub mod scene3d;
pub mod spec;
mod text;
mod theme;
mod validate;

pub use build::{build_scene, build_scene_with_theme};
pub use build3d::{build_scene_3d, build_scene_3d_with_theme, framing_radius};
pub use color::{Color, ParseColorError};
pub use scene::Scene;
pub use scene3d::Scene3d;
pub use spec::ChartSpec;
pub use text::TextMeasure;
pub use theme::Theme;
pub use validate::{Issue, parse_spec, parse_spec_json, validate};
