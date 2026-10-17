//! Backend-neutral frame contracts, rendering graph, presenters, and GPU context.
//!
//! Scene extraction and rendering policy live in plugins; this crate owns presentation primitives.

pub mod frame;
pub mod graph;
pub mod present;

#[cfg(feature = "gpu")]
pub mod gpu;

pub use frame::{DrawCommand, DrawList, Effect, FramePlan, Framebuffer, Renderer};
