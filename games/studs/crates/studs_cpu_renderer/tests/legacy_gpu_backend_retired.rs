//! Architecture regression tests for the retired legacy GPU voxel raycaster.
//!
//! These tests intentionally inspect the package source tree: the removal contract is not
//! satisfied merely because normal examples happen not to instantiate the old backend.

use std::fs;

fn manifest_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn legacy_gpu_raycaster_and_its_shader_files_are_absent() {
    let root = manifest_dir();
    assert!(
        !root.join("src/gpu").exists(),
        "the removed GPU renderer module directory must not remain"
    );
    assert!(
        !root.join("shaders/voxel").exists(),
        "the old voxel raycaster shader directory must not remain"
    );
    for relative in [
        "src/gpu/mod.rs",
        "src/gpu/voxel.rs",
        "shaders/voxel/world.wgsl",
        "shaders/voxel/ui.wgsl",
        "shaders/voxel/present.wgsl",
        "tests/parity.rs",
    ] {
        assert!(
            !root.join(relative).exists(),
            "retired legacy GPU raycaster file still exists: {relative}"
        );
    }
}

#[test]
fn rendering_crate_does_not_reintroduce_a_gpu_raycaster_feature_or_dependencies() {
    let root = manifest_dir();
    let manifest =
        fs::read_to_string(root.join("Cargo.toml")).expect("rendering Cargo.toml must be readable");
    let features = manifest
        .split("[features]")
        .nth(1)
        .and_then(|tail| tail.split("[dependencies]").next())
        .expect("rendering Cargo.toml must have a features section before dependencies");
    assert!(features
        .lines()
        .any(|line| line.trim() == "default = [\"cpu\"]"));
    assert!(
        !features
            .lines()
            .any(|line| line.trim_start().starts_with("gpu =")),
        "the retired legacy GPU raycaster feature must not be reintroduced"
    );
    for dependency in ["wgpu =", "pollster =", "bytemuck ="] {
        assert!(
            !manifest
                .lines()
                .any(|line| line.trim_start().starts_with(dependency)),
            "legacy renderer-only dependency remains in rendering Cargo.toml: {dependency}"
        );
    }
}

#[test]
fn generic_ecs_bridge_is_cpu_only_and_does_not_reference_the_removed_renderer() {
    let root = manifest_dir();
    let source =
        fs::read_to_string(root.join("src/ecs.rs")).expect("rendering ECS bridge must be readable");
    for forbidden in [
        "GpuBackend",
        "GpuRenderer",
        "GpuPresenter",
        "render_gpu_frame_system",
        "RenderBackend",
        "crate::gpu",
    ] {
        assert!(
            !source.contains(forbidden),
            "generic ECS bridge still references retired GPU backend symbol: {forbidden}"
        );
    }

    let lib =
        fs::read_to_string(root.join("src/lib.rs")).expect("rendering crate root must be readable");
    assert!(
        !lib.contains("pub mod gpu"),
        "the legacy GPU module must not be exported"
    );
    assert!(root.join("src/cpu/render/raycaster.rs").is_file());
}

#[test]
fn active_workspace_rust_sources_do_not_reference_the_retired_gpu_api() {
    let workspace = manifest_dir().join("../../../../");
    let guard_path = manifest_dir().join("tests/legacy_gpu_backend_retired.rs");
    let guard_path = guard_path
        .canonicalize()
        .expect("the source-guard test must have a canonical path");
    let forbidden = [
        "studs_cpu_renderer::gpu",
        "GpuRenderer",
        "GpuPresenter",
        "render_gpu_frame_system",
        "RenderBackend",
    ];
    for relative in ["crates", "plugins", "games", "examples"] {
        let tree = workspace.join(relative);
        for entry in walk_rust_sources(&tree) {
            let canonical_entry = entry.canonicalize().unwrap_or_else(|error| {
                panic!("could not resolve Rust source {}: {error}", entry.display())
            });
            if canonical_entry == guard_path {
                continue;
            }
            let source = fs::read_to_string(&entry).unwrap_or_else(|error| {
                panic!("could not read Rust source {}: {error}", entry.display())
            });
            for &symbol in &forbidden {
                assert!(
                    !source.contains(symbol),
                    "retired backend symbol `{symbol}` remains in active Rust source {}",
                    entry.display()
                );
            }
        }
    }
}

#[test]
fn studs_selects_the_engine_mesh_renderer_without_coupling_to_legacy_ecs_backend() {
    let workspace = manifest_dir().join("../../../../");
    // The STUDS app was split into the `src/app/` module directory; inspect its module root
    // rather than the pre-refactor `src/app.rs` path.
    let app = fs::read_to_string(workspace.join("games/studs/src/app/mod.rs"))
        .expect("STUDS app module source must be readable");
    assert!(app.contains("OffscreenVoxelRenderer"));
    assert!(app.contains("StudsRendererMode"));
    assert!(app.contains("render_voxel_world"));
    assert!(app.contains("fall_back_to_cpu"));
    assert!(!app.contains("studs_cpu_renderer::gpu"));
    assert!(!app.contains("RenderBackend"));

    let cli = fs::read_to_string(workspace.join("games/studs/src/main.rs"))
        .expect("STUDS CLI source must be readable");
    assert!(cli.contains("studs::app::StudsRendererMode"));
    assert!(!cli.contains("studs_cpu_renderer::ecs::RenderBackend"));
}

fn walk_rust_sources(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut result = Vec::new();
    let entries = fs::read_dir(directory).unwrap_or_else(|error| {
        panic!(
            "could not read source directory {}: {error}",
            directory.display()
        )
    });
    for entry in entries {
        let entry = entry.expect("directory entry must be readable");
        let path = entry.path();
        if path.is_dir() {
            result.extend(walk_rust_sources(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            result.push(path);
        }
    }
    result
}
