use super::shadows::directional_cascade_snap_debug;
use super::{
    build_mips, pack_point_lights, select_point_lights, select_spot_lights,
    DirectionalShadowCasterSettings, DirectionalShadowDebugMode, DirectionalShadowDebugOptions,
    Draw, MeshRenderer, PbrMaterialDesc, PbrMaterialRenderOptions, RenderTarget, Scene,
    SkinnedDraw, TextureData,
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

fn packed_rgb(pixels: &[u32], width: u32, x: u32, y: u32) -> [u8; 3] {
    let packed = pixels[(y * width + x) as usize];
    [
        ((packed >> 16) & 0xff) as u8,
        ((packed >> 8) & 0xff) as u8,
        (packed & 0xff) as u8,
    ]
}

/// Reconstruct the world-space point covered by a pixel on the horizontal receiver plane.
/// The transition sweep reads the center pixel, whose sample center is half a pixel away from
/// the mathematical screen center. With a downward-looking camera, that small offset changes
/// the ray/ground intersection depth enough to invalidate a target-point-only CPU comparison.
fn world_position_at_pixel_on_horizontal_plane(
    view_proj: Mat4,
    width: u32,
    height: u32,
    pixel_x: u32,
    pixel_y: u32,
    plane_y: f32,
) -> Vec3 {
    assert!(width > 0 && height > 0);
    assert!(pixel_x < width && pixel_y < height);

    let inverse = view_proj.inverse();
    assert!(inverse
        .to_cols_array()
        .iter()
        .all(|value| value.is_finite()));
    let ndc_x = ((pixel_x as f32 + 0.5) / width as f32) * 2.0 - 1.0;
    // Texture/readback row zero is the top row; NDC +Y points upward.
    let ndc_y = 1.0 - ((pixel_y as f32 + 0.5) / height as f32) * 2.0;
    let unproject = |depth: f32| {
        let homogeneous = inverse * glam::Vec4::new(ndc_x, ndc_y, depth, 1.0);
        assert!(homogeneous.is_finite() && homogeneous.w.abs() > 1.0e-7);
        homogeneous.truncate() / homogeneous.w
    };
    let ray_near = unproject(0.0);
    let ray_far = unproject(1.0);
    let ray = ray_far - ray_near;
    assert!(ray.is_finite() && ray.y.abs() > 1.0e-7);
    let distance = (plane_y - ray_near.y) / ray.y;
    assert!(
        distance.is_finite() && (0.0..=1.0).contains(&distance),
        "pixel ray does not intersect the receiver plane between the camera near/far planes"
    );
    let world_position = ray_near + ray * distance;
    assert!(world_position.is_finite());
    world_position
}

fn downsample_rgba(
    pixels: &[u32],
    width: u32,
    height: u32,
    out_width: u32,
    out_height: u32,
) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((out_width * out_height * 4) as usize);
    for y in 0..out_height {
        let source_y = ((u64::from(y) * u64::from(height)) / u64::from(out_height)) as u32;
        for x in 0..out_width {
            let source_x = ((u64::from(x) * u64::from(width)) / u64::from(out_width)) as u32;
            let [red, green, blue] = packed_rgb(pixels, width, source_x, source_y);
            rgba.extend_from_slice(&[red, green, blue, 255]);
        }
    }
    rgba
}

fn rgba_from_packed_pixels(pixels: &[u32]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for packed in pixels {
        rgba.extend_from_slice(&[
            ((packed >> 16) & 0xff) as u8,
            ((packed >> 8) & 0xff) as u8,
            (packed & 0xff) as u8,
            255,
        ]);
    }
    rgba
}

fn render_directional_visibility_frame(
    context: &GpuContext,
    renderer: &mut MeshRenderer,
    scene: &Scene<'_>,
    skinned_draws: &[SkinnedDraw],
    size: (u32, u32),
) -> Vec<u32> {
    renderer.set_directional_shadow_debug(DirectionalShadowDebugOptions {
        mode: DirectionalShadowDebugMode::ShadowVisibility,
        depth_layer: 0,
    });
    clear_view(
        context.device(),
        context.queue(),
        context.target().view(),
        [0.0, 0.0, 0.0, 1.0],
    );
    renderer.render_with_skinning(
        context.device(),
        context.queue(),
        context.target().view(),
        size,
        scene,
        skinned_draws,
    );
    context
        .readback()
        .expect("read CSM shadow-visibility frame")
        .pixels
}

fn write_rgba_png(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create diagnostic PNG directory");
    }
    assert_eq!(rgba.len(), (width * height * 4) as usize);
    let file = File::create(path).expect("create diagnostic PNG");
    let mut encoder = png::Encoder::new(file, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("write diagnostic PNG header");
    writer
        .write_image_data(rgba)
        .expect("write diagnostic PNG pixels");
}

fn write_contact_sheet(
    path: &std::path::Path,
    tiles: &[Vec<u8>],
    columns: u32,
    tile_width: u32,
    tile_height: u32,
) {
    let rows = tiles.len().div_ceil(columns as usize) as u32;
    let width = columns * tile_width;
    let height = rows * tile_height;
    let mut sheet = vec![0_u8; (width * height * 4) as usize];
    for (tile_index, tile) in tiles.iter().enumerate() {
        assert_eq!(tile.len(), (tile_width * tile_height * 4) as usize);
        let tile_x = (tile_index as u32 % columns) * tile_width;
        let tile_y = (tile_index as u32 / columns) * tile_height;
        for y in 0..tile_height {
            let source_start = (y * tile_width * 4) as usize;
            let source_end = source_start + (tile_width * 4) as usize;
            let destination_start = ((tile_y + y) * width * 4 + tile_x * 4) as usize;
            let destination_end = destination_start + (tile_width * 4) as usize;
            sheet[destination_start..destination_end]
                .copy_from_slice(&tile[source_start..source_end]);
        }
    }
    write_rgba_png(path, width, height, &sheet);
}

fn render_directional_debug_frame(
    context: &GpuContext,
    renderer: &mut MeshRenderer,
    scene: &Scene<'_>,
    skinned_draws: &[SkinnedDraw],
    size: (u32, u32),
    mode: DirectionalShadowDebugMode,
) -> Vec<u32> {
    renderer.set_directional_shadow_debug(DirectionalShadowDebugOptions {
        mode,
        depth_layer: 0,
    });
    clear_view(
        context.device(),
        context.queue(),
        context.target().view(),
        [0.0, 0.0, 0.0, 1.0],
    );
    renderer.render_with_skinning(
        context.device(),
        context.queue(),
        context.target().view(),
        size,
        scene,
        skinned_draws,
    );
    context
        .readback()
        .expect("read directional-shadow debug frame")
        .pixels
}

fn render_directional_depth_layer_frame(
    context: &GpuContext,
    renderer: &mut MeshRenderer,
    scene: &Scene<'_>,
    size: (u32, u32),
    layer: u32,
) -> Vec<u32> {
    renderer.set_directional_shadow_debug(DirectionalShadowDebugOptions {
        mode: DirectionalShadowDebugMode::DepthLayer,
        depth_layer: layer,
    });
    clear_view(
        context.device(),
        context.queue(),
        context.target().view(),
        [0.0, 0.0, 0.0, 1.0],
    );
    renderer.render(
        context.device(),
        context.queue(),
        context.target().view(),
        size,
        scene,
    );
    context
        .readback()
        .expect("read directional depth layer")
        .pixels
}

fn shadow_map_pixel_position(matrix: Mat4, world_position: Vec3) -> glam::Vec2 {
    let clip = matrix * world_position.extend(1.0);
    assert!(clip.is_finite() && clip.w.abs() > 1.0e-7);
    let ndc = clip.truncate() / clip.w;
    glam::Vec2::new(
        (ndc.x * 0.5 + 0.5) * super::shadows::DIRECTIONAL_SHADOW_RESOLUTION as f32,
        (0.5 - ndc.y * 0.5) * super::shadows::DIRECTIONAL_SHADOW_RESOLUTION as f32,
    )
}

fn grayscale_image_difference(a: &[u32], b: &[u32]) -> (usize, u8, f64) {
    assert_eq!(a.len(), b.len());
    let mut changed = 0usize;
    let mut max_delta = 0u8;
    let mut total_delta = 0u64;
    for (left, right) in a.iter().zip(b) {
        let left = ((left >> 16) & 0xff) as u8;
        let right = ((right >> 16) & 0xff) as u8;
        let delta = left.abs_diff(right);
        max_delta = max_delta.max(delta);
        total_delta += u64::from(delta);
        if delta > 1 {
            changed += 1;
        }
    }
    (
        changed,
        max_delta,
        total_delta as f64 / a.len().max(1) as f64,
    )
}

fn grayscale_shift_difference(
    reference: &[u32],
    current: &[u32],
    width: u32,
    height: u32,
    dx: i32,
    dy: i32,
) -> (usize, u8, f64, usize) {
    assert_eq!(reference.len(), (width * height) as usize);
    assert_eq!(current.len(), reference.len());
    let mut changed = 0usize;
    let mut max_delta = 0u8;
    let mut total_delta = 0u64;
    let mut compared = 0usize;
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let reference_x = x - dx;
            let reference_y = y - dy;
            if reference_x < 0
                || reference_y < 0
                || reference_x >= width as i32
                || reference_y >= height as i32
            {
                continue;
            }
            let current_pixel = current[(y as u32 * width + x as u32) as usize];
            let reference_pixel =
                reference[(reference_y as u32 * width + reference_x as u32) as usize];
            let current_gray = ((current_pixel >> 16) & 0xff) as u8;
            let reference_gray = ((reference_pixel >> 16) & 0xff) as u8;
            let delta = current_gray.abs_diff(reference_gray);
            max_delta = max_delta.max(delta);
            total_delta += u64::from(delta);
            if delta > 1 {
                changed += 1;
            }
            compared += 1;
        }
    }
    (
        changed,
        max_delta,
        total_delta as f64 / compared.max(1) as f64,
        compared,
    )
}

#[test]
#[ignore = "requires a working wgpu adapter; verifies all four depth layers remain pixel-stable under sub-texel camera translation"]
fn directional_cascade_depth_layers_are_stable_under_sub_texel_camera_motion() {
    const WIDTH: u32 = 1024;
    const HEIGHT: u32 = 1024;
    let context = pollster::block_on(GpuContext::new_offscreen(WIDTH, HEIGHT))
        .expect("CSM texel-stability test requires a working wgpu adapter/device");
    let device = context.device();
    let queue = context.queue();
    let mut renderer = MeshRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
    let cube_model = |position: Vec3, scale: Vec3| {
        Mat4::from_scale_rotation_translation(scale, glam::Quat::IDENTITY, position)
    };
    let draws = [
        cube(
            cube_model(Vec3::new(0.0, -0.12, -20.0), Vec3::new(100.0, 0.24, 120.0)),
            [0.68, 0.70, 0.66],
        ),
        cube(
            cube_model(Vec3::new(-3.0, 1.6, 12.0), Vec3::new(1.6, 3.2, 1.6)),
            [0.30, 0.36, 0.32],
        ),
        cube(
            cube_model(Vec3::new(3.0, 2.0, 0.0), Vec3::new(1.6, 4.0, 1.6)),
            [0.36, 0.32, 0.28],
        ),
        cube(
            cube_model(Vec3::new(-4.0, 2.4, -15.0), Vec3::new(1.8, 4.8, 1.8)),
            [0.28, 0.32, 0.38],
        ),
        cube(
            cube_model(Vec3::new(4.0, 2.8, -38.0), Vec3::new(2.0, 5.6, 2.0)),
            [0.32, 0.36, 0.30],
        ),
        cube(
            cube_model(Vec3::new(-5.0, 3.0, -62.0), Vec3::new(2.2, 6.0, 2.2)),
            [0.38, 0.30, 0.32],
        ),
    ];
    let sun = SunLight {
        direction: Vec3::new(-0.65, -0.65, -0.35).normalize(),
        color: Vec3::splat(1.8),
        ambient: Vec3::splat(0.035),
    };
    let base_eye = Vec3::new(0.0, 6.0, 20.0);
    let base_target = Vec3::new(0.0, 0.0, -20.0);
    let base_camera = Transform3D::looking_at(base_eye, base_target, Vec3::Y);
    let camera = Camera3D {
        fov_y: 70.0_f32.to_radians(),
        near: 0.05,
        far: 80.0,
    };
    let scene_at_offset = |offset: Vec3| {
        let eye = base_eye + offset;
        let view = Mat4::from_rotation_translation(base_camera.rotation, eye).inverse();
        Scene {
            view_proj: camera.projection(WIDTH as f32 / HEIGHT as f32) * view,
            camera_position: eye,
            camera_rotation: base_camera.rotation,
            sun: &sun,
            spot_lights: &[],
            point_lights: &[],
            fog: FogSettings::default(),
            draws: &draws,
            particles: &[],
        }
    };

    let output_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/csm_texel_stability");
    std::fs::create_dir_all(&output_dir).expect("create CSM texel-stability output directory");
    let mut csv = String::from(
        "cascade,texel_world_size,within_fraction_x,within_fraction_y,occupied_depth_pixels,within_changed_pixels,within_max_delta,within_mean_delta,crossing_expected_dx,crossing_expected_dy,crossing_phase_x,crossing_changed_pixels,crossing_max_delta,crossing_mean_delta,crossing_compared_pixels\n",
    );

    let seed_scene = scene_at_offset(Vec3::ZERO);
    for layer in 0..super::shadows::DIRECTIONAL_CASCADE_COUNT {
        let seed = directional_cascade_snap_debug(&seed_scene, layer)
            .expect("valid cascade snap diagnostics");

        // The former one-shot alignment assumed the first frustum reconstruction retained the
        // exact same texel size. On a 1024x1024 test camera, inverse-VP f32 reconstruction can
        // perturb the fitted radius enough to leave the requested fraction slightly misplaced.
        // Use feedback from the same runtime diagnostic helper to align the *measured* center.
        const TARGET_FRACTION: f32 = 0.10;
        let mut alignment = seed.right
            * (seed.snapped_x + seed.texel_size * TARGET_FRACTION - seed.center_x)
            + seed.up * (seed.snapped_y + seed.texel_size * TARGET_FRACTION - seed.center_y);
        for _ in 0..32 {
            let probe_scene = scene_at_offset(alignment);
            let probe = directional_cascade_snap_debug(&probe_scene, layer)
                .expect("cascade snap diagnostics while calibrating GPU test");
            let fraction_x = (probe.center_x - probe.snapped_x) / probe.texel_size;
            let fraction_y = (probe.center_y - probe.snapped_y) / probe.texel_size;
            let error_x = TARGET_FRACTION - fraction_x;
            let error_y = TARGET_FRACTION - fraction_y;
            if error_x.abs().max(error_y.abs()) <= 2.0e-5 {
                break;
            }
            alignment += probe.right * (error_x * probe.texel_size * 0.5)
                + probe.up * (error_y * probe.texel_size * 0.5);
        }

        let aligned_scene = scene_at_offset(alignment);
        let aligned = directional_cascade_snap_debug(&aligned_scene, layer)
            .expect("aligned cascade snap diagnostics");
        let base_fraction_x = (aligned.center_x - aligned.snapped_x) / aligned.texel_size;
        let base_fraction_y = (aligned.center_y - aligned.snapped_y) / aligned.texel_size;
        // The inverse-VP reconstruction can retain a small per-axis residual even after
        // iterative feedback (the translation is sub-millimeter for the near cascades).
        // The exact residual is not the property under test: both later probes are positioned
        // from these measured fractions. Require a broad, safe interior window instead of
        // failing on an arbitrary target tolerance; this keeps at least 0.25 texel of margin
        // from the rounding boundary and avoids treating f32 reconstruction noise as shimmer.
        const MIN_BASELINE_FRACTION: f32 = 0.025;
        const MAX_BASELINE_FRACTION: f32 = 0.25;
        assert!(
            (MIN_BASELINE_FRACTION..=MAX_BASELINE_FRACTION).contains(&base_fraction_x)
                && (MIN_BASELINE_FRACTION..=MAX_BASELINE_FRACTION)
                    .contains(&base_fraction_y),
            "cascade {layer} GPU probe baseline is too close to a snap-cell edge: measured=({base_fraction_x:.6}, {base_fraction_y:.6}), safe_window=[{MIN_BASELINE_FRACTION:.3}, {MAX_BASELINE_FRACTION:.2}]"
        );

        // Place the within-cell probe using one fixed world-space translation from the measured
        // baseline. Do not run an iterative coordinate solver here: the inverse-VP frustum is
        // reconstructed in f32, so repeated tiny corrections can hit a quantization floor even
        // though a direct sub-texel displacement is well-defined. The expected phase is computed
        // from the measured baseline phase, while the renderer's snap/matrix/image assertions
        // below remain strict and independently verify that the probe stayed in the same cell.
        const WITHIN_CELL_FRACTION: f32 = 0.35;
        let within_offset = alignment
            + aligned.right * (aligned.texel_size * (WITHIN_CELL_FRACTION - base_fraction_x))
            + aligned.up * (aligned.texel_size * (WITHIN_CELL_FRACTION - base_fraction_y));
        let within_scene = scene_at_offset(within_offset);
        let within = directional_cascade_snap_debug(&within_scene, layer)
            .expect("within-cell cascade snap diagnostics");
        let within_fraction_x =
            ((within.center_x - aligned.snapped_x) / aligned.texel_size).rem_euclid(1.0);
        let within_fraction_y =
            ((within.center_y - aligned.snapped_y) / aligned.texel_size).rem_euclid(1.0);
        const MIN_WITHIN_PHASE: f32 = 0.30;
        const MAX_WITHIN_PHASE: f32 = 0.40;
        assert!(
            (MIN_WITHIN_PHASE..=MAX_WITHIN_PHASE).contains(&within_fraction_x)
                && (MIN_WITHIN_PHASE..=MAX_WITHIN_PHASE).contains(&within_fraction_y),
            "cascade {layer} GPU within-cell probe missed the safe interior: measured=({within_fraction_x:.6}, {within_fraction_y:.6}), safe_window=[{MIN_WITHIN_PHASE:.2}, {MAX_WITHIN_PHASE:.2}]"
        );
        assert_eq!(
            within.texel_size, aligned.texel_size,
            "cascade {layer} changed world-space texel size inside the tested snap cell"
        );
        assert_eq!(within.snapped_x, aligned.snapped_x);
        assert_eq!(within.snapped_y, aligned.snapped_y);
        // The interval width is fixed by the shared quantized radius, but its absolute
        // light-space anchor is formed from the f32 camera position. Constructing a translated
        // world-space eye rounds each component independently, so a nominal light-plane move
        // can leave a one-ULP residual in dot(camera_position, light_direction). This is not a
        // change in the fitted sphere or its depth extent. Compare both endpoints using a
        // scale-aware f32 tolerance, and check the span separately so a real radius/padding
        // change cannot be hidden by the endpoint allowance.
        let base_depth_span = aligned.depth_max - aligned.depth_min;
        let within_depth_span = within.depth_max - within.depth_min;
        let depth_coordinate_scale = aligned
            .depth_min
            .abs()
            .max(aligned.depth_max.abs())
            .max(within.depth_min.abs())
            .max(within.depth_max.abs())
            .max(1.0);
        let endpoint_tolerance = (depth_coordinate_scale * f32::EPSILON).max(5.0e-6);
        let span_tolerance =
            (base_depth_span.abs().max(within_depth_span.abs()) * f32::EPSILON).max(2.0e-6);
        let min_endpoint_delta = (within.depth_min - aligned.depth_min).abs();
        let max_endpoint_delta = (within.depth_max - aligned.depth_max).abs();
        let span_delta = (within_depth_span - base_depth_span).abs();
        assert!(
            min_endpoint_delta <= endpoint_tolerance
                && max_endpoint_delta <= endpoint_tolerance
                && span_delta <= span_tolerance,
            "cascade {layer} light-space depth fit drifted inside an XY snap cell: base=({:.9}, {:.9}), within=({:.9}, {:.9}), endpoint_delta=({min_endpoint_delta:.9}, {max_endpoint_delta:.9}), span_delta={span_delta:.9}, tolerances=(endpoint {endpoint_tolerance:.9}, span {span_tolerance:.9})",
            aligned.depth_min,
            aligned.depth_max,
            within.depth_min,
            within.depth_max,
        );

        let base_matrix = MeshRenderer::directional_shadow_cascades(&aligned_scene).matrices[layer];
        let within_matrix =
            MeshRenderer::directional_shadow_cascades(&within_scene).matrices[layer];
        let base_matrix_values = base_matrix.to_cols_array();
        let within_matrix_values = within_matrix.to_cols_array();
        let (matrix_delta_index, matrix_delta) = base_matrix_values
            .into_iter()
            .zip(within_matrix_values)
            .enumerate()
            .map(|(index, (left, right))| (index, (left - right).abs()))
            .max_by(|left, right| left.1.total_cmp(&right.1))
            .expect("matrices contain sixteen elements");
        assert!(
            matrix_delta <= 1.0e-7,
            "cascade {layer} changed matrix within a snap cell: max_delta={matrix_delta} at column-major element {matrix_delta_index}"
        );

        let base_pixels = render_directional_depth_layer_frame(
            &context,
            &mut renderer,
            &aligned_scene,
            (WIDTH, HEIGHT),
            layer as u32,
        );
        let within_pixels = render_directional_depth_layer_frame(
            &context,
            &mut renderer,
            &within_scene,
            (WIDTH, HEIGHT),
            layer as u32,
        );
        let occupied_depth_pixels = base_pixels
            .iter()
            .filter(|pixel| ((**pixel >> 16) & 0xff) > 1)
            .count();
        assert!(
            occupied_depth_pixels > 128,
            "cascade {layer} depth preview contains too little caster data: {occupied_depth_pixels} pixels"
        );
        let (within_changed, within_max, within_mean) =
            grayscale_image_difference(&base_pixels, &within_pixels);
        if within_changed != 0 || within_max > 1 {
            // Preserve the exact failing GPU frames and a compact list of changed pixels so a
            // failure can be classified as matrix drift, depth quantization, or raster coverage.
            let base_failure_path = output_dir.join(format!("cascade_{layer}_base_failure.png"));
            let within_failure_path =
                output_dir.join(format!("cascade_{layer}_within_failure.png"));
            write_rgba_png(
                &base_failure_path,
                WIDTH,
                HEIGHT,
                &rgba_from_packed_pixels(&base_pixels),
            );
            write_rgba_png(
                &within_failure_path,
                WIDTH,
                HEIGHT,
                &rgba_from_packed_pixels(&within_pixels),
            );
            let changed_pixels = base_pixels
                .iter()
                .zip(&within_pixels)
                .enumerate()
                .filter_map(|(index, (base, within))| {
                    let base_gray = ((base >> 16) & 0xff) as u8;
                    let within_gray = ((within >> 16) & 0xff) as u8;
                    let delta = base_gray.abs_diff(within_gray);
                    (delta > 1).then_some((
                        index as u32 % WIDTH,
                        index as u32 / WIDTH,
                        base_gray,
                        within_gray,
                        delta,
                    ))
                })
                .take(16)
                .collect::<Vec<_>>();
            eprintln!(
                "CSM texel probe failure diagnostics: cascade={layer}, matrix_delta={matrix_delta:.9} at element {matrix_delta_index}, base_center_depth={:.9}, within_center_depth={:.9}, base_depth_range=({:.9}, {:.9}), within_depth_range=({:.9}, {:.9}), base_snap=({:.9}, {:.9}), within_snap=({:.9}, {:.9}), texel={:.9}, changed_pixels={changed_pixels:?}, base_png={}, within_png={}",
                (aligned.depth_min + aligned.depth_max) * 0.5,
                (within.depth_min + within.depth_max) * 0.5,
                aligned.depth_min,
                aligned.depth_max,
                within.depth_min,
                within.depth_max,
                aligned.snapped_x,
                aligned.snapped_y,
                within.snapped_x,
                within.snapped_y,
                aligned.texel_size,
                base_failure_path.display(),
                within_failure_path.display(),
            );
        }
        assert!(
            within_changed == 0 && within_max <= 1,
            "cascade {layer} depth image changed inside a snap cell: changed={within_changed}, max_delta={within_max}, mean={within_mean:.6}; failure PNGs and pixel diagnostics were emitted above"
        );

        // Cross exactly one rounding boundary with a single translation derived from the
        // measured baseline phase. This avoids asking an f32 inverse-VP solver to converge to an
        // exact world-space coordinate; the measured phase and one-texel snap delta below remain
        // the acceptance criteria.
        const CROSSING_FRACTION: f32 = 0.65;
        let crossing_offset = alignment
            + aligned.right * (aligned.texel_size * (CROSSING_FRACTION - base_fraction_x));
        let crossing_scene = scene_at_offset(crossing_offset);
        let crossing = directional_cascade_snap_debug(&crossing_scene, layer)
            .expect("crossing cascade snap diagnostics");
        let crossing_phase_x =
            ((crossing.center_x - aligned.snapped_x) / aligned.texel_size).rem_euclid(1.0);
        let _crossing_phase_y =
            ((crossing.center_y - aligned.snapped_y) / aligned.texel_size).rem_euclid(1.0);
        assert_eq!(
            crossing.texel_size, aligned.texel_size,
            "cascade {layer} changed world-space texel size while crossing one snap boundary"
        );
        const MIN_CROSSING_PHASE: f32 = 0.60;
        const MAX_CROSSING_PHASE: f32 = 0.70;
        assert!(
            (MIN_CROSSING_PHASE..=MAX_CROSSING_PHASE).contains(&crossing_phase_x),
            "cascade {layer} GPU crossing probe did not reach the safe post-boundary interval: measured={crossing_phase_x:.6}, safe_window=[{MIN_CROSSING_PHASE:.2}, {MAX_CROSSING_PHASE:.2}]"
        );
        // `snapped_x` is the nearest grid coordinate, not the lower edge of a cell. Once the
        // unwrapped cell phase passes 0.5, the signed residual therefore wraps from +0.5 to -0.5.
        let crossing_signed_fraction =
            (crossing.center_x - crossing.snapped_x) / crossing.texel_size;
        assert!(
            ((crossing.snapped_x - aligned.snapped_x).abs() - aligned.texel_size).abs()
                <= aligned.texel_size * 2.0e-3,
            "cascade {layer} did not snap by exactly one texel: phase={crossing_phase_x:.6}, signed_residual={crossing_signed_fraction:.6}"
        );
        assert!((crossing.snapped_y - aligned.snapped_y).abs() <= aligned.texel_size * 1.0e-4);
        let crossing_matrix =
            MeshRenderer::directional_shadow_cascades(&crossing_scene).matrices[layer];
        let base_pixel = shadow_map_pixel_position(base_matrix, aligned.center);
        let crossing_pixel = shadow_map_pixel_position(crossing_matrix, aligned.center);
        let pixel_delta = crossing_pixel - base_pixel;
        assert!(
            (pixel_delta.x.abs() - 1.0).abs() <= 0.03 && pixel_delta.y.abs() <= 0.03,
            "cascade {layer} projection should move by one texel, got {pixel_delta:?}"
        );
        let expected_dx = pixel_delta.x.round() as i32;
        let expected_dy = pixel_delta.y.round() as i32;
        let crossing_pixels = render_directional_depth_layer_frame(
            &context,
            &mut renderer,
            &crossing_scene,
            (WIDTH, HEIGHT),
            layer as u32,
        );
        let (cross_changed, cross_max, cross_mean, cross_compared) = grayscale_shift_difference(
            &base_pixels,
            &crossing_pixels,
            WIDTH,
            HEIGHT,
            expected_dx,
            expected_dy,
        );
        assert!(
            cross_mean <= 0.25 && cross_changed as f64 / cross_compared.max(1) as f64 <= 0.005,
            "cascade {layer} depth layer is not a one-texel translation after snap: dx={expected_dx}, dy={expected_dy}, changed={cross_changed}/{cross_compared}, max={cross_max}, mean={cross_mean:.4}"
        );

        let base_path = output_dir.join(format!("cascade_{layer}_base.png"));
        let within_path = output_dir.join(format!("cascade_{layer}_within_cell.png"));
        let crossing_path = output_dir.join(format!("cascade_{layer}_after_one_texel.png"));
        write_rgba_png(
            &base_path,
            WIDTH,
            HEIGHT,
            &rgba_from_packed_pixels(&base_pixels),
        );
        write_rgba_png(
            &within_path,
            WIDTH,
            HEIGHT,
            &rgba_from_packed_pixels(&within_pixels),
        );
        write_rgba_png(
            &crossing_path,
            WIDTH,
            HEIGHT,
            &rgba_from_packed_pixels(&crossing_pixels),
        );
        csv.push_str(&format!(
            "{layer},{:.8},{within_fraction_x:.6},{within_fraction_y:.6},{occupied_depth_pixels},{within_changed},{within_max},{within_mean:.8},{expected_dx},{expected_dy},{crossing_phase_x:.6},{cross_changed},{cross_max},{cross_mean:.8},{cross_compared}\n",
            aligned.texel_size,
        ));
    }

    std::fs::write(output_dir.join("texel_stability.csv"), csv)
        .expect("write CSM texel-stability metrics");
    std::fs::write(
        output_dir.join("README.txt"),
        "Ribbon Engine cascaded directional shadow texel-stability diagnostic\n\
         GPU: one 1024x1024 depth-layer preview per cascade and camera pose.\n\
         For each layer the camera is translated in light-space with feedback against fixed world-space center coordinates derived from the baseline snap grid. This avoids moving the target grid while measuring the result. The baseline begins near 0.1 texel within a snap cell.\n\
         The within-cell pose targets 0.35 texel on both light-space axes, leaving 0.15 texel of margin from the half-texel boundary; the selected cascade matrix and depth preview must remain unchanged.\n\
         The crossing pose targets world-space X at 0.65 texel beyond the baseline snap origin. Because the snap origin is rounded to the nearest grid point, the signed residual is expected to wrap to about -0.35 texel; the snap origin must advance by exactly one world texel and the depth preview must match a one-pixel translation. The CSV records the wrapped phase in [0, 1).\n\
         This validates discrete sub-texel stabilization of the fitted depth map; it is not a continuous-time animation capture and does not by itself prove all camera-rotation behavior.\n\
         Files: cascade_N_base.png, cascade_N_within_cell.png, cascade_N_after_one_texel.png, texel_stability.csv.\n\
         Compare the base/within images pixel-for-pixel. The after-one-texel image should match the base after the integer pixel offset in CSV.\n",
    )
    .expect("write CSM texel-stability README");
    eprintln!("CSM texel-stability diagnostics: {}", output_dir.display());
}

fn decode_cascade_debug_color(rgb: [u8; 3]) -> usize {
    const PALETTE: [[i32; 3]; 4] = [[242, 31, 31], [31, 230, 56], [31, 89, 255], [255, 209, 20]];
    PALETTE
        .iter()
        .enumerate()
        .map(|(index, color)| {
            let distance = (0..3)
                .map(|channel| (i32::from(rgb[channel]) - color[channel]).pow(2))
                .sum::<i32>();
            (index, distance)
        })
        .min_by_key(|(_, distance)| *distance)
        .expect("cascade palette is non-empty")
        .0
}

#[test]
fn directional_shadow_debug_defaults_to_disabled_and_clamps_depth_layer() {
    assert_eq!(
        DirectionalShadowDebugOptions::default(),
        DirectionalShadowDebugOptions {
            mode: DirectionalShadowDebugMode::Disabled,
            depth_layer: 0,
        }
    );
    assert_eq!(DirectionalShadowDebugMode::ShadowVisibility as u32, 4);
    assert_eq!(
        DirectionalShadowDebugMode::ShadowVisibilityComponents as u32,
        5
    );
    let settings = DirectionalShadowDebugOptions {
        mode: DirectionalShadowDebugMode::DepthLayer,
        depth_layer: u32::MAX,
    }
    .sanitized();
    assert_eq!(settings.mode, DirectionalShadowDebugMode::DepthLayer);
    assert_eq!(settings.depth_layer, 3);
    assert_eq!(std::mem::size_of::<super::ShadowDepthDebugUniform>(), 32);
}

#[test]
fn skinned_vertex_attributes_match_tangent_uv_and_eight_influence_storage() {
    use crate::mesh::{SkinnedVertex, Vertex};
    use std::mem::{offset_of, size_of};

    let offset = |location| {
        super::VERTEX_ATTRIBUTES
            .iter()
            .find(|attribute| attribute.shader_location == location)
            .unwrap_or_else(|| panic!("missing vertex shader location {location}"))
            .offset
    };

    // The old CPU input remains source-compatible; the GPU upload format is explicit and larger.
    assert_eq!(size_of::<Vertex>(), 80);
    assert_eq!(size_of::<SkinnedVertex>(), 104);
    assert_eq!(offset(0), offset_of!(SkinnedVertex, position) as u64);
    assert_eq!(offset(1), offset_of!(SkinnedVertex, normal) as u64);
    assert_eq!(offset(2), offset_of!(SkinnedVertex, uv) as u64);
    assert_eq!(offset(3), offset_of!(SkinnedVertex, tangent) as u64);
    assert_eq!(offset(4), offset_of!(SkinnedVertex, uv1) as u64);
    assert_eq!(offset(5), offset_of!(SkinnedVertex, joints) as u64);
    assert_eq!(
        offset(6),
        (offset_of!(SkinnedVertex, joints) + size_of::<[u16; 4]>()) as u64
    );
    assert_eq!(offset(7), offset_of!(SkinnedVertex, weights) as u64);
    assert_eq!(
        offset(8),
        (offset_of!(SkinnedVertex, weights) + size_of::<[f32; 4]>()) as u64
    );
}

#[test]
fn material_texture_uv_sets_encode_each_role_and_fall_back_above_uv1() {
    let (uv_sets, extra) =
        super::encode_material_texture_coordinates(super::MaterialTextureCoordinates {
            base_color: 1,
            metallic_roughness: 0,
            normal: 1,
            occlusion: 2,
            emissive: 1,
        });
    assert_eq!(uv_sets, [1, 0, 1, 0]);
    assert_eq!(extra, [1, 0, 0, 0]);
}

#[test]
fn static_vertex_attributes_match_the_compact_layout() {
    use crate::mesh::StaticVertex;
    use std::mem::{offset_of, size_of};

    assert_eq!(size_of::<StaticVertex>(), 56);
    assert_eq!(size_of::<super::MaterialUniform>(), 96);
    assert_eq!(offset_of!(super::MaterialUniform, texture_uv_sets), 64);
    assert_eq!(
        offset_of!(super::MaterialUniform, texture_uv_sets_extra),
        80
    );
    let expected = [
        (0, offset_of!(StaticVertex, position) as u64),
        (1, offset_of!(StaticVertex, normal) as u64),
        (2, offset_of!(StaticVertex, uv) as u64),
        (3, offset_of!(StaticVertex, tangent) as u64),
        (4, offset_of!(StaticVertex, uv1) as u64),
    ];
    for (location, offset) in expected {
        let attribute = super::STATIC_VERTEX_ATTRIBUTES
            .iter()
            .find(|attribute| attribute.shader_location == location)
            .unwrap_or_else(|| panic!("missing static vertex shader location {location}"));
        assert_eq!(attribute.offset, offset);
    }
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
fn directional_shadow_caster_settings_sanitize_distance_without_disabling_by_default() {
    let defaults = DirectionalShadowCasterSettings::default();
    assert_eq!(
        defaults.max_extrusion_distance,
        super::DEFAULT_DIRECTIONAL_SHADOW_CASTER_EXTRUSION_DISTANCE
    );
    assert_eq!(
        DirectionalShadowCasterSettings {
            max_extrusion_distance: -10.0,
        }
        .sanitized()
        .max_extrusion_distance,
        0.0
    );
    assert_eq!(
        DirectionalShadowCasterSettings {
            max_extrusion_distance: f32::INFINITY,
        }
        .sanitized()
        .max_extrusion_distance,
        super::DEFAULT_DIRECTIONAL_SHADOW_CASTER_EXTRUSION_DISTANCE
    );
    assert_eq!(
        DirectionalShadowCasterSettings {
            max_extrusion_distance: f32::MAX,
        }
        .sanitized()
        .max_extrusion_distance,
        super::MAX_DIRECTIONAL_SHADOW_CASTER_EXTRUSION_DISTANCE
    );
}

#[test]
fn lighting_post_and_shadow_wgsl_shaders_validate_without_a_gpu_adapter() {
    let shaders = [
        include_str!("../../shaders/mesh.wgsl"),
        include_str!("../../shaders/shadow.wgsl"),
        include_str!("../../shaders/mesh_static.wgsl"),
        include_str!("../../shaders/shadow_static.wgsl"),
        include_str!("../../shaders/shadow_debug.wgsl"),
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
fn spotlight_selection_prefers_the_most_important_shadow_caster() {
    let non_shadow = SpotLight {
        position: Vec3::new(1.0, 2.0, 3.0),
        casts_shadows: false,
        ..SpotLight::default()
    };
    let distant_shadow = SpotLight {
        position: Vec3::new(0.0, 0.0, -18.0),
        casts_shadows: true,
        ..SpotLight::default()
    };
    let nearby_shadow = SpotLight {
        position: Vec3::new(0.0, 0.0, -2.0),
        casts_shadows: true,
        ..SpotLight::default()
    };
    let lights = [non_shadow, distant_shadow, nearby_shadow];
    let (active, caster) = select_spot_lights(&lights, Vec3::ZERO);
    assert_eq!(active, Some(nearby_shadow));
    assert_eq!(caster, Some(nearby_shadow));

    let reversed = [nearby_shadow, distant_shadow, non_shadow];
    assert_eq!(
        select_spot_lights(&reversed, Vec3::ZERO),
        (Some(nearby_shadow), Some(nearby_shadow))
    );
}

#[test]
fn spotlight_selection_ranks_intensity_range_and_color_contribution() {
    let weak = SpotLight {
        position: Vec3::new(0.0, 0.0, -1.0),
        intensity: 1.0,
        range: 2.0,
        color: Vec3::splat(0.1),
        ..SpotLight::default()
    };
    let strong = SpotLight {
        position: Vec3::new(0.0, 0.0, -5.0),
        intensity: 80.0,
        range: 20.0,
        color: Vec3::ONE,
        ..SpotLight::default()
    };

    assert_eq!(
        select_spot_lights(&[weak, strong], Vec3::ZERO),
        (Some(strong), Some(strong))
    );
}

#[test]
fn spotlight_selection_uses_stable_value_ties_not_input_order() {
    let left = SpotLight {
        position: Vec3::new(-2.0, 0.0, 0.0),
        ..SpotLight::default()
    };
    let right = SpotLight {
        position: Vec3::new(2.0, 0.0, 0.0),
        ..SpotLight::default()
    };
    let first = select_spot_lights(&[right, left], Vec3::ZERO);
    let second = select_spot_lights(&[left, right], Vec3::ZERO);

    assert_eq!(first, second);
    assert_eq!(first, (Some(left), Some(left)));
}

#[test]
fn spotlight_selection_falls_back_to_the_best_unshadowed_light() {
    let distant = SpotLight {
        position: Vec3::new(0.0, 0.0, -16.0),
        casts_shadows: false,
        ..SpotLight::default()
    };
    let nearby = SpotLight {
        position: Vec3::new(0.0, 0.0, -1.0),
        casts_shadows: false,
        ..SpotLight::default()
    };

    assert_eq!(
        select_spot_lights(&[distant, nearby], Vec3::ZERO),
        (Some(nearby), None)
    );
}

#[test]
fn spotlight_selection_rejects_non_finite_and_non_contributing_lights() {
    let invalid = SpotLight {
        position: Vec3::splat(f32::NAN),
        ..SpotLight::default()
    };
    let zero_intensity = SpotLight {
        intensity: 0.0,
        ..SpotLight::default()
    };
    let valid = SpotLight::default();

    assert_eq!(
        select_spot_lights(&[invalid, zero_intensity, valid], Vec3::ZERO),
        (Some(valid), Some(valid))
    );
    assert_eq!(
        select_spot_lights(&[valid], Vec3::splat(f32::INFINITY)),
        (None, None)
    );
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
#[ignore = "requires a working wgpu adapter; writes a four-cascade directional-shadow PNG"]
fn directional_cascades_render_a_headless_png() {
    const WIDTH: u32 = 256;
    const HEIGHT: u32 = 144;
    let context = pollster::block_on(GpuContext::new_offscreen(WIDTH, HEIGHT))
        .expect("directional cascade screenshot test requires a working wgpu adapter/device");
    let (device, queue) = (context.device(), context.queue());
    let mut renderer = MeshRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
    let cube_model = |position: Vec3, scale: Vec3| {
        Mat4::from_scale_rotation_translation(scale, glam::Quat::IDENTITY, position)
    };
    let mut draws = vec![cube(
        cube_model(Vec3::new(0.0, -0.12, -20.0), Vec3::new(80.0, 0.24, 100.0)),
        [0.72, 0.74, 0.69],
    )];
    // Put opaque casters in all four practical camera-depth ranges. The image is intended
    // for visual inspection of cascade coverage and transition seams on the target adapter.
    for (x, depth, height) in [
        (-2.0, 6.5, 2.8),
        (2.0, 11.0, 3.4),
        (-2.0, 22.0, 4.0),
        (2.0, 42.0, 4.8),
    ] {
        draws.push(cube(
            cube_model(
                Vec3::new(x, height * 0.5, 20.0 - depth),
                Vec3::new(1.4, height, 1.4),
            ),
            [0.17, 0.20, 0.18],
        ));
    }

    // Add a weighted legacy mesh as a skinned caster too. This exercises the 104-byte
    // vertex stream in both the main color pass and all four directional shadow passes.
    let mut skinned_mesh_data = crate::mesh::cube();
    for vertex in &mut skinned_mesh_data.vertices {
        vertex.weights = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    }
    let skinned_mesh = renderer.upload_mesh(
        device,
        &skinned_mesh_data.vertices,
        &skinned_mesh_data.indices,
    );
    let skinned_draw = SkinnedDraw {
        draw: Draw {
            mesh: skinned_mesh,
            material: MeshRenderer::WHITE,
            model: cube_model(Vec3::new(3.0, 1.6, -2.0), Vec3::new(1.6, 3.2, 1.6)),
            tint: [0.24, 0.18, 0.14],
        },
        joint_matrices: Arc::from([Mat4::IDENTITY]),
    };

    let camera = Transform3D::looking_at(
        Vec3::new(0.0, 6.0, 20.0),
        Vec3::new(0.0, 0.0, -20.0),
        Vec3::Y,
    );
    let view = Mat4::from_rotation_translation(camera.rotation, camera.translation).inverse();
    let sun = SunLight {
        direction: Vec3::new(-0.65, -0.65, -0.35).normalize(),
        color: Vec3::splat(1.8),
        ambient: Vec3::splat(0.035),
    };
    clear_view(
        device,
        queue,
        context.target().view(),
        [0.002, 0.003, 0.004, 1.0],
    );
    renderer.render_with_skinning(
        device,
        queue,
        context.target().view(),
        (WIDTH, HEIGHT),
        &Scene {
            view_proj: Camera3D {
                fov_y: 70.0_f32.to_radians(),
                near: 0.05,
                far: 80.0,
            }
            .projection(WIDTH as f32 / HEIGHT as f32)
                * view,
            camera_position: camera.translation,
            camera_rotation: camera.rotation,
            sun: &sun,
            spot_lights: &[],
            point_lights: &[],
            fog: FogSettings {
                color: Vec3::ZERO,
                density: 0.0,
                base_height: 0.0,
                height_falloff: 0.0,
            },
            draws: &draws,
            particles: &[],
        },
        &[skinned_draw],
    );

    let pixels = context
        .readback()
        .expect("read back cascaded directional-shadow fixture")
        .pixels;
    let mut rendered_pixels = 0usize;
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for packed_pixel in pixels {
        let red = ((packed_pixel >> 16) & 0xff) as u8;
        let green = ((packed_pixel >> 8) & 0xff) as u8;
        let blue = (packed_pixel & 0xff) as u8;
        let luminance = (u32::from(red) * 3 + u32::from(green) * 6 + u32::from(blue)) / 10;
        if luminance > 12 {
            rendered_pixels += 1;
        }
        rgba.extend_from_slice(&[red, green, blue, 255]);
    }

    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/cascaded_directional_shadows.png");
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent).expect("create target directory for CSM screenshot");
    }
    let file = File::create(&output).expect("create CSM screenshot PNG");
    let mut encoder = png::Encoder::new(file, WIDTH, HEIGHT);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("write CSM PNG header");
    writer
        .write_image_data(&rgba)
        .expect("write CSM screenshot image data");
    eprintln!(
        "cascaded directional-shadow screenshot: {}",
        output.display()
    );
    assert!(
        rendered_pixels > (WIDTH * HEIGHT / 20) as usize,
        "directional cascade fixture should contain a visible rendered scene; lit pixels={rendered_pixels}; inspect {}",
        output.display()
    );
}

#[test]
#[ignore = "requires a working wgpu adapter; validates an off-frustum static directional caster"]
fn directional_csm_static_off_frustum_caster_extends_depth_and_negative_controls_do_not() {
    const WIDTH: u32 = 256;
    const HEIGHT: u32 = 144;
    let context = pollster::block_on(GpuContext::new_offscreen(WIDTH, HEIGHT))
        .expect("static caster CSM GPU test requires a working adapter");
    let (device, queue) = (context.device(), context.queue());
    let mut renderer = MeshRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
    let camera = Transform3D::looking_at(Vec3::new(0.0, 3.0, 6.0), Vec3::ZERO, Vec3::Y);
    let view = Mat4::from_rotation_translation(camera.rotation, camera.translation).inverse();
    let scene_camera = Camera3D {
        fov_y: 70.0_f32.to_radians(),
        near: 0.05,
        far: 80.0,
    };
    let sun = SunLight {
        direction: Vec3::new(-1.0, -1.0, -1.0).normalize(),
        color: Vec3::splat(1.8),
        ambient: Vec3::splat(0.035),
    };
    let receiver = cube(
        Mat4::from_scale_rotation_translation(
            Vec3::new(24.0, 0.2, 24.0),
            glam::Quat::IDENTITY,
            Vec3::new(0.0, -0.1, 0.0),
        ),
        [0.8, 0.8, 0.8],
    );
    let light_direction = sun.direction.normalize();
    let caster_center = -light_direction * 25.0;
    let caster = cube(
        Mat4::from_scale_rotation_translation(
            Vec3::splat(4.0),
            glam::Quat::IDENTITY,
            caster_center,
        ),
        [0.2, 0.2, 0.2],
    );
    let view_proj = scene_camera.projection(WIDTH as f32 / HEIGHT as f32) * view;
    let caster_clip = view_proj * caster_center.extend(1.0);
    assert!(caster_clip.is_finite() && caster_clip.w.abs() > 1.0e-6);
    for corner in [
        Vec3::new(-1.0, -1.0, -1.0),
        Vec3::new(1.0, -1.0, -1.0),
        Vec3::new(-1.0, 1.0, -1.0),
        Vec3::new(1.0, 1.0, -1.0),
        Vec3::new(-1.0, -1.0, 1.0),
        Vec3::new(1.0, -1.0, 1.0),
        Vec3::new(-1.0, 1.0, 1.0),
        Vec3::new(1.0, 1.0, 1.0),
    ] {
        let clip = view_proj * (caster_center + corner * 2.0).extend(1.0);
        assert!(clip.is_finite());
        assert!(
            clip.w <= 0.0,
            "the complete static caster must be behind the camera: {clip:?}"
        );
    }

    let no_caster_draws = [receiver];
    let receiver_scene = Scene {
        view_proj,
        camera_position: camera.translation,
        camera_rotation: camera.rotation,
        sun: &sun,
        spot_lights: &[],
        point_lights: &[],
        fog: FogSettings {
            color: Vec3::ZERO,
            density: 0.0,
            base_height: 0.0,
            height_falloff: 0.0,
        },
        draws: &no_caster_draws,
        particles: &[],
    };
    let unshadowed = render_directional_visibility_frame(
        &context,
        &mut renderer,
        &receiver_scene,
        &[],
        (WIDTH, HEIGHT),
    );
    let unshadowed_center = packed_rgb(&unshadowed, WIDTH, WIDTH / 2, HEIGHT / 2)[0];

    let with_caster_draws = [receiver, caster];
    let caster_scene = Scene {
        view_proj,
        camera_position: camera.translation,
        camera_rotation: camera.rotation,
        sun: &sun,
        spot_lights: &[],
        point_lights: &[],
        fog: FogSettings {
            color: Vec3::ZERO,
            density: 0.0,
            base_height: 0.0,
            height_falloff: 0.0,
        },
        draws: &with_caster_draws,
        particles: &[],
    };
    renderer.set_directional_shadow_caster_extrusion_distance(40.0);
    let shadowed = render_directional_visibility_frame(
        &context,
        &mut renderer,
        &caster_scene,
        &[],
        (WIDTH, HEIGHT),
    );
    let shadowed_center = packed_rgb(&shadowed, WIDTH, WIDTH / 2, HEIGHT / 2)[0];
    assert!(
        unshadowed_center > 220,
        "receiver control should be unshadowed at center, visibility={unshadowed_center}"
    );
    assert!(
        shadowed_center + 50 < unshadowed_center,
        "off-frustum static caster should shadow the receiver, control={unshadowed_center}, with caster={shadowed_center}"
    );

    // Negative control 1: the same caster is beyond the configured extrusion band.
    renderer.set_directional_shadow_caster_extrusion_distance(10.0);
    let beyond_limit = render_directional_visibility_frame(
        &context,
        &mut renderer,
        &caster_scene,
        &[],
        (WIDTH, HEIGHT),
    );
    let beyond_limit_center = packed_rgb(&beyond_limit, WIDTH, WIDTH / 2, HEIGHT / 2)[0];
    assert!(
        beyond_limit_center > shadowed_center + 40,
        "caster beyond extrusion distance must not shadow the receiver: shadowed={shadowed_center}, limited={beyond_limit_center}"
    );

    // Negative control 2: move the caster outside the cascade light-space XY footprint.
    let reference_up = if light_direction.dot(Vec3::Y).abs() > 0.95 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let right = light_direction.cross(reference_up).normalize_or_zero();
    assert!(right.is_finite() && right.length_squared() > 0.5);
    let outside_xy = cube(
        Mat4::from_scale_rotation_translation(
            Vec3::splat(4.0),
            glam::Quat::IDENTITY,
            caster_center + right * 40.0,
        ),
        [0.2, 0.2, 0.2],
    );
    let outside_xy_draws = [receiver, outside_xy];
    let outside_xy_scene = Scene {
        view_proj,
        camera_position: camera.translation,
        camera_rotation: camera.rotation,
        sun: &sun,
        spot_lights: &[],
        point_lights: &[],
        fog: FogSettings {
            color: Vec3::ZERO,
            density: 0.0,
            base_height: 0.0,
            height_falloff: 0.0,
        },
        draws: &outside_xy_draws,
        particles: &[],
    };
    renderer.set_directional_shadow_caster_extrusion_distance(40.0);
    let outside_xy_pixels = render_directional_visibility_frame(
        &context,
        &mut renderer,
        &outside_xy_scene,
        &[],
        (WIDTH, HEIGHT),
    );
    let outside_xy_center = packed_rgb(&outside_xy_pixels, WIDTH, WIDTH / 2, HEIGHT / 2)[0];
    assert!(
        outside_xy_center > shadowed_center + 40,
        "caster without light-space XY overlap must not shadow receiver: shadowed={shadowed_center}, outside_xy={outside_xy_center}"
    );

    // Negative control 3: a caster downstream of the receiver cannot cast toward it.
    let downstream = cube(
        Mat4::from_scale_rotation_translation(
            Vec3::splat(4.0),
            glam::Quat::IDENTITY,
            light_direction * 25.0,
        ),
        [0.2, 0.2, 0.2],
    );
    let downstream_draws = [receiver, downstream];
    let downstream_scene = Scene {
        view_proj,
        camera_position: camera.translation,
        camera_rotation: camera.rotation,
        sun: &sun,
        spot_lights: &[],
        point_lights: &[],
        fog: FogSettings {
            color: Vec3::ZERO,
            density: 0.0,
            base_height: 0.0,
            height_falloff: 0.0,
        },
        draws: &downstream_draws,
        particles: &[],
    };
    let downstream_pixels = render_directional_visibility_frame(
        &context,
        &mut renderer,
        &downstream_scene,
        &[],
        (WIDTH, HEIGHT),
    );
    let downstream_center = packed_rgb(&downstream_pixels, WIDTH, WIDTH / 2, HEIGHT / 2)[0];
    assert!(
        downstream_center > shadowed_center + 40,
        "downstream caster must not shadow receiver: shadowed={shadowed_center}, downstream={downstream_center}"
    );
}

#[test]
#[ignore = "requires a working wgpu adapter; validates current-pose skinned off-frustum caster coverage"]
fn directional_csm_skinned_off_frustum_caster_uses_non_identity_pose_bounds() {
    const WIDTH: u32 = 256;
    const HEIGHT: u32 = 144;
    let context = pollster::block_on(GpuContext::new_offscreen(WIDTH, HEIGHT))
        .expect("skinned caster CSM GPU test requires a working adapter");
    let (device, queue) = (context.device(), context.queue());
    let mut renderer = MeshRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
    let mut skinned_cube = crate::mesh::cube();
    for vertex in &mut skinned_cube.vertices {
        vertex.joints = [0; 8];
        vertex.weights = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    }
    let skinned_mesh = renderer.upload_mesh(device, &skinned_cube.vertices, &skinned_cube.indices);

    let camera = Transform3D::looking_at(Vec3::new(0.0, 3.0, 6.0), Vec3::ZERO, Vec3::Y);
    let view = Mat4::from_rotation_translation(camera.rotation, camera.translation).inverse();
    let view_proj = Camera3D {
        fov_y: 70.0_f32.to_radians(),
        near: 0.05,
        far: 80.0,
    }
    .projection(WIDTH as f32 / HEIGHT as f32)
        * view;
    let sun = SunLight {
        direction: Vec3::new(-1.0, -1.0, -1.0).normalize(),
        color: Vec3::splat(1.8),
        ambient: Vec3::splat(0.035),
    };
    let receiver = cube(
        Mat4::from_scale_rotation_translation(
            Vec3::new(24.0, 0.2, 24.0),
            glam::Quat::IDENTITY,
            Vec3::new(0.0, -0.1, 0.0),
        ),
        [0.8, 0.8, 0.8],
    );
    let light_direction = sun.direction.normalize();
    let deformed_center = -light_direction * 25.0;
    // The non-identity joint translation moves the bind-pose cube by +20Y. Compensating the
    // model translation keeps the *deformed* center on the receiver's light ray. A bind-pose
    // fallback would be displaced by twenty world units and miss the receiver's narrow XY footprint.
    let pose_offset = Vec3::Y * 20.0;
    let pose = Mat4::from_translation(pose_offset);
    let skinned_draw = SkinnedDraw {
        draw: Draw {
            mesh: skinned_mesh,
            material: MeshRenderer::WHITE,
            model: Mat4::from_translation(deformed_center - pose_offset),
            tint: [0.2, 0.2, 0.2],
        },
        joint_matrices: Arc::from([pose]),
    };
    let camera_clip = view_proj * deformed_center.extend(1.0);
    assert!(camera_clip.is_finite() && camera_clip.w.abs() > 1.0e-6);
    for corner in [
        Vec3::new(-0.5, -0.5, -0.5),
        Vec3::new(0.5, -0.5, -0.5),
        Vec3::new(-0.5, 0.5, -0.5),
        Vec3::new(0.5, 0.5, -0.5),
        Vec3::new(-0.5, -0.5, 0.5),
        Vec3::new(0.5, -0.5, 0.5),
        Vec3::new(-0.5, 0.5, 0.5),
        Vec3::new(0.5, 0.5, 0.5),
    ] {
        let clip = view_proj * (deformed_center + corner).extend(1.0);
        assert!(clip.is_finite());
        assert!(
            clip.w <= 0.0,
            "the complete deformed skinned caster must be behind the camera: {clip:?}"
        );
    }

    let receiver_draws = [receiver];
    let receiver_scene = Scene {
        view_proj,
        camera_position: camera.translation,
        camera_rotation: camera.rotation,
        sun: &sun,
        spot_lights: &[],
        point_lights: &[],
        fog: FogSettings {
            color: Vec3::ZERO,
            density: 0.0,
            base_height: 0.0,
            height_falloff: 0.0,
        },
        draws: &receiver_draws,
        particles: &[],
    };
    let control = render_directional_visibility_frame(
        &context,
        &mut renderer,
        &receiver_scene,
        &[],
        (WIDTH, HEIGHT),
    );
    let control_center = packed_rgb(&control, WIDTH, WIDTH / 2, HEIGHT / 2)[0];

    renderer.set_directional_shadow_caster_extrusion_distance(40.0);
    let with_skinned = render_directional_visibility_frame(
        &context,
        &mut renderer,
        &receiver_scene,
        std::slice::from_ref(&skinned_draw),
        (WIDTH, HEIGHT),
    );
    let skinned_center = packed_rgb(&with_skinned, WIDTH, WIDTH / 2, HEIGHT / 2)[0];
    assert!(
        control_center > 220,
        "receiver control should be lit: {control_center}"
    );
    assert!(
        skinned_center + 50 < control_center,
        "non-identity skinned off-frustum caster should shadow receiver: control={control_center}, skinned={skinned_center}"
    );
}

#[test]
#[ignore = "requires a working wgpu adapter; writes high-resolution CSM diagnostic PNGs"]
fn directional_shadow_debug_modes_write_high_resolution_pngs() {
    const WIDTH: u32 = 1280;
    const HEIGHT: u32 = 720;
    let context = pollster::block_on(GpuContext::new_offscreen(WIDTH, HEIGHT))
        .expect("CSM debug screenshot test requires a working wgpu adapter/device");
    let (device, queue) = (context.device(), context.queue());
    let mut renderer = MeshRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
    let cube_model = |position: Vec3, scale: Vec3| {
        Mat4::from_scale_rotation_translation(scale, glam::Quat::IDENTITY, position)
    };
    let mut draws = vec![cube(
        cube_model(Vec3::new(0.0, -0.12, -20.0), Vec3::new(80.0, 0.24, 100.0)),
        [0.72, 0.74, 0.69],
    )];
    for (x, depth, height) in [
        (-2.0, 6.5, 2.8),
        (2.0, 11.0, 3.4),
        (-2.0, 22.0, 4.0),
        (2.0, 42.0, 4.8),
    ] {
        draws.push(cube(
            cube_model(
                Vec3::new(x, height * 0.5, 20.0 - depth),
                Vec3::new(1.4, height, 1.4),
            ),
            [0.17, 0.20, 0.18],
        ));
    }

    let mut skinned_mesh_data = crate::mesh::cube();
    for vertex in &mut skinned_mesh_data.vertices {
        vertex.weights = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    }
    let skinned_mesh = renderer.upload_mesh(
        device,
        &skinned_mesh_data.vertices,
        &skinned_mesh_data.indices,
    );
    let skinned_draw = SkinnedDraw {
        draw: Draw {
            mesh: skinned_mesh,
            material: MeshRenderer::WHITE,
            model: cube_model(Vec3::new(3.0, 1.6, -2.0), Vec3::new(1.6, 3.2, 1.6)),
            tint: [0.24, 0.18, 0.14],
        },
        joint_matrices: Arc::from([Mat4::IDENTITY]),
    };

    let camera = Transform3D::looking_at(
        Vec3::new(0.0, 6.0, 20.0),
        Vec3::new(0.0, 0.0, -20.0),
        Vec3::Y,
    );
    let view = Mat4::from_rotation_translation(camera.rotation, camera.translation).inverse();
    let sun = SunLight {
        direction: Vec3::new(-0.65, -0.65, -0.35).normalize(),
        color: Vec3::splat(1.8),
        ambient: Vec3::splat(0.035),
    };
    let scene = Scene {
        view_proj: Camera3D {
            fov_y: 70.0_f32.to_radians(),
            near: 0.05,
            far: 80.0,
        }
        .projection(WIDTH as f32 / HEIGHT as f32)
            * view,
        camera_position: camera.translation,
        camera_rotation: camera.rotation,
        sun: &sun,
        spot_lights: &[],
        point_lights: &[],
        fog: FogSettings {
            color: Vec3::ZERO,
            density: 0.0,
            base_height: 0.0,
            height_falloff: 0.0,
        },
        draws: &draws,
        particles: &[],
    };

    let output_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&output_dir).expect("create CSM diagnostic output directory");
    {
        let mut write_debug_image = |mode: DirectionalShadowDebugMode, layer: u32, name: &str| {
            renderer.set_directional_shadow_debug(DirectionalShadowDebugOptions {
                mode,
                depth_layer: layer,
            });
            clear_view(device, queue, context.target().view(), [0.0, 0.0, 0.0, 1.0]);
            renderer.render_with_skinning(
                device,
                queue,
                context.target().view(),
                (WIDTH, HEIGHT),
                &scene,
                std::slice::from_ref(&skinned_draw),
            );
            let pixels = context
                .readback()
                .expect("read CSM diagnostic image")
                .pixels;
            let mut rgba = Vec::with_capacity(pixels.len() * 4);
            for packed in pixels {
                rgba.extend_from_slice(&[
                    ((packed >> 16) & 0xff) as u8,
                    ((packed >> 8) & 0xff) as u8,
                    (packed & 0xff) as u8,
                    255,
                ]);
            }
            let output = output_dir.join(name);
            let file = File::create(&output).expect("create CSM diagnostic PNG");
            let mut encoder = png::Encoder::new(file, WIDTH, HEIGHT);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder
                .write_header()
                .expect("write CSM diagnostic PNG header");
            writer
                .write_image_data(&rgba)
                .expect("write CSM diagnostic PNG pixels");
            eprintln!("CSM diagnostic image: {}", output.display());
            assert_eq!(rgba.len(), (WIDTH * HEIGHT * 4) as usize);
        };

        write_debug_image(
            DirectionalShadowDebugMode::CascadeIndex,
            0,
            "csm_debug_cascade_index.png",
        );
        write_debug_image(
            DirectionalShadowDebugMode::BlendWeight,
            0,
            "csm_debug_blend_weight.png",
        );
        for layer in 0..4 {
            let name = format!("csm_debug_depth_layer_{layer}.png");
            write_debug_image(DirectionalShadowDebugMode::DepthLayer, layer, &name);
        }
    }
    renderer.set_directional_shadow_debug(DirectionalShadowDebugOptions::default());
    assert_eq!(
        renderer.directional_shadow_debug(),
        DirectionalShadowDebugOptions::default()
    );
}

#[derive(Clone, Copy, Debug)]
struct ShadowBlendProbe {
    x: u32,
    y: u32,
    current_visibility: f32,
    next_visibility: f32,
    blended_visibility: f32,
    blend_weight: f32,
    expected_visibility: f32,
    mix_error: f32,
    input_delta: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct ShadowBlendProbeScan {
    checked_pixels: usize,
    max_mix_error: f32,
    max_final_mode_error: f32,
    best_probe: Option<ShadowBlendProbe>,
}

fn scan_shadow_blend_components(
    cascade_pixels: &[u32],
    blend_pixels: &[u32],
    component_pixels: &[u32],
    final_visibility_pixels: &[u32],
    width: u32,
    height: u32,
    expected_cascade: usize,
) -> ShadowBlendProbeScan {
    assert_eq!(cascade_pixels.len(), (width * height) as usize);
    assert_eq!(blend_pixels.len(), cascade_pixels.len());
    assert_eq!(component_pixels.len(), cascade_pixels.len());
    assert_eq!(final_visibility_pixels.len(), cascade_pixels.len());

    let mut scan = ShadowBlendProbeScan::default();
    for y in 0..height {
        for x in 0..width {
            let cascade_index = decode_cascade_debug_color(packed_rgb(cascade_pixels, width, x, y));
            if cascade_index != expected_cascade {
                continue;
            }
            let blend_rgb = packed_rgb(blend_pixels, width, x, y);
            let blend_weight = f32::from(blend_rgb[0]) / 255.0;
            if !(0.10..=0.90).contains(&blend_weight) {
                continue;
            }

            let component_rgb = packed_rgb(component_pixels, width, x, y);
            let current_visibility = f32::from(component_rgb[0]) / 255.0;
            let next_visibility = f32::from(component_rgb[1]) / 255.0;
            let blended_visibility = f32::from(component_rgb[2]) / 255.0;
            let expected_visibility =
                current_visibility * (1.0 - blend_weight) + next_visibility * blend_weight;
            let mix_error = (blended_visibility - expected_visibility).abs();
            let final_rgb = packed_rgb(final_visibility_pixels, width, x, y);
            let final_mode_error = (f32::from(final_rgb[0]) / 255.0 - blended_visibility).abs();
            let grayscale_error = f32::from(
                final_rgb[0]
                    .abs_diff(final_rgb[1])
                    .max(final_rgb[1].abs_diff(final_rgb[2])),
            ) / 255.0;
            let input_delta = (current_visibility - next_visibility).abs();

            scan.checked_pixels += 1;
            scan.max_mix_error = scan.max_mix_error.max(mix_error);
            scan.max_final_mode_error = scan
                .max_final_mode_error
                .max(final_mode_error)
                .max(grayscale_error);
            if scan
                .best_probe
                .is_none_or(|probe| input_delta > probe.input_delta)
            {
                scan.best_probe = Some(ShadowBlendProbe {
                    x,
                    y,
                    current_visibility,
                    next_visibility,
                    blended_visibility,
                    blend_weight,
                    expected_visibility,
                    mix_error,
                    input_delta,
                });
            }
        }
    }
    scan
}

#[test]
#[ignore = "requires a working wgpu adapter; writes camera-sweep CSM transition PNGs and numeric CSV"]
fn directional_cascade_transition_camera_sweep_writes_gpu_and_numeric_diagnostics() {
    const WIDTH: u32 = 1280;
    const HEIGHT: u32 = 720;
    const TILE_WIDTH: u32 = 320;
    const TILE_HEIGHT: u32 = 180;
    const SAMPLES_PER_BOUNDARY: usize = 8;
    const MIX_ERROR_TOLERANCE: f32 = 3.0 / 255.0;
    const MIN_CASCADE_VISIBILITY_DELTA: f32 = 0.10;

    let context = pollster::block_on(GpuContext::new_offscreen(WIDTH, HEIGHT))
        .expect("CSM transition sweep requires a working wgpu adapter/device");
    let (device, queue) = (context.device(), context.queue());
    let mut renderer = MeshRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
    let cube_model = |position: Vec3, scale: Vec3| {
        Mat4::from_scale_rotation_translation(scale, glam::Quat::IDENTITY, position)
    };

    // A broad receiver plane plus static casters and one weighted/skinned caster. The probe
    // scans every visible overlap pixel and chooses a point where the two cascade PCF results
    // actually differ; this avoids the old center-pixel probe being fully shadowed in both maps.
    let draws = [
        cube(
            cube_model(Vec3::new(0.0, -0.12, 0.0), Vec3::new(80.0, 0.24, 80.0)),
            [0.70, 0.72, 0.68],
        ),
        cube(
            cube_model(Vec3::new(4.0, 2.0, 2.15), Vec3::new(2.0, 4.0, 2.0)),
            [0.22, 0.24, 0.20],
        ),
        cube(
            cube_model(Vec3::new(-5.0, 1.1, -10.0), Vec3::new(1.8, 2.2, 1.8)),
            [0.30, 0.32, 0.36],
        ),
    ];

    let mut skinned_mesh_data = crate::mesh::cube();
    for vertex in &mut skinned_mesh_data.vertices {
        vertex.weights = [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    }
    let skinned_mesh = renderer.upload_mesh(
        device,
        &skinned_mesh_data.vertices,
        &skinned_mesh_data.indices,
    );
    let skinned_draw = SkinnedDraw {
        draw: Draw {
            mesh: skinned_mesh,
            material: MeshRenderer::WHITE,
            model: cube_model(Vec3::new(6.0, 1.4, -14.0), Vec3::new(1.4, 2.8, 1.4)),
            tint: [0.24, 0.20, 0.30],
        },
        joint_matrices: Arc::from([Mat4::IDENTITY]),
    };

    let target = Vec3::ZERO;
    let camera_forward = Vec3::new(0.0, -6.0, -40.0).normalize();
    let sun = SunLight {
        direction: Vec3::new(-0.65, -0.65, -0.35).normalize(),
        color: Vec3::splat(1.8),
        ambient: Vec3::splat(0.035),
    };
    let camera = Camera3D {
        fov_y: 70.0_f32.to_radians(),
        near: 0.05,
        far: 80.0,
    };
    // This is camera distance to the look-at target, not the view depth of the center-pixel
    // receiver-plane intersection. We solve that mapping instead of assuming they are identical.
    let scene_at_camera_distance = |camera_distance: f32| {
        let camera_position = target - camera_forward * camera_distance;
        let transform = Transform3D::looking_at(camera_position, target, Vec3::Y);
        let view =
            Mat4::from_rotation_translation(transform.rotation, transform.translation).inverse();
        Scene {
            view_proj: camera.projection(WIDTH as f32 / HEIGHT as f32) * view,
            camera_position,
            camera_rotation: transform.rotation,
            sun: &sun,
            spot_lights: &[],
            point_lights: &[],
            fog: FogSettings::default(),
            draws: &draws,
            particles: &[],
        }
    };
    let measured_center_receiver_depth = |camera_distance: f32| {
        let scene = scene_at_camera_distance(camera_distance);
        let receiver_position = world_position_at_pixel_on_horizontal_plane(
            scene.view_proj,
            WIDTH,
            HEIGHT,
            WIDTH / 2,
            HEIGHT / 2,
            0.0,
        );
        let cascades = MeshRenderer::directional_shadow_cascades(&scene);
        (receiver_position - scene.camera_position).dot(cascades.camera_forward)
    };
    // Bisection makes the actual receiver Pixel's measured view depth land on the requested
    // sample. In particular, `after_split` must be on the far side of SplitFar in GPU space.
    let camera_distance_for_receiver_depth = |desired_depth: f32| {
        let mut low = (desired_depth * 0.5).max(0.1);
        let mut high = desired_depth * 1.5 + 0.1;
        let mut low_depth = measured_center_receiver_depth(low);
        let mut high_depth = measured_center_receiver_depth(high);
        for _ in 0..16 {
            if low_depth < desired_depth {
                break;
            }
            low *= 0.5;
            low_depth = measured_center_receiver_depth(low);
        }
        for _ in 0..16 {
            if high_depth > desired_depth {
                break;
            }
            high *= 1.5;
            high_depth = measured_center_receiver_depth(high);
        }
        assert!(
            low_depth < desired_depth && high_depth > desired_depth,
            "could not bracket receiver depth {desired_depth}: low=({low},{low_depth}), high=({high},{high_depth})"
        );
        // Keep the depth values associated with each endpoint. The camera transform and
        // unprojection use f32, so after enough iterations `middle` can round to an endpoint
        // and the requested depth may no longer be representable by another camera distance.
        // Return the representable endpoint whose measured receiver depth is closest to target.
        for _ in 0..48 {
            let middle = (low + high) * 0.5;
            if middle <= low || middle >= high {
                break;
            }
            let middle_depth = measured_center_receiver_depth(middle);
            if middle_depth < desired_depth {
                low = middle;
                low_depth = middle_depth;
            } else {
                high = middle;
                high_depth = middle_depth;
            }
        }
        if (low_depth - desired_depth).abs() <= (high_depth - desired_depth).abs() {
            low
        } else {
            high
        }
    };

    let reference_scene = scene_at_camera_distance(10.0);
    let reference_cascades = MeshRenderer::directional_shadow_cascades(&reference_scene);
    let near_depth = reference_cascades.near_depth;
    let split_depths = reference_cascades.split_depths;
    let blend_fraction = super::shadows::DIRECTIONAL_CASCADE_BLEND;

    let output_dir =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/csm_transition_diagnostics");
    std::fs::create_dir_all(&output_dir).expect("create CSM transition diagnostic directory");

    let mut cascade_tiles = Vec::with_capacity(3 * SAMPLES_PER_BOUNDARY);
    let mut blend_tiles = Vec::with_capacity(3 * SAMPLES_PER_BOUNDARY);
    let mut visibility_tiles = Vec::with_capacity(3 * SAMPLES_PER_BOUNDARY);
    let mut component_tiles = Vec::with_capacity(3 * SAMPLES_PER_BOUNDARY);
    let mut csv = String::from(
        "boundary,sample,sample_name,requested_receiver_depth,camera_target_distance,measured_receiver_depth,receiver_world_x,receiver_world_y,receiver_world_z,expected_cascade_index,gpu_cascade_index,split_near,split_far,blend_start,expected_blend_weight,gpu_blend_weight,center_gpu_shadow_visibility,component_pixels_checked,probe_pixel_x,probe_pixel_y,probe_current_visibility,probe_next_visibility,probe_blended_visibility,probe_blend_weight,probe_expected_visibility,probe_mix_error,probe_input_delta\n",
    );
    let mut metrics = String::new();

    let mut boundary_near = near_depth;
    for (boundary, split_far) in split_depths
        .iter()
        .copied()
        .take(super::shadows::DIRECTIONAL_CASCADE_COUNT - 1)
        .enumerate()
    {
        let split_near = boundary_near;
        let split_span = split_far - split_near;
        let blend_start = split_far - split_span * blend_fraction;
        let blend_width = split_far - blend_start;
        let split_epsilon = (split_span * 0.001).max(0.0002);
        // The bisection operates through f32 camera transforms and pixel-ray unprojection.
        // Its final representable camera distance can therefore miss a requested receiver
        // depth by a small amount. Keep this tolerance well below the before/after split
        // offset so those two samples are still guaranteed to remain on opposite sides.
        let receiver_depth_tolerance = (split_epsilon * 0.2).max(0.0001);
        let samples = [
            ("before_blend", blend_start - blend_width * 0.05),
            ("blend_start", blend_start),
            ("blend_25", blend_start + blend_width * 0.25),
            ("blend_50", blend_start + blend_width * 0.50),
            ("blend_75", blend_start + blend_width * 0.75),
            ("blend_95", blend_start + blend_width * 0.95),
            ("before_split", split_far - split_epsilon),
            ("after_split", split_far + split_epsilon),
        ];
        let mut center_visibility_values = Vec::with_capacity(SAMPLES_PER_BOUNDARY);
        let mut best_boundary_probe: Option<(usize, ShadowBlendProbe)> = None;
        let mut boundary_component_pixels_checked = 0usize;
        let mut boundary_max_mix_error = 0.0_f32;
        let mut boundary_max_final_mode_error = 0.0_f32;

        for (sample_index, (sample_name, requested_receiver_depth)) in
            samples.into_iter().enumerate()
        {
            let camera_distance = camera_distance_for_receiver_depth(requested_receiver_depth);
            let scene = scene_at_camera_distance(camera_distance);
            let active_cascades = MeshRenderer::directional_shadow_cascades(&scene);
            let receiver_position = world_position_at_pixel_on_horizontal_plane(
                scene.view_proj,
                WIDTH,
                HEIGHT,
                WIDTH / 2,
                HEIGHT / 2,
                0.0,
            );
            let measured_view_depth =
                (receiver_position - scene.camera_position).dot(active_cascades.camera_forward);
            let receiver_depth_error = (measured_view_depth - requested_receiver_depth).abs();
            assert!(
                receiver_depth_error <= receiver_depth_tolerance,
                "receiver-depth solver missed target at boundary {boundary}, sample {sample_name}: requested={requested_receiver_depth}, measured={measured_view_depth}, error={receiver_depth_error}, tolerance={receiver_depth_tolerance}, camera_distance={camera_distance}"
            );
            let numeric = super::shadows::directional_cascade_debug_sample(
                measured_view_depth,
                active_cascades.near_depth,
                active_cascades.split_depths,
                blend_fraction,
            );
            let expected_cascade_index = if measured_view_depth > split_far {
                boundary + 1
            } else {
                boundary
            };
            assert_eq!(
                numeric.cascade_index, expected_cascade_index,
                "CPU cascade selection did not match measured depth at boundary {boundary}, sample {sample_name}: depth={measured_view_depth}, split={split_far}"
            );
            if sample_name == "before_split" {
                assert!(measured_view_depth < split_far);
                assert_eq!(expected_cascade_index, boundary);
            }
            if sample_name == "after_split" {
                assert!(measured_view_depth > split_far);
                assert_eq!(expected_cascade_index, boundary + 1);
            }

            let cascade_pixels = render_directional_debug_frame(
                &context,
                &mut renderer,
                &scene,
                std::slice::from_ref(&skinned_draw),
                (WIDTH, HEIGHT),
                DirectionalShadowDebugMode::CascadeIndex,
            );
            let center_rgb = packed_rgb(&cascade_pixels, WIDTH, WIDTH / 2, HEIGHT / 2);
            let gpu_cascade_index = decode_cascade_debug_color(center_rgb);
            assert_eq!(
                gpu_cascade_index, numeric.cascade_index,
                "GPU cascade selection differs at boundary {boundary}, sample {sample_name}: rgb={center_rgb:?}, numeric={numeric:?}"
            );
            if sample_name == "after_split" {
                assert_eq!(
                    gpu_cascade_index,
                    boundary + 1,
                    "GPU center pixel did not enter the next cascade after boundary {boundary}"
                );
            }
            cascade_tiles.push(downsample_rgba(
                &cascade_pixels,
                WIDTH,
                HEIGHT,
                TILE_WIDTH,
                TILE_HEIGHT,
            ));

            let blend_pixels = render_directional_debug_frame(
                &context,
                &mut renderer,
                &scene,
                std::slice::from_ref(&skinned_draw),
                (WIDTH, HEIGHT),
                DirectionalShadowDebugMode::BlendWeight,
            );
            let blend_rgb = packed_rgb(&blend_pixels, WIDTH, WIDTH / 2, HEIGHT / 2);
            let gpu_blend_weight = f32::from(blend_rgb[0]) / 255.0;
            assert!(
                (gpu_blend_weight - numeric.blend_weight).abs() <= 0.01,
                "GPU blend weight differs at boundary {boundary}, sample {sample_name}: rgb={blend_rgb:?}, numeric={numeric:?}"
            );
            blend_tiles.push(downsample_rgba(
                &blend_pixels,
                WIDTH,
                HEIGHT,
                TILE_WIDTH,
                TILE_HEIGHT,
            ));

            let visibility_pixels = render_directional_debug_frame(
                &context,
                &mut renderer,
                &scene,
                std::slice::from_ref(&skinned_draw),
                (WIDTH, HEIGHT),
                DirectionalShadowDebugMode::ShadowVisibility,
            );
            let visibility_rgb = packed_rgb(&visibility_pixels, WIDTH, WIDTH / 2, HEIGHT / 2);
            assert!(
                visibility_rgb[0].abs_diff(visibility_rgb[1]) <= 2
                    && visibility_rgb[1].abs_diff(visibility_rgb[2]) <= 2,
                "shadow-visibility diagnostic must be grayscale: {visibility_rgb:?}"
            );
            center_visibility_values.push(f32::from(visibility_rgb[0]) / 255.0);
            visibility_tiles.push(downsample_rgba(
                &visibility_pixels,
                WIDTH,
                HEIGHT,
                TILE_WIDTH,
                TILE_HEIGHT,
            ));

            // RGB = current PCF / next PCF / actual mix. The weight is read from the BlendWeight
            // pass at the same pixel, because the offscreen readback helper intentionally stores RGB.
            let component_pixels = render_directional_debug_frame(
                &context,
                &mut renderer,
                &scene,
                std::slice::from_ref(&skinned_draw),
                (WIDTH, HEIGHT),
                DirectionalShadowDebugMode::ShadowVisibilityComponents,
            );
            component_tiles.push(downsample_rgba(
                &component_pixels,
                WIDTH,
                HEIGHT,
                TILE_WIDTH,
                TILE_HEIGHT,
            ));
            let scan = scan_shadow_blend_components(
                &cascade_pixels,
                &blend_pixels,
                &component_pixels,
                &visibility_pixels,
                WIDTH,
                HEIGHT,
                boundary,
            );
            boundary_component_pixels_checked += scan.checked_pixels;
            boundary_max_mix_error = boundary_max_mix_error.max(scan.max_mix_error);
            boundary_max_final_mode_error =
                boundary_max_final_mode_error.max(scan.max_final_mode_error);
            assert!(
                scan.max_mix_error <= MIX_ERROR_TOLERANCE,
                "GPU final visibility is not the expected current/next PCF mix at boundary {boundary}, sample {sample_name}: max_error={}, checked={}",
                scan.max_mix_error,
                scan.checked_pixels,
            );
            assert!(
                scan.max_final_mode_error <= 2.0 / 255.0,
                "grayscale ShadowVisibility output differs from the component-mode final mix at boundary {boundary}, sample {sample_name}: max_error={}",
                scan.max_final_mode_error,
            );
            if let Some(probe) = scan.best_probe {
                if best_boundary_probe
                    .as_ref()
                    .is_none_or(|(_, best)| probe.input_delta > best.input_delta)
                {
                    best_boundary_probe = Some((sample_index, probe));
                }
            }

            if sample_index == 3 {
                write_rgba_png(
                    &output_dir.join(format!("boundary_{boundary}_mid_cascade_index.png")),
                    WIDTH,
                    HEIGHT,
                    &rgba_from_packed_pixels(&cascade_pixels),
                );
                write_rgba_png(
                    &output_dir.join(format!("boundary_{boundary}_mid_blend_weight.png")),
                    WIDTH,
                    HEIGHT,
                    &rgba_from_packed_pixels(&blend_pixels),
                );
                write_rgba_png(
                    &output_dir.join(format!("boundary_{boundary}_mid_shadow_visibility.png")),
                    WIDTH,
                    HEIGHT,
                    &rgba_from_packed_pixels(&visibility_pixels),
                );
                write_rgba_png(
                    &output_dir.join(format!(
                        "boundary_{boundary}_mid_shadow_visibility_components.png"
                    )),
                    WIDTH,
                    HEIGHT,
                    &rgba_from_packed_pixels(&component_pixels),
                );
            }

            let probe_csv = match scan.best_probe {
                Some(probe) => format!(
                    "{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}",
                    probe.x,
                    probe.y,
                    probe.current_visibility,
                    probe.next_visibility,
                    probe.blended_visibility,
                    probe.blend_weight,
                    probe.expected_visibility,
                    probe.mix_error,
                    probe.input_delta,
                ),
                None => "-1,-1,nan,nan,nan,nan,nan,nan,nan".to_owned(),
            };
            csv.push_str(&format!(
                "{boundary},{sample_index},{sample_name},{requested_receiver_depth:.6},{camera_distance:.6},{measured_view_depth:.6},{:.6},{:.6},{:.6},{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{probe_csv}\n",
                receiver_position.x,
                receiver_position.y,
                receiver_position.z,
                numeric.cascade_index,
                gpu_cascade_index,
                numeric.split_near,
                numeric.split_far,
                numeric.blend_start,
                numeric.blend_weight,
                gpu_blend_weight,
                center_visibility_values[sample_index],
                scan.checked_pixels,
            ));
        }

        let min_center_visibility = center_visibility_values
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        let max_center_visibility = center_visibility_values
            .iter()
            .copied()
            .fold(f32::NEG_INFINITY, f32::max);
        let max_center_adjacent_delta = center_visibility_values
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .fold(0.0_f32, f32::max);
        let (probe_sample, probe) = best_boundary_probe
            .expect("each cascade overlap must expose a measurable current/next PCF probe");
        assert!(
            probe.input_delta >= MIN_CASCADE_VISIBILITY_DELTA,
            "boundary {boundary} did not produce a non-degenerate current/next cascade PCF pair: max_delta={} at pixel ({}, {})",
            probe.input_delta,
            probe.x,
            probe.y,
        );
        assert!(
            boundary_component_pixels_checked > 0,
            "boundary {boundary} produced no overlap pixels for component validation"
        );
        assert!(
            boundary_max_mix_error <= MIX_ERROR_TOLERANCE,
            "boundary {boundary} has an invalid final PCF mix: max_error={boundary_max_mix_error}"
        );
        assert!(
            boundary_max_final_mode_error <= 2.0 / 255.0,
            "boundary {boundary} has a mismatch between grayscale visibility and component output: max_error={boundary_max_final_mode_error}"
        );
        metrics.push_str(&format!(
            "Boundary {boundary}: split_near={split_near:.5}, split_far={split_far:.5}, blend_start={blend_start:.5}, blend_width={blend_width:.5}, center_visibility_min={min_center_visibility:.4}, center_visibility_max={max_center_visibility:.4}, center_max_adjacent_delta={max_center_adjacent_delta:.4}, component_pixels_checked={boundary_component_pixels_checked}, max_mix_error={boundary_max_mix_error:.6}, max_final_mode_error={boundary_max_final_mode_error:.6}, edge_probe_sample={probe_sample}, edge_probe_pixel=({},{}), edge_probe_current={:.4}, edge_probe_next={:.4}, edge_probe_final={:.4}, edge_probe_weight={:.4}, edge_probe_expected={:.4}, edge_probe_input_delta={:.4}, edge_probe_mix_error={:.6}, status=BOUNDARY_CROSSED_AND_PCF_MIX_VALIDATED\n",
            probe.x,
            probe.y,
            probe.current_visibility,
            probe.next_visibility,
            probe.blended_visibility,
            probe.blend_weight,
            probe.expected_visibility,
            probe.input_delta,
            probe.mix_error,
        ));
        boundary_near = split_far;
    }

    write_contact_sheet(
        &output_dir.join("cascade_index_camera_sweep.png"),
        &cascade_tiles,
        SAMPLES_PER_BOUNDARY as u32,
        TILE_WIDTH,
        TILE_HEIGHT,
    );
    write_contact_sheet(
        &output_dir.join("blend_weight_camera_sweep.png"),
        &blend_tiles,
        SAMPLES_PER_BOUNDARY as u32,
        TILE_WIDTH,
        TILE_HEIGHT,
    );
    write_contact_sheet(
        &output_dir.join("shadow_visibility_camera_sweep.png"),
        &visibility_tiles,
        SAMPLES_PER_BOUNDARY as u32,
        TILE_WIDTH,
        TILE_HEIGHT,
    );
    write_contact_sheet(
        &output_dir.join("shadow_visibility_components_camera_sweep.png"),
        &component_tiles,
        SAMPLES_PER_BOUNDARY as u32,
        TILE_WIDTH,
        TILE_HEIGHT,
    );
    std::fs::write(output_dir.join("numeric_trace.csv"), csv)
        .expect("write CSM numeric transition trace");
    let metadata = format!(
        "Ribbon Engine CSM dynamic transition diagnostic\n\
         Render resolution: {WIDTH}x{HEIGHT}\n\
         Contact-sheet tile resolution: {TILE_WIDTH}x{TILE_HEIGHT}\n\
         Rows: transition boundaries 0, 1, 2 in order.\n\
         Columns: before blend, blend start, 25%, 50%, 75%, 95%, before split, after split.\n\
         Camera position is solved by bisection so the measured center-pixel receiver-plane view\n\
         depth equals each requested sample. Each after_split sample must select the next cascade.\n\
         `cascade_index_camera_sweep.png` shows the GPU-selected cascade.\n\
         `blend_weight_camera_sweep.png` shows the GPU fragment blend weight.\n\
         `shadow_visibility_camera_sweep.png` shows final blended 3x3-PCF visibility.\n\
         `shadow_visibility_components_camera_sweep.png` stores current PCF in R, next PCF in G,\n\
         and the actual final mix in B. The test scans every eligible overlap pixel, checks the mix\n\
         equation within 3/255 quantization tolerance, and requires a per-boundary edge probe with\n\
         a current/next visibility difference of at least {MIN_CASCADE_VISIBILITY_DELTA:.2}.\n\
         Center-pixel visibility is context only; it is not used by itself to claim that blending works.\n\
         {metrics}"
    );
    std::fs::write(output_dir.join("README.txt"), metadata)
        .expect("write CSM transition diagnostic metadata");

    renderer.set_directional_shadow_debug(DirectionalShadowDebugOptions::default());
    assert_eq!(
        renderer.directional_shadow_debug(),
        DirectionalShadowDebugOptions::default()
    );
    eprintln!(
        "CSM dynamic transition diagnostics: {}",
        output_dir.display()
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
