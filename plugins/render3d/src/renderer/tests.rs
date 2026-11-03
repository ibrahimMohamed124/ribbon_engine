use super::{
    build_mips, pack_point_lights, select_point_lights, select_spot_lights, Draw, MeshRenderer,
    PbrMaterialDesc, PbrMaterialRenderOptions, RenderTarget, Scene, SkinnedDraw, TextureData,
};
use crate::components::{Camera3D, FogSettings, PointLight, SpotLight, SunLight, Transform3D};
use glam::{Mat4, Vec3};
use ribbon_plugin_gltf::AlphaMode;
use ribbon_render::gpu::GpuContext;
use ribbon_render::gpu_window::clear_view;
use ribbon_render_contracts::{
    ParticleBatch, ParticleBillboard, ParticleBlendMode, ParticleInstance, ParticleShape,
};
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;

const SIZE: u32 = 64;

struct Harness {
    context: GpuContext,
    renderer: MeshRenderer,
}

fn harness() -> Option<Harness> {
    let context = match pollster::block_on(GpuContext::new_offscreen(SIZE, SIZE)) {
        Ok(context) => context,
        Err(error) => {
            eprintln!("skipping GPU test: {}", error.0);
            return None;
        }
    };
    clear_view(
        context.device(),
        context.queue(),
        context.target().view(),
        [0.0, 0.0, 0.0, 1.0],
    );
    let renderer = MeshRenderer::new(
        context.device(),
        context.queue(),
        wgpu::TextureFormat::Rgba8Unorm,
    );
    Some(Harness { context, renderer })
}

impl Harness {
    fn draw(&mut self, draws: &[Draw]) -> Vec<u32> {
        let eye = Transform3D::looking_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, Vec3::Y);
        let view = Mat4::from_rotation_translation(eye.rotation, eye.translation).inverse();
        self.renderer.render(
            self.context.device(),
            self.context.queue(),
            self.context.target().view(),
            (SIZE, SIZE),
            &Scene {
                view_proj: Camera3D::default().projection(1.0) * view,
                camera_position: eye.translation,
                camera_rotation: glam::Quat::IDENTITY,
                sun: &SunLight::default(),
                spot_lights: &[],
                point_lights: &[],
                fog: FogSettings::default(),
                draws,
                particles: &[],
            },
        );
        self.context.readback().expect("readback").pixels
    }
}

fn pixel(pixels: &[u32], x: u32, y: u32) -> (u32, u32, u32) {
    let p = pixels[(y * SIZE + x) as usize];
    ((p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff)
}

fn cube(model: Mat4, tint: [f32; 3]) -> Draw {
    Draw {
        mesh: MeshRenderer::CUBE,
        material: MeshRenderer::WHITE,
        model,
        tint,
    }
}

#[test]
fn vertex_attributes_match_the_interleaved_joint_and_weight_storage_layout() {
    use crate::mesh::Vertex;
    use std::mem::{offset_of, size_of};

    let offset = |location| {
        super::VERTEX_ATTRIBUTES
            .iter()
            .find(|attribute| attribute.shader_location == location)
            .unwrap_or_else(|| panic!("missing vertex shader location {location}"))
            .offset
    };

    assert_eq!(size_of::<Vertex>(), 80);
    assert_eq!(offset_of!(Vertex, joints), 32);
    assert_eq!(offset_of!(Vertex, weights), 48);
    assert_eq!(offset(0), offset_of!(Vertex, position) as u64);
    assert_eq!(offset(1), offset_of!(Vertex, normal) as u64);
    assert_eq!(offset(2), offset_of!(Vertex, uv) as u64);
    assert_eq!(offset(3), offset_of!(Vertex, joints) as u64);
    assert_eq!(offset(4), offset_of!(Vertex, weights) as u64);
    assert_eq!(
        offset(5),
        (offset_of!(Vertex, joints) + size_of::<[u16; 4]>()) as u64
    );
    assert_eq!(
        offset(6),
        (offset_of!(Vertex, weights) + size_of::<[f32; 4]>()) as u64
    );
}

#[test]
#[ignore = "requires a working wgpu adapter; validates particles beside a spotlight shadow pass"]
fn particles_are_drawn_in_the_color_pass_when_a_shadow_caster_is_active() {
    let Some(mut harness) = harness() else {
        eprintln!("skipping particle color-pass regression test: no wgpu adapter available");
        return;
    };

    let eye = Transform3D::looking_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, Vec3::Y);
    let view = Mat4::from_rotation_translation(eye.rotation, eye.translation).inverse();
    let sun = SunLight::default();
    // This specifically exercises the shadow-caster branch that previously attempted to
    // bind the color-target particle pipeline inside a depth-only spotlight pass.
    let spotlight = SpotLight::default();
    let particles = [ParticleBatch {
        blend: ParticleBlendMode::Alpha,
        shape: ParticleShape::Square,
        billboard: ParticleBillboard::Camera,
        sort_back_to_front: false,
        instances: vec![ParticleInstance {
            position: [0.0, 0.0, 0.0],
            size: [1.5, 1.5],
            color: [1.0, 0.0, 0.0, 1.0],
            ..ParticleInstance::default()
        }],
    }];

    clear_view(
        harness.context.device(),
        harness.context.queue(),
        harness.context.target().view(),
        [0.0, 0.0, 0.0, 1.0],
    );
    harness.renderer.render(
        harness.context.device(),
        harness.context.queue(),
        harness.context.target().view(),
        (SIZE, SIZE),
        &Scene {
            view_proj: Camera3D::default().projection(1.0) * view,
            camera_position: eye.translation,
            camera_rotation: glam::Quat::IDENTITY,
            sun: &sun,
            spot_lights: std::slice::from_ref(&spotlight),
            point_lights: &[],
            fog: FogSettings::default(),
            draws: &[],
            particles: &particles,
        },
    );

    let pixels = harness
        .context
        .readback()
        .expect("particle readback")
        .pixels;
    assert!(
        pixels.iter().any(|pixel| {
            let red = (pixel >> 16) & 0xff;
            let green = (pixel >> 8) & 0xff;
            let blue = pixel & 0xff;
            red > 32 && red > green.saturating_add(16) && red > blue.saturating_add(16)
        }),
        "the alpha particle should contribute visible red pixels to the color target"
    );
}

#[test]
#[ignore = "requires a working wgpu adapter; validates GPU pipeline creation"]
fn renderer_creates_alpha_and_double_sided_pipeline_variants_when_gpu_is_available() {
    // Pipeline creation performs wgpu's bind-group/layout validation. The separate ignored
    // readback test below validates the visible MASK/BLEND/double-sided results.
    let Some(_harness) = harness() else {
        eprintln!("skipping alpha pipeline creation test: no wgpu adapter available");
        return;
    };
}

#[test]
fn lighting_post_and_shadow_wgsl_shaders_validate_without_a_gpu_adapter() {
    let shaders = [
        include_str!("../../shaders/mesh.wgsl"),
        include_str!("../../shaders/shadow.wgsl"),
        include_str!("../../shaders/bloom.wgsl"),
        include_str!("../../shaders/tonemap.wgsl"),
        include_str!("../../shaders/tonemap_fxaa.wgsl"),
        include_str!("../../shaders/tonemap_unorm.wgsl"),
        include_str!("../../shaders/tonemap_post.wgsl"),
        include_str!("../../shaders/auto_exposure.wgsl"),
        include_str!("../../shaders/gtao.wgsl"),
        include_str!("../../shaders/gtao_msaa.wgsl"),
        include_str!("../../shaders/particles.wgsl"),
    ];
    for (index, source) in shaders.iter().enumerate() {
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|error| panic!("WGSL shader {index} failed to parse: {error:?}"));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("WGSL shader {index} failed validation: {error:?}"));
    }
}

#[test]
#[ignore = "requires a working wgpu adapter; run tools/test_skeletal_skinning_gpu.ps1"]
fn skinned_vertex_palette_moves_geometry_in_the_color_pass_on_gpu() {
    use crate::mesh::Vertex;

    let mut harness = harness().expect(
        "skeletal skinning GPU test requires a working wgpu adapter; do not treat adapter absence as a pass",
    );
    let vertices = [
        Vertex {
            position: [-0.45, -0.35, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
            joints: [0, 1, 0, 0, 0, 0, 0, 0],
            weights: [0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        },
        Vertex {
            position: [0.45, -0.35, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [1.0, 0.0],
            joints: [0, 1, 0, 0, 0, 0, 0, 0],
            weights: [0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        },
        Vertex {
            position: [0.0, 0.45, 0.0],
            normal: [0.0, 0.0, 1.0],
            uv: [0.5, 1.0],
            joints: [0, 1, 0, 0, 0, 0, 0, 0],
            weights: [0.5, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        },
    ];
    let mesh = harness
        .renderer
        .upload_mesh(harness.context.device(), &vertices, &[0, 1, 2]);
    clear_view(
        harness.context.device(),
        harness.context.queue(),
        harness.context.target().view(),
        [0.0, 0.0, 0.0, 1.0],
    );
    let eye = Transform3D::looking_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, Vec3::Y);
    let view = Mat4::from_rotation_translation(eye.rotation, eye.translation).inverse();
    let scene = Scene {
        view_proj: Camera3D::default().projection(1.0) * view,
        camera_position: eye.translation,
        camera_rotation: glam::Quat::IDENTITY,
        sun: &SunLight::default(),
        spot_lights: &[],
        point_lights: &[],
        fog: FogSettings::default(),
        draws: &[],
        particles: &[],
    };
    let skin = Arc::<[Mat4]>::from(vec![
        Mat4::from_translation(Vec3::new(0.4, 0.0, 0.0)),
        Mat4::from_translation(Vec3::new(1.2, 0.0, 0.0)),
    ]);
    harness.renderer.render_with_skinning(
        harness.context.device(),
        harness.context.queue(),
        harness.context.target().view(),
        (SIZE, SIZE),
        &scene,
        &[SkinnedDraw {
            draw: Draw {
                mesh,
                material: MeshRenderer::WHITE,
                model: Mat4::IDENTITY,
                tint: [1.0; 3],
            },
            joint_matrices: skin,
        }],
    );
    let pixels = harness.context.readback().expect("readback").pixels;
    let mut lit_count = 0_u32;
    let mut x_sum = 0_u64;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (r, g, b) = pixel(&pixels, x, y);
            if r + g + b > 45 {
                lit_count += 1;
                x_sum += u64::from(x);
            }
        }
    }
    assert!(
        lit_count > 8,
        "skinned triangle should produce visible pixels"
    );
    let centroid_x = x_sum as f32 / lit_count as f32;
    assert!(
        centroid_x > (SIZE as f32 * 0.58),
        "palette translation was not applied: centroid x={centroid_x}"
    );
}

#[test]
fn joint_palette_capacity_rejects_overflow_without_corrupting_previous_offsets() {
    use super::draw::append_joint_palette;

    let palette = vec![Mat4::IDENTITY; super::MAX_SKIN_MATRICES_PER_FRAME];
    let mut destination = Vec::new();
    assert_eq!(append_joint_palette(&palette, &mut destination), Some(0));
    assert_eq!(destination.len(), super::MAX_SKIN_MATRICES_PER_FRAME);
    let before = destination.len();
    assert_eq!(
        append_joint_palette(&[Mat4::IDENTITY], &mut destination),
        None
    );
    assert_eq!(destination.len(), before);
    assert_eq!(append_joint_palette(&[], &mut destination), None);
    assert_eq!(destination.len(), before);
}

#[test]
fn spotlight_selection_prefers_the_shadow_casting_light() {
    let non_shadow = SpotLight {
        position: Vec3::new(1.0, 2.0, 3.0),
        casts_shadows: false,
        ..SpotLight::default()
    };
    let shadow = SpotLight {
        position: Vec3::new(4.0, 5.0, 6.0),
        casts_shadows: true,
        ..SpotLight::default()
    };
    let (active, caster) = select_spot_lights(&[non_shadow, shadow]);
    assert_eq!(active, Some(shadow));
    assert_eq!(caster, Some(shadow));
    let (active, caster) = select_spot_lights(&[non_shadow]);
    assert_eq!(active, Some(non_shadow));
    assert_eq!(caster, None);
}

#[test]
fn point_light_selection_ranks_by_distance_range_intensity_and_color_contribution() {
    let near_bright = PointLight {
        position: Vec3::new(2.0, 0.0, 0.0),
        color: Vec3::ONE,
        intensity: 10.0,
        range: 10.0,
    };
    let far_long_range = PointLight {
        position: Vec3::new(50.0, 0.0, 0.0),
        color: Vec3::ONE,
        intensity: 10.0,
        range: 50.0,
    };
    let near_dim = PointLight {
        position: Vec3::new(2.0, 0.0, 0.0),
        color: Vec3::splat(0.05),
        intensity: 1.0,
        range: 10.0,
    };
    let selected = select_point_lights(&[near_dim, far_long_range, near_bright], Vec3::ZERO);
    assert_eq!(selected, vec![near_bright, far_long_range, near_dim]);

    let moved = select_point_lights(
        &[near_dim, far_long_range, near_bright],
        Vec3::new(50.0, 0.0, 0.0),
    );
    assert_eq!(
        moved[0], far_long_range,
        "camera movement should affect ranking"
    );
}

#[test]
fn equal_importance_point_lights_use_stable_value_tie_breaking() {
    let left = PointLight {
        position: Vec3::new(-3.0, 0.0, 0.0),
        ..PointLight::default()
    };
    let right = PointLight {
        position: Vec3::new(3.0, 0.0, 0.0),
        ..PointLight::default()
    };
    let a = select_point_lights(&[right, left], Vec3::ZERO);
    let b = select_point_lights(&[left, right], Vec3::ZERO);
    assert_eq!(a, b);
    assert_eq!(a, vec![left, right]);
}

#[test]
fn point_light_selection_is_stable_under_insertion_order_and_caps_at_shader_capacity() {
    let lights: Vec<_> = (0..12)
        .map(|index| PointLight {
            position: Vec3::new(index as f32 - 5.0, 0.0, -2.0),
            color: Vec3::new(0.5 + index as f32 * 0.01, 0.8, 1.0),
            intensity: 4.0,
            range: 16.0,
        })
        .collect();
    let forward = select_point_lights(&lights, Vec3::ZERO);
    let reverse_input: Vec<_> = lights.iter().copied().rev().collect();
    let reverse = select_point_lights(&reverse_input, Vec3::ZERO);
    assert_eq!(forward, reverse);
    assert_eq!(forward.len(), super::MAX_POINT_LIGHTS);

    let invalid = PointLight {
        position: Vec3::splat(f32::NAN),
        ..PointLight::default()
    };
    let selected = select_point_lights(&[invalid, PointLight::default()], Vec3::ZERO);
    assert_eq!(selected, vec![PointLight::default()]);
}

#[test]
fn point_light_packing_caps_at_eight_and_clamps_negative_ranges() {
    let lights: Vec<_> = (0..10)
        .map(|index| PointLight {
            position: Vec3::new(index as f32, 2.0, 3.0),
            color: Vec3::new(0.1 * index as f32, 0.25, 0.75),
            intensity: if index == 3 { -4.0 } else { index as f32 },
            range: if index == 4 {
                -2.0
            } else {
                10.0 + index as f32
            },
        })
        .collect();
    let (positions, colors) = pack_point_lights(&lights);
    assert_eq!(positions.len(), 8);
    assert_eq!(positions[0], [0.0, 2.0, 3.0, 10.0]);
    assert_eq!(positions[4][3], 0.0);
    assert_eq!(positions[7][0], 7.0);
    assert_eq!(colors[3][3], 0.0);
    assert_eq!(colors[7][3], 7.0);
}

#[test]
fn frustum_rejects_objects_behind_the_camera_and_keeps_visible_objects() {
    let projection = Camera3D::default().projection(1.0);
    let view = Mat4::IDENTITY;
    let planes = super::frustum_planes(projection * view);
    assert!(super::sphere_visible(
        &planes,
        Vec3::new(0.0, 0.0, -3.0),
        0.5
    ));
    assert!(!super::sphere_visible(
        &planes,
        Vec3::new(0.0, 0.0, 3.0),
        0.5
    ));
}

#[test]
#[ignore = "requires a working wgpu adapter; validates rendered pixels"]
fn cube_is_drawn_in_the_center_and_background_is_untouched() {
    let Some(mut h) = harness() else { return };
    let pixels = h.draw(&[cube(Mat4::IDENTITY, [1.0, 0.0, 0.0])]);
    let (r, g, b) = pixel(&pixels, SIZE / 2, SIZE / 2);
    // PBR specular highlights can contribute a small amount to channels that
    // are zero in the base color, so test red dominance rather than exact RGB.
    assert!(
        r > 0 && r > 4 * g && r > 4 * b,
        "expected a red-dominant center pixel, got {r},{g},{b}"
    );
    assert_eq!(pixel(&pixels, 0, 0), (0, 0, 0));
}

#[test]
#[ignore = "requires a working wgpu adapter; validates the main depth pass"]
fn depth_test_keeps_the_nearer_cube_in_front() {
    let Some(mut h) = harness() else { return };
    let far = Mat4::from_translation(Vec3::new(0.0, 0.0, -3.0));
    // The far cube is submitted last; without a depth test it would overwrite the red one.
    let pixels = h.draw(&[
        cube(Mat4::IDENTITY, [1.0, 0.0, 0.0]),
        cube(far, [0.0, 0.0, 1.0]),
    ]);
    let (r, _, b) = pixel(&pixels, SIZE / 2, SIZE / 2);
    assert!(
        r > 0 && r > 4 * b,
        "expected the near red cube to dominate, got r={r} b={b}"
    );
}

#[test]
#[ignore = "requires a working wgpu adapter; validates material texture output"]
fn base_color_texture_shows_up_on_the_surface() {
    let Some(mut h) = harness() else { return };
    // 2x2 checker: red, green / blue, white.
    let texels: [u8; 16] = [
        255, 0, 0, 255, 0, 255, 0, 255, //
        0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let material = h.renderer.upload_material(
        h.context.device(),
        h.context.queue(),
        [1.0; 4],
        Some(TextureData {
            width: 2,
            height: 2,
            rgba8: &texels,
        }),
    );
    let pixels = h.draw(&[Draw {
        mesh: MeshRenderer::CUBE,
        material,
        model: Mat4::IDENTITY,
        tint: [1.0; 3],
    }]);
    let mut colors = std::collections::HashSet::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = pixel(&pixels, x, y);
            if p != (0, 0, 0) {
                colors.insert(p);
            }
        }
    }
    assert!(
        colors.len() >= 3,
        "texture should produce several colors, saw {}",
        colors.len()
    );
}

#[test]
#[ignore = "requires a working wgpu adapter; writes and checks a rendered PNG"]
fn spotlight_only_corridor_writes_a_headless_png() {
    const WIDTH: u32 = 160;
    const HEIGHT: u32 = 90;
    let context = match pollster::block_on(GpuContext::new_offscreen(WIDTH, HEIGHT)) {
        Ok(context) => context,
        Err(error) => {
            eprintln!(
                "skipping spotlight screenshot test: no usable GPU adapter/device: {}",
                error.0
            );
            return;
        }
    };
    let (device, queue) = (context.device(), context.queue());
    let mut renderer = MeshRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
    let cube_model = |position: Vec3, scale: Vec3| {
        Mat4::from_scale_rotation_translation(scale, glam::Quat::IDENTITY, position)
    };
    let draws = [
        cube(
            cube_model(Vec3::new(0.0, -0.12, -0.6), Vec3::new(8.0, 0.24, 14.0)),
            [0.66, 0.69, 0.65],
        ),
        cube(
            cube_model(Vec3::new(-3.9, 1.5, -0.6), Vec3::new(0.2, 3.0, 14.0)),
            [0.34, 0.40, 0.35],
        ),
        cube(
            cube_model(Vec3::new(3.9, 1.5, -0.6), Vec3::new(0.2, 3.0, 14.0)),
            [0.34, 0.40, 0.35],
        ),
        cube(
            cube_model(Vec3::new(0.0, 1.5, -7.3), Vec3::new(8.0, 3.0, 0.2)),
            [0.30, 0.35, 0.31],
        ),
        cube(
            // Move the occluder off the camera centerline so its floor shadow
            // remains visible instead of being hidden behind the blocking cube.
            cube_model(Vec3::new(-1.2, 0.75, -2.0), Vec3::new(1.1, 1.5, 0.9)),
            [0.08, 0.10, 0.08],
        ),
    ];
    let camera = Transform3D::looking_at(
        Vec3::new(0.0, 2.05, 4.2),
        Vec3::new(0.0, 0.65, -2.3),
        Vec3::Y,
    );
    let view = Mat4::from_rotation_translation(camera.rotation, camera.translation).inverse();
    let spotlight = SpotLight {
        // Place the test light close enough to the central blocker that its
        // floor shadow receives enough linear-light energy to survive RGBA8
        // quantization in this headless regression target.
        position: Vec3::new(0.0, 2.0, 1.5),
        // Aim the cone through the off-center blocker so its floor shadow is
        // illuminated strongly enough to be measured by the regression test.
        direction: Vec3::new(-0.31, -0.32, -0.91).normalize(),
        color: Vec3::new(0.82, 0.93, 0.80),
        // Keep the offscreen test signal above RGBA8 quantization and cover
        // the visible floor-shadow region on the side of the occluder.
        intensity: 320.0,
        range: 13.0,
        inner_cone_angle: 16.0_f32.to_radians(),
        outer_cone_angle: 32.0_f32.to_radians(),
        casts_shadows: true,
    };
    let render_corridor = |renderer: &mut MeshRenderer, spot: SpotLight| {
        clear_view(
            device,
            queue,
            context.target().view(),
            [0.002, 0.003, 0.003, 1.0],
        );
        renderer.render(
            device,
            queue,
            context.target().view(),
            (WIDTH, HEIGHT),
            &Scene {
                view_proj: Camera3D {
                    fov_y: 70.0_f32.to_radians(),
                    near: 0.05,
                    far: 24.0,
                }
                .projection(WIDTH as f32 / HEIGHT as f32)
                    * view,
                camera_position: camera.translation,
                camera_rotation: glam::Quat::IDENTITY,
                sun: &SunLight {
                    direction: Vec3::NEG_Y,
                    color: Vec3::ZERO,
                    ambient: Vec3::splat(0.0005),
                },
                spot_lights: std::slice::from_ref(&spot),
                point_lights: &[],
                fog: FogSettings {
                    color: Vec3::new(0.015, 0.023, 0.020),
                    density: 0.035,
                    base_height: 0.0,
                    height_falloff: 0.08,
                },
                draws: &draws,
                particles: &[],
            },
        );
        context
            .readback()
            .expect("read back spotlight corridor")
            .pixels
    };
    let framebuffer_pixels = render_corridor(&mut renderer, spotlight);
    // A second frame with the exact same light and geometry but disabled shadows
    // makes this a regression check for the shadow map, not merely a dark-corner check.
    let unshadowed_pixels = render_corridor(
        &mut renderer,
        SpotLight {
            casts_shadows: false,
            ..spotlight
        },
    );
    let mut shadow_changed_pixels = 0usize;
    let mut shadow_difference_energy = 0_u64;
    for (shadowed, unshadowed) in framebuffer_pixels.iter().zip(&unshadowed_pixels) {
        let shadowed = *shadowed;
        let unshadowed = *unshadowed;
        let shadowed_rgb = (
            (shadowed >> 16) & 0xff,
            (shadowed >> 8) & 0xff,
            shadowed & 0xff,
        );
        let unshadowed_rgb = (
            (unshadowed >> 16) & 0xff,
            (unshadowed >> 8) & 0xff,
            unshadowed & 0xff,
        );
        let difference = shadowed_rgb.0.abs_diff(unshadowed_rgb.0)
            + shadowed_rgb.1.abs_diff(unshadowed_rgb.1)
            + shadowed_rgb.2.abs_diff(unshadowed_rgb.2);
        shadow_difference_energy += u64::from(difference);
        if difference > 12 {
            shadow_changed_pixels += 1;
        }
    }
    let mut bright_pixels = 0usize;
    let mut dark_pixels = 0usize;
    let mut rgba = Vec::with_capacity(framebuffer_pixels.len() * 4);
    for packed_pixel in framebuffer_pixels {
        let r = ((packed_pixel >> 16) & 0xff) as u8;
        let g = ((packed_pixel >> 8) & 0xff) as u8;
        let b = (packed_pixel & 0xff) as u8;
        let luminance = (u32::from(r) * 3 + u32::from(g) * 6 + u32::from(b)) / 10;
        if luminance > 24 {
            bright_pixels += 1;
        }
        if luminance < 6 {
            dark_pixels += 1;
        }
        rgba.extend_from_slice(&[r, g, b, 255]);
    }
    let output =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/spotlight_corridor.png");
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).expect("create target directory for screenshot");
    }
    let file = File::create(&output).expect("create spotlight screenshot PNG");
    let mut encoder = png::Encoder::new(file, WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("write PNG header");
    writer
        .write_image_data(&rgba)
        .expect("write screenshot image data");
    eprintln!("spotlight corridor screenshot: {}", output.display());

    // Persist the screenshot before asserting so a failing GPU regression still leaves
    // an inspectable artifact in target/ on the developer machine.
    assert!(
            shadow_changed_pixels > 16 && shadow_difference_energy > 800,
            "spotlight shadow map should darken a measurable corridor region; changed pixels={shadow_changed_pixels}, difference energy={shadow_difference_energy}; inspect {}",
            output.display()
        );
    assert!(
            bright_pixels > 80,
            "spotlight-only corridor should contain a lit region; bright pixels={bright_pixels}; inspect {}",
            output.display()
        );
    assert!(
        dark_pixels > 80,
        "spotlight-only corridor should retain deep shadow; dark pixels={dark_pixels}; inspect {}",
        output.display()
    );
}

#[test]
fn mip_chain_halves_down_to_one_pixel_and_averages_in_linear_light() {
    let texels: [u8; 16] = [
        0, 0, 0, 255, 255, 255, 255, 255, //
        255, 255, 255, 255, 0, 0, 0, 255,
    ];
    let mips = build_mips(2, 2, &texels);
    assert_eq!(mips.len(), 2);
    assert_eq!((mips[1].0, mips[1].1), (1, 1));
    // Half black, half white is 0.5 linear, which encodes to about 188 in sRGB.
    assert!((187..=188).contains(&mips[1].2[0]), "{}", mips[1].2[0]);
    let odd = build_mips(5, 3, &[255; 5 * 3 * 4]);
    let sizes: Vec<_> = odd.iter().map(|(w, h, _)| (*w, *h)).collect();
    assert_eq!(sizes, vec![(5, 3), (2, 1), (1, 1)]);
}

#[test]
#[ignore = "requires a working wgpu adapter; validates real alpha discard and blending"]
fn gltf_mask_discards_texture_alpha_and_blend_composites_back_to_front() {
    let mut harness = harness().expect(
        "glTF alpha GPU regression requires a working wgpu adapter; do not treat adapter absence as a pass",
    );
    // Texture alpha alone is just above the threshold; multiplying by the factor alpha
    // drops coverage below the MASK cutoff and must reveal the opaque object behind it.
    let transparent_texel = [255_u8, 255, 255, 128];
    let mask_material = harness.renderer.upload_pbr_material_with_options(
        harness.context.device(),
        harness.context.queue(),
        PbrMaterialDesc {
            base_color: [1.0, 1.0, 1.0, 0.5],
            metallic: 0.0,
            roughness: 1.0,
            base_color_texture: Some(TextureData {
                width: 1,
                height: 1,
                rgba8: &transparent_texel,
            }),
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            occlusion_strength: 1.0,
            emissive_texture: None,
            emissive_factor: [0.0; 3],
        },
        PbrMaterialRenderOptions {
            alpha_mode: AlphaMode::Mask,
            alpha_cutoff: 0.5,
            double_sided: false,
        },
    );
    clear_view(
        harness.context.device(),
        harness.context.queue(),
        harness.context.target().view(),
        [0.0, 0.0, 0.0, 1.0],
    );
    let mask_pixels = harness.draw(&[
        cube(
            Mat4::from_translation(Vec3::new(0.0, 0.0, -0.55)),
            [0.0, 0.0, 1.0],
        ),
        Draw {
            mesh: MeshRenderer::CUBE,
            material: mask_material,
            model: Mat4::from_translation(Vec3::new(0.0, 0.0, 0.45)),
            tint: [1.0; 3],
        },
    ]);
    let (mask_r, _, mask_b) = pixel(&mask_pixels, SIZE / 2, SIZE / 2);
    assert!(
        mask_b > mask_r.saturating_mul(2),
        "MASK must discard the transparent texel and reveal the blue surface behind it; got R={mask_r}, B={mask_b}"
    );

    // OPAQUE must ignore even a deliberately tiny base-color factor alpha and fully cover
    // the opaque blue object behind it.
    let opaque_material = harness.renderer.upload_pbr_material_with_options(
        harness.context.device(),
        harness.context.queue(),
        PbrMaterialDesc {
            base_color: [1.0, 0.0, 0.0, 0.01],
            metallic: 0.0,
            roughness: 1.0,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            occlusion_strength: 1.0,
            emissive_texture: None,
            emissive_factor: [0.0; 3],
        },
        PbrMaterialRenderOptions {
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
        },
    );
    clear_view(
        harness.context.device(),
        harness.context.queue(),
        harness.context.target().view(),
        [0.0, 0.0, 0.0, 1.0],
    );
    let opaque_pixels = harness.draw(&[
        cube(
            Mat4::from_translation(Vec3::new(0.0, 0.0, -0.55)),
            [0.0, 0.0, 1.0],
        ),
        Draw {
            mesh: MeshRenderer::CUBE,
            material: opaque_material,
            model: Mat4::from_translation(Vec3::new(0.0, 0.0, 0.45)),
            tint: [1.0; 3],
        },
    ]);
    let (opaque_r, _, opaque_b) = pixel(&opaque_pixels, SIZE / 2, SIZE / 2);
    assert!(
        opaque_r > opaque_b.saturating_mul(2),
        "OPAQUE must ignore base-color alpha and cover the blue surface behind it; got R={opaque_r}, B={opaque_b}"
    );

    let blend_material = harness.renderer.upload_pbr_material_with_options(
        harness.context.device(),
        harness.context.queue(),
        PbrMaterialDesc {
            base_color: [1.0, 1.0, 1.0, 0.5],
            metallic: 0.0,
            roughness: 1.0,
            base_color_texture: Some(TextureData {
                width: 1,
                height: 1,
                rgba8: &transparent_texel,
            }),
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            occlusion_strength: 1.0,
            emissive_texture: None,
            emissive_factor: [0.0; 3],
        },
        PbrMaterialRenderOptions {
            alpha_mode: AlphaMode::Blend,
            alpha_cutoff: 0.5,
            double_sided: false,
        },
    );
    clear_view(
        harness.context.device(),
        harness.context.queue(),
        harness.context.target().view(),
        [0.0, 0.0, 0.0, 1.0],
    );
    // Deliberately submit near-to-far transparent objects before the opaque background.
    // Correct sorting must draw blue opaque, then green transparent, then red transparent.
    let blend_pixels = harness.draw(&[
        Draw {
            mesh: MeshRenderer::CUBE,
            material: blend_material,
            model: Mat4::from_translation(Vec3::new(0.0, 0.0, 0.45)),
            tint: [1.0, 0.0, 0.0],
        },
        Draw {
            mesh: MeshRenderer::CUBE,
            material: blend_material,
            model: Mat4::from_translation(Vec3::new(0.0, 0.0, 0.0)),
            tint: [0.0, 1.0, 0.0],
        },
        cube(
            Mat4::from_translation(Vec3::new(0.0, 0.0, -0.55)),
            [0.0, 0.0, 1.0],
        ),
    ]);
    let (blend_r, blend_g, blend_b) = pixel(&blend_pixels, SIZE / 2, SIZE / 2);
    assert!(
        blend_r > blend_g && blend_g > 8 && blend_b > 8,
        "BLEND must render opaque blue, then transparent green, then transparent red; got R={blend_r}, G={blend_g}, B={blend_b}"
    );

    // Reverse every triangle winding: the camera sees the cube's back faces. A
    // double-sided material must render those faces without disabling culling for
    // the renderer's ordinary material pipelines.
    let mut reversed = crate::mesh::cube();
    let (triangles, trailing_indices) = reversed.indices.as_chunks_mut::<3>();
    assert!(
        trailing_indices.is_empty(),
        "cube mesh indices must contain complete triangles"
    );
    for triangle in triangles {
        triangle.swap(1, 2);
    }
    let reversed_mesh = harness.renderer.upload_mesh(
        harness.context.device(),
        &reversed.vertices,
        &reversed.indices,
    );
    let double_sided_material = harness.renderer.upload_pbr_material_with_options(
        harness.context.device(),
        harness.context.queue(),
        PbrMaterialDesc {
            base_color: [0.0, 1.0, 0.0, 1.0],
            metallic: 0.0,
            roughness: 1.0,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            occlusion_strength: 1.0,
            emissive_texture: None,
            emissive_factor: [0.0; 3],
        },
        PbrMaterialRenderOptions {
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: true,
        },
    );
    clear_view(
        harness.context.device(),
        harness.context.queue(),
        harness.context.target().view(),
        [0.0, 0.0, 0.0, 1.0],
    );
    let double_sided_pixels = harness.draw(&[Draw {
        mesh: reversed_mesh,
        material: double_sided_material,
        model: Mat4::IDENTITY,
        tint: [1.0; 3],
    }]);
    let (double_r, double_g, double_b) = pixel(&double_sided_pixels, SIZE / 2, SIZE / 2);
    assert!(
        double_g > double_r && double_g > double_b,
        "double-sided material must render reversed winding; got R={double_r}, G={double_g}, B={double_b}"
    );
}

#[test]
#[ignore = "requires a real wgpu adapter; validates the single-sample GTAO depth path"]
fn gtao_single_sample_depth_compute_runs_on_gpu() {
    use crate::post::{
        AmbientOcclusionSettings, HdrTarget, PostProcessRenderOptions, PostProcessingSettings,
        ToneMapper, HDR_FORMAT,
    };

    let context = pollster::block_on(GpuContext::new_offscreen(SIZE, SIZE))
        .expect("GTAO GPU test requires a working wgpu adapter");
    let depth_features = context
        .adapter()
        .get_texture_format_features(crate::renderer::DEPTH_FORMAT);
    assert!(
        depth_features.allowed_usages.contains(
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING
        ),
        "adapter depth format cannot be sampled by the GTAO compute path"
    );
    let hdr_target = HdrTarget::new(context.device(), (SIZE, SIZE));
    let mut renderer =
        MeshRenderer::new_with_sample_count(context.device(), context.queue(), HDR_FORMAT, 1);
    let eye = Transform3D::looking_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, Vec3::Y);
    let view = Mat4::from_rotation_translation(eye.rotation, eye.translation).inverse();
    let camera = Camera3D::default();
    let sun = SunLight::default();
    let scene = Scene {
        view_proj: camera.projection(1.0) * view,
        camera_position: eye.translation,
        camera_rotation: glam::Quat::IDENTITY,
        sun: &sun,
        spot_lights: &[],
        point_lights: &[],
        fog: FogSettings::default(),
        draws: &[cube(Mat4::IDENTITY, [0.9, 0.65, 0.3])],
        particles: &[],
    };
    renderer.render(
        context.device(),
        context.queue(),
        &hdr_target.view,
        (SIZE, SIZE),
        &scene,
    );
    let ao_frame = crate::post::AmbientOcclusionFrame {
        depth_view: renderer
            .depth_view()
            .expect("single-sample depth target should exist"),
        inverse_view_projection: scene.view_proj.inverse().to_cols_array_2d(),
        camera_position: [eye.translation.x, eye.translation.y, eye.translation.z],
        fov_y_radians: camera.fov_y,
        sample_count: 1,
    };
    let mut tonemapper = ToneMapper::new(
        context.device(),
        &hdr_target.view,
        wgpu::TextureFormat::Rgba8Unorm,
        (SIZE, SIZE),
    );
    let post_settings = PostProcessingSettings::default();
    let ao_settings = AmbientOcclusionSettings::cinematic();
    tonemapper.render_with_options(
        context.device(),
        context.queue(),
        context.target().view(),
        PostProcessRenderOptions {
            settings: &post_settings,
            fxaa: false,
            ao_settings: &ao_settings,
            ao_frame: Some(ao_frame),
            profile_cpu: false,
            submitted_frame: 1,
        },
    );
    let pixels = context.readback().expect("GTAO output readback").pixels;
    assert!(
        pixels.iter().any(|pixel| {
            ((pixel >> 16) & 0xff) > 1 || ((pixel >> 8) & 0xff) > 1 || (pixel & 0xff) > 1
        }),
        "single-sample GTAO plus tone mapping produced an all-black image"
    );
}

#[test]
#[ignore = "requires a real GPU adapter with an MSAA resolve-capable HDR/depth format pair"]
fn multisampled_hdr_resolves_then_fxaa_tonemaps_on_gpu() {
    use crate::post::{HdrTarget, ToneMapper, HDR_FORMAT};

    let context = pollster::block_on(GpuContext::new_offscreen(SIZE, SIZE))
        .expect("MSAA/FXAA GPU test requires a working wgpu adapter");
    let color_features = context.adapter().get_texture_format_features(HDR_FORMAT);
    let depth_features = context
        .adapter()
        .get_texture_format_features(crate::renderer::DEPTH_FORMAT);
    let supported: Vec<u32> = [1, 2, 4, 8]
        .into_iter()
        .filter(|count| {
            *count == 1
                || (color_features.allowed_usages.contains(
                    wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                ) && color_features
                    .flags
                    .contains(wgpu::TextureFormatFeatureFlags::FILTERABLE)
                    && color_features
                        .flags
                        .contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
                    && color_features.flags.sample_count_supported(*count)
                    && depth_features.allowed_usages.contains(
                        wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING,
                    )
                    && depth_features.flags.sample_count_supported(*count))
        })
        .collect();
    assert!(
        supported.contains(&4),
        "test adapter does not expose a usable 4x HDR/depth MSAA pair: {supported:?}"
    );
    let sample_count = 4;

    let msaa_texture = context.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("MSAA regression HDR color target"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format: HDR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let msaa_view = msaa_texture.create_view(&wgpu::TextureViewDescriptor::default());
    clear_view(
        context.device(),
        context.queue(),
        &msaa_view,
        [0.0, 0.0, 0.0, 1.0],
    );

    let hdr_target = HdrTarget::new(context.device(), (SIZE, SIZE));
    let mut renderer = MeshRenderer::new_with_sample_count(
        context.device(),
        context.queue(),
        HDR_FORMAT,
        sample_count,
    );
    assert_eq!(renderer.sample_count(), sample_count);
    let eye = Transform3D::looking_at(Vec3::new(0.0, 0.0, 4.0), Vec3::ZERO, Vec3::Y);
    let view = Mat4::from_rotation_translation(eye.rotation, eye.translation).inverse();
    let sun = SunLight::default();
    let particles = [ParticleBatch {
        blend: ParticleBlendMode::Alpha,
        shape: ParticleShape::Square,
        billboard: ParticleBillboard::Camera,
        sort_back_to_front: false,
        instances: vec![ParticleInstance {
            position: [0.0, 0.0, 0.0],
            size: [1.5, 1.5],
            color: [1.0, 0.0, 0.0, 1.0],
            ..ParticleInstance::default()
        }],
    }];
    // Keep the original mesh-rendering coverage in this MSAA regression test;
    // the particle draw is additive coverage, not a replacement for the cube.
    let draws = [cube(
        Mat4::from_translation(Vec3::new(1.2, 0.0, 0.0)),
        [0.9, 0.65, 0.3],
    )];
    let scene = Scene {
        view_proj: Camera3D::default().projection(1.0) * view,
        camera_position: eye.translation,
        camera_rotation: glam::Quat::IDENTITY,
        sun: &sun,
        spot_lights: &[],
        point_lights: &[],
        fog: FogSettings::default(),
        draws: &draws,
        particles: &particles,
    };
    renderer.render_with_skinning_resolve(
        RenderTarget {
            device: context.device(),
            queue: context.queue(),
            view: &msaa_view,
            resolve_view: Some(&hdr_target.view),
            size: (SIZE, SIZE),
            scene: &scene,
        },
        &[],
    );

    let mut tonemapper = ToneMapper::new_with_fxaa(
        context.device(),
        &hdr_target.view,
        wgpu::TextureFormat::Rgba8Unorm,
        (SIZE, SIZE),
        true,
    );
    let ao_frame = crate::post::AmbientOcclusionFrame {
        depth_view: renderer
            .depth_view()
            .expect("main scene depth target should exist after the MSAA mesh pass"),
        inverse_view_projection: scene.view_proj.inverse().to_cols_array_2d(),
        camera_position: [eye.translation.x, eye.translation.y, eye.translation.z],
        fov_y_radians: Camera3D::default().fov_y,
        sample_count,
    };
    let post_settings = crate::post::PostProcessingSettings::cinematic();
    let ao_settings = crate::post::AmbientOcclusionSettings::cinematic();
    let phases = tonemapper.render_with_options(
        context.device(),
        context.queue(),
        context.target().view(),
        crate::post::PostProcessRenderOptions {
            settings: &post_settings,
            fxaa: true,
            ao_settings: &ao_settings,
            ao_frame: Some(ao_frame),
            profile_cpu: false,
            submitted_frame: 1,
        },
    );
    assert!(phases.is_empty());
    let pixels = context
        .readback()
        .expect("resolved MSAA/FXAA/post-process readback")
        .pixels;
    assert!(
        pixels.iter().any(|pixel| {
            let red = (pixel >> 16) & 0xff;
            let green = (pixel >> 8) & 0xff;
            let blue = pixel & 0xff;
            (red > 1) || (green > 1) || (blue > 1)
        }),
        "MSAA resolve followed by FXAA and display effects produced an all-black RGB image"
    );
    assert!(
        pixels.iter().any(|pixel| {
            let red = (pixel >> 16) & 0xff;
            let green = (pixel >> 8) & 0xff;
            let blue = pixel & 0xff;
            red > 24 && red > green.saturating_add(12) && red > blue.saturating_add(12)
        }),
        "the alpha particle should remain visible after the 4x MSAA resolve and display effects"
    );
    let mesh_region_has_color = (24..44).any(|y| {
        (42..62).any(|x| {
            let pixel = pixels[(y * SIZE + x) as usize];
            let red = (pixel >> 16) & 0xff;
            let green = (pixel >> 8) & 0xff;
            let blue = pixel & 0xff;
            red > 20 && green > 12 && red > blue.saturating_add(4)
        })
    });
    assert!(
        mesh_region_has_color,
        "the original offset mesh draw should remain visible in the same 4x MSAA scene"
    );
}
