//! GPU rendering on top of wgpu. Compiles natively and for `wasm32-unknown-unknown`.

#[cfg(all(
    target_arch = "wasm32",
    not(any(feature = "webgpu", feature = "webgl"))
))]
compile_error!("on wasm32, enable the `webgpu` and/or `webgl` feature of facetry-render");

mod camera;
mod error;
mod gpu;
mod rect;
mod renderer;
mod solid;
mod text;

pub use camera::{Camera, MAX_DISTANCE, MAX_ELEVATION, MIN_DISTANCE, MIN_ELEVATION, Projection};
pub use error::Error;
pub use renderer::{BackendPreference, Renderer};

#[cfg(test)]
mod shader_tests {
    //! There is no GPU in CI's unit-test job, but naga (the shader translator inside wgpu)
    //! runs anywhere: if these pass, the shaders parse, validate, and translate to GLSL ES.

    use naga::back::glsl;
    use naga::valid::{Capabilities, ValidationFlags, Validator};

    fn check(name: &str, source: &str, globals_size: u32) {
        let module = naga::front::wgsl::parse_str(source).unwrap_or_else(|e| panic!("{name}: {e}"));
        // The uniform block must match the Rust struct and be a multiple of 16 bytes (a
        // WebGL2 requirement that native validation does not enforce).
        let globals = module
            .types
            .iter()
            .find(|(_, t)| t.name.as_deref() == Some("Globals"))
            .map(|(_, t)| match &t.inner {
                naga::TypeInner::Struct { span, .. } => *span,
                other => panic!("{other:?}"),
            })
            .expect("Globals struct");
        assert_eq!(globals, globals_size, "{name}: Globals size");
        let info = Validator::new(ValidationFlags::all(), Capabilities::empty())
            .validate(&module)
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        for stage in [naga::ShaderStage::Vertex, naga::ShaderStage::Fragment] {
            let entry = module
                .entry_points
                .iter()
                .find(|e| e.stage == stage)
                .unwrap();
            let options = glsl::Options {
                version: glsl::Version::Embedded {
                    version: 300,
                    is_webgl: true,
                },
                ..Default::default()
            };
            let pipeline = glsl::PipelineOptions {
                shader_stage: stage,
                entry_point: entry.name.clone(),
                multiview: None,
            };
            let mut out = String::new();
            glsl::Writer::new(
                &mut out,
                &module,
                &info,
                &options,
                &pipeline,
                naga::proc::BoundsCheckPolicies::default(),
            )
            .unwrap_or_else(|e| panic!("{name} {stage:?}: {e}"))
            .write()
            .unwrap_or_else(|e| panic!("{name} {stage:?}: {e}"));
            assert!(out.contains("#version 300 es"));
        }
    }

    #[test]
    fn rect_shader_is_valid_and_webgl2_translatable() {
        check("rect", include_str!("shaders/rect.wgsl"), 32);
    }

    #[test]
    fn text_shader_is_valid_and_webgl2_translatable() {
        check("text", include_str!("shaders/text.wgsl"), 32);
    }

    // The 3D shaders share one 144-byte `Globals` (see `gpu::Globals3`).
    #[test]
    fn box_shader_is_valid_and_webgl2_translatable() {
        check("box", include_str!("shaders/box.wgsl"), 144);
    }

    #[test]
    fn line3d_shader_is_valid_and_webgl2_translatable() {
        check("line3d", include_str!("shaders/line3d.wgsl"), 144);
    }

    #[test]
    fn text3d_shader_is_valid_and_webgl2_translatable() {
        check("text3d", include_str!("shaders/text3d.wgsl"), 144);
    }
}
