use std::fmt;

/// Everything that can go wrong while setting up or driving the GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// No usable GPU backend: no adapter, no WebGL2/WebGPU, or device creation failed.
    GpuUnavailable(String),
    /// The GPU rejected a frame.
    RenderFailed(String),
}

impl Error {
    /// A stable, machine-readable identifier that JS callers can switch on.
    pub fn code(&self) -> &'static str {
        match self {
            Self::GpuUnavailable(_) => "gpu-unavailable",
            Self::RenderFailed(_) => "render-failed",
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GpuUnavailable(msg) => write!(f, "GPU unavailable: {msg}"),
            Self::RenderFailed(msg) => write!(f, "render failed: {msg}"),
        }
    }
}

impl std::error::Error for Error {}
