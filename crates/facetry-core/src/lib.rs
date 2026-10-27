//! Pure-Rust chart model: the spec, validation, scales, layout and the scene. Deliberately
//! free of GPU and browser dependencies so it can be unit-tested natively and reused by any
//! renderer.

pub mod anim;
mod build;
mod build3d;
mod build_arc;
mod build_arc3d;
mod color;
pub mod diagram;
mod diagram_glossy;
mod diagram_groups;
mod diagram_hops;
mod diagram_isometric;
mod diagram_schematic;
pub mod fill;
pub mod icons;
mod legend;
mod pathmorph;
pub mod scale;
pub mod scene;
pub mod scene3d;
pub mod spec;
mod spec_read;
pub mod svg_path;
mod text;
mod theme;
pub mod transform;
pub mod transition;
pub mod transition3d;
mod validate;
pub mod value;

pub use build::{build_scene, build_scene_with_theme};
pub use build3d::{build_scene_3d, build_scene_3d_with_theme, framing_radius};
pub use color::{Color, ParseColorError};
pub use diagram::build_diagram_3d;
pub use legend::{series_names, set_series_visible, settle_hidden};
pub use scene::Scene;
pub use scene3d::Scene3d;
pub use spec::ChartSpec;
pub use spec_read::{parse_spec_value, parse_transition_value};
pub use text::TextMeasure;
pub use theme::Theme;
pub use validate::{Issue, parse_spec_json, validate, validate_transition};
pub use value::Value;
