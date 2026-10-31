use super::{
    build_mips, pack_point_lights, select_spot_lights, Draw, MeshRenderer, Scene, TextureData,
};
use crate::components::{Camera3D, FogSettings, PointLight, SpotLight, SunLight, Transform3D};
use glam::{Mat4, Vec3};
use ribbon_render::gpu::GpuContext;
use ribbon_render::gpu_window::clear_view;
use std::fs::File;
use std::path::PathBuf;

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
                sun: &SunLight::default(),
                spot_lights: &[],
                point_lights: &[],
                fog: FogSettings::default(),
                draws,
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
fn lighting_post_and_shadow_wgsl_shaders_validate_without_a_gpu_adapter() {
    let shaders = [
        include_str!("../../shaders/mesh.wgsl"),
        include_str!("../../shaders/shadow.wgsl"),
        include_str!("../../shaders/bloom.wgsl"),
        include_str!("../../shaders/tonemap.wgsl"),
        include_str!("../../shaders/tonemap_unorm.wgsl"),
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
