//! GPU rendering on top of wgpu. Compiles natively and for `wasm32-unknown-unknown`.

mod error;
mod renderer;

pub use error::Error;
pub use renderer::{BackendPreference, Renderer};
