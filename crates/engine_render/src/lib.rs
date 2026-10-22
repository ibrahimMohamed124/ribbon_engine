//! Backend-neutral frame contracts, rendering graph, presenters, and GPU context.
//!
//! Scene extraction and rendering policy live in plugins; this crate owns presentation primitives.

pub mod frame;
pub mod graph;
pub mod present;
// Keep diagnostics backend-neutral and available even when GPU defaults are disabled.
pub mod profiling;

#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg(feature = "gpu")]
pub mod gpu_window;

pub use frame::{DrawCommand, DrawList, Effect, FramePlan, Framebuffer, Renderer};
pub use profiling::{
    RenderDiagnostics, RenderDurationSummary, RenderGraphTiming, RenderPassTiming,
    RenderProfilePlugin, RenderProfileRecorder, RenderProfileSample, RenderProfileSummary,
    RenderResourceStats,
};
