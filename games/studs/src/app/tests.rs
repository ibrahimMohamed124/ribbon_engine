use super::{default_backend, RendererImpl, StudsRenderer, StudsRendererMode};
use crate::input::{Controls, InputFrame};

#[test]
fn default_backend_matches_the_compiled_gpu_feature() {
    #[cfg(feature = "gpu")]
    assert_eq!(default_backend(), StudsRendererMode::Gpu);
    #[cfg(not(feature = "gpu"))]
    assert_eq!(default_backend(), StudsRendererMode::Cpu);
}

#[test]
fn explicit_cpu_mode_uses_the_cpu_renderer_without_a_warning() {
    let (renderer, warning) = StudsRenderer::new(2, 2, StudsRendererMode::Cpu);
    assert!(matches!(renderer.renderer, RendererImpl::Cpu(_)));
    assert!(warning.is_none());
}

#[cfg(not(feature = "gpu"))]
#[test]
fn gpu_request_without_gpu_feature_falls_back_to_cpu_with_a_warning() {
    let (renderer, warning) = StudsRenderer::new(2, 2, StudsRendererMode::Gpu);
    assert!(matches!(renderer.renderer, RendererImpl::Cpu(_)));
    let warning = warning.expect("unsupported GPU request must explain the fallback");
    assert!(warning.contains("without the `gpu` feature"));
}

#[test]
fn mesh_gpu_input_preserves_reference_horizontal_controls() {
    // GPU selection must not reverse the game-level controls during a renderer-only refactor.
    let frame = InputFrame {
        controls: Controls {
            mouse_yaw: 0.25,
            look_right: true,
            right: true,
            forward: true,
            ..Controls::default()
        },
        ..InputFrame::default()
    };

    let mut buffered = InputFrame::default();
    buffered.merge(frame);

    assert_eq!(buffered.controls.mouse_yaw, 0.25);
    assert!(buffered.controls.look_right);
    assert!(!buffered.controls.look_left);
    assert!(buffered.controls.right);
    assert!(!buffered.controls.left);
    assert!(buffered.controls.forward);
}
