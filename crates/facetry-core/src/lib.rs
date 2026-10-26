//! Pure-Rust chart model: the spec, validation, scales, layout and the scene. Deliberately
//! free of GPU and browser dependencies so it can be unit-tested natively and reused by any
//! renderer.

mod build;
mod build3d;
mod build_arc;
mod build_arc3d;
mod color;
mod legend;
pub mod scale;
pub mod scene;
pub mod scene3d;
pub mod spec;
mod spec_read;
mod text;
mod theme;
pub mod transform;
mod validate;
pub mod value;

pub use build::{build_scene, build_scene_with_theme};
pub use build3d::{build_scene_3d, build_scene_3d_with_theme, framing_radius};
pub use color::{Color, ParseColorError};
pub use legend::{series_names, set_series_visible, settle_hidden};
pub use scene::Scene;
pub use scene3d::Scene3d;
pub use spec::ChartSpec;
pub use spec_read::parse_spec_value;
pub use text::TextMeasure;
pub use theme::Theme;
pub use validate::{Issue, parse_spec_json, validate};
pub use value::Value;
