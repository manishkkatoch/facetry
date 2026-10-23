//! Pure-Rust chart model. Deliberately free of GPU and browser dependencies so it
//! can be unit-tested natively and reused by any renderer.

mod color;
mod theme;

pub use color::{Color, ParseColorError};
pub use theme::Theme;
