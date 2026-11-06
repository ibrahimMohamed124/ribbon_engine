use super::*;

/// The directional shadow allocation is fixed at construction time. Keep this value in sync
/// with `renderer/setup.rs`; texel snapping uses the actual texture resolution.
pub(super) const DIRECTIONAL_SHADOW_RESOLUTION: u32 = 1024;
/// Four camera-depth ranges share one array texture; each layer is rendered independently.
pub(super) const DIRECTIONAL_CASCADE_COUNT: usize = 4;
/// Practical split blend between logarithmic and linear camera-depth partitions.
const DIRECTIONAL_CASCADE_LAMBDA: f32 = 0.65;
/// Blend the final ten percent of a cascade into the next cascade to hide split seams.
pub(super) const DIRECTIONAL_CASCADE_BLEND: f32 = 0.10;
/// Quantize the fitted sphere radius to suppress floating-point scale changes when the same
/// camera frustum is translated or rotated. This stabilizes the world-space shadow texel grid.
const DIRECTIONAL_RADIUS_QUANTIZATION: f32 = 16.0;
/// Quantize the light-space depth center on a conservative world-space grid. The same
/// quantum is used for the radius so the depth interval can include half a cell of guard.
const DIRECTIONAL_DEPTH_CENTER_QUANTIZATION: f32 = DIRECTIONAL_RADIUS_QUANTIZATION;

#[derive(Clone, Copy, Debug)]
pub(super) struct DirectionalShadowCascades {
    pub(super) matrices: [Mat4; DIRECTIONAL_CASCADE_COUNT],
    /// Positive camera-space distance to the far edge of each cascade.
    pub(super) split_depths: [f32; DIRECTIONAL_CASCADE_COUNT],
    pub(super) near_depth: f32,
    pub(super) camera_forward: Vec3,
}

/// Test-only numeric reference for the selection performed by `directional_cascade_selection` in WGSL.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DirectionalCascadeDebugSample {
    pub(super) view_depth: f32,
    pub(super) cascade_index: usize,
    pub(super) split_near: f32,
    pub(super) split_far: f32,
    pub(super) blend_start: f32,
    pub(super) blend_weight: f32,
}

/// Mirror the shader's depth selection and final `blend_fraction` interval on the CPU.
#[cfg(test)]
pub(super) fn directional_cascade_debug_sample(
    view_depth: f32,
    near_depth: f32,
    split_depths: [f32; DIRECTIONAL_CASCADE_COUNT],
    blend_fraction: f32,
) -> DirectionalCascadeDebugSample {
    let depth = if view_depth.is_finite() {
        view_depth
    } else {
        0.0
    };
    let mut cascade_index = 0;
    for (index, split) in split_depths
        .iter()
        .copied()
        .enumerate()
        .take(DIRECTIONAL_CASCADE_COUNT - 1)
    {
        if depth > split {
            cascade_index = index + 1;
        }
    }

    let split_near = if cascade_index == 0 {
        near_depth
    } else {
        split_depths[cascade_index - 1]
    };
    let split_far = split_depths[cascade_index];
    let blend_fraction = if blend_fraction.is_finite() {
        blend_fraction.clamp(0.0, 0.25)
    } else {
        0.0
    };
    let blend_start = if cascade_index < DIRECTIONAL_CASCADE_COUNT - 1 {
        split_far - (split_far - split_near) * blend_fraction
    } else {
        split_far
    };
    let blend_weight = if cascade_index < DIRECTIONAL_CASCADE_COUNT - 1 && depth > blend_start {
        ((depth - blend_start) / (split_far - blend_start).max(0.0001)).clamp(0.0, 1.0)
    } else {
        0.0
    };

    DirectionalCascadeDebugSample {
        view_depth: depth,
        cascade_index,
        split_near,
        split_far,
        blend_start,
        blend_weight,
    }
}

fn frustum_corners(view_proj: Mat4) -> Option<[Vec3; 8]> {
    if !view_proj
        .to_cols_array()
        .iter()
        .all(|value| value.is_finite())
    {
        return None;
    }
    let determinant = view_proj.determinant();
    if !determinant.is_finite() || determinant.abs() <= 1.0e-10 {
        return None;
    }
    let inverse = view_proj.inverse();
    if !inverse
        .to_cols_array()
        .iter()
        .all(|value| value.is_finite())
    {
        return None;
    }

    let mut corners = [Vec3::ZERO; 8];
    let mut index = 0;
    // Ribbon uses the wgpu/glam right-handed 0..1 depth convention.
    for z in [0.0_f32, 1.0] {
        for y in [-1.0_f32, 1.0] {
            for x in [-1.0_f32, 1.0] {
                let homogeneous = inverse * Vec4::new(x, y, z, 1.0);
                if !homogeneous.is_finite() || homogeneous.w.abs() <= 1.0e-8 {
                    return None;
                }
                let point = homogeneous.truncate() / homogeneous.w;
                if !point.is_finite() {
                    return None;
                }
                corners[index] = point;
                index += 1;
            }
        }
    }
    Some(corners)
}

fn camera_frustum_depth_range(scene: &Scene<'_>, corners: &[Vec3; 8]) -> Option<(Vec3, f32, f32)> {
    let near_center_offset = average_offset_from_origin(scene.camera_position, &corners[..4]);
    let far_center_offset = average_offset_from_origin(scene.camera_position, &corners[4..]);
    let camera_forward = (far_center_offset - near_center_offset).normalize_or_zero();
    if !camera_forward.is_finite() || camera_forward.length_squared() < 0.5 {
        return None;
    }
    let near_depth = stable_light_projection(near_center_offset, camera_forward);
    let far_depth = stable_light_projection(far_center_offset, camera_forward);
    if !near_depth.is_finite()
        || !far_depth.is_finite()
        || near_depth < 0.0
        || far_depth <= near_depth + 0.1
    {
        return None;
    }
    Some((camera_forward, near_depth, far_depth))
}

pub(super) fn finite_direction(direction: Vec3) -> Vec3 {
    let normalized = direction.normalize_or_zero();
    if normalized.is_finite() && normalized.length_squared() > 0.5 {
        normalized
    } else {
        Vec3::new(-0.4, -0.8, -0.45).normalize()
    }
}

/// Stable orthonormal basis shared by cascade fitting and texel-grid diagnostics.
pub(super) fn directional_light_basis(direction: Vec3) -> Option<(Vec3, Vec3)> {
    let reference_up = if direction.dot(Vec3::Y).abs() > 0.95 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let right = direction.cross(reference_up).normalize_or_zero();
    let up = right.cross(direction).normalize_or_zero();
    if right.is_finite()
        && up.is_finite()
        && right.length_squared() >= 0.5
        && up.length_squared() >= 0.5
    {
        Some((right, up))
    } else {
        None
    }
}

/// Return the orthographic half extent and the world-space size of one shadow texel.
/// Keep the 1.5-texel guard band in one place so tests use the exact runtime grid.
/// Quantize the fitted sphere radius once and reuse it for both XY extent and Z padding.
/// Otherwise the raw f32 radius can leave the XY texel size unchanged while still perturbing
/// the depth padding and the complete light-space matrix.
fn quantized_directional_radius(radius: f32) -> f32 {
    let raw_radius = radius.max(0.5);
    if raw_radius.is_finite() && raw_radius <= f32::MAX / DIRECTIONAL_RADIUS_QUANTIZATION {
        (raw_radius * DIRECTIONAL_RADIUS_QUANTIZATION).round() / DIRECTIONAL_RADIUS_QUANTIZATION
            + 0.5 / DIRECTIONAL_RADIUS_QUANTIZATION
    } else {
        raw_radius
    }
}

fn directional_shadow_extent(radius: f32) -> (f32, f32) {
    // The enclosing sphere is rotation invariant in exact arithmetic, but inverse-view-projection
    // corner reconstruction can perturb its f32 radius by a few thousandths of a world unit.
    // Because the snap grid is anchored at world-space zero, even a tiny texel-size change can
    // shift distant grid coordinates by a visible fraction of a texel. The conservative half-step
    // guard makes the quantized radius safe for either rounding direction.
    let base_half_extent = quantized_directional_radius(radius);
    let nominal_texel = (2.0 * base_half_extent) / DIRECTIONAL_SHADOW_RESOLUTION as f32;
    let half_extent = base_half_extent + nominal_texel * 1.5;
    let texel_size = (2.0 * half_extent) / DIRECTIONAL_SHADOW_RESOLUTION as f32;
    (half_extent, texel_size)
}

fn directional_shadow_depth_padding(radius: f32) -> f32 {
    (quantized_directional_radius(radius) * 0.08).max(1.0)
}

fn snap_light_space_coordinate(coordinate: f32, texel_size: f32) -> f32 {
    if coordinate.is_finite() && texel_size.is_finite() && texel_size > 0.0 {
        (coordinate / texel_size).round() * texel_size
    } else {
        coordinate
    }
}

/// Compute a stable centroid offset without summing large absolute world-space coordinates.
/// For a pure translation, subtracting the camera origin first keeps the frustum geometry in a
/// small, camera-relative range and avoids translation-dependent cancellation in the sum.
fn average_offset_from_origin(origin: Vec3, points: &[Vec3]) -> Vec3 {
    points.iter().map(|point| *point - origin).sum::<Vec3>() / points.len().max(1) as f32
}

/// Evaluate a dot product in f64 after receiving the f32 world-space inputs. This is especially
/// useful for camera-origin projections: a light-plane translation should not perturb the depth
/// anchor due only to f32 accumulation error.
fn stable_light_projection(value: Vec3, axis: Vec3) -> f32 {
    ((value.x as f64 * axis.x as f64)
        + (value.y as f64 * axis.y as f64)
        + (value.z as f64 * axis.z as f64)) as f32
}

/// Snap a camera-relative light-space depth coordinate to a fixed grid.
fn quantized_directional_depth_center(center_depth_offset: f32) -> f32 {
    let quantum = DIRECTIONAL_DEPTH_CENTER_QUANTIZATION;
    if center_depth_offset.is_finite() && center_depth_offset.abs() <= f32::MAX / quantum {
        (center_depth_offset * quantum).round() / quantum
    } else {
        center_depth_offset
    }
}

/// Build a conservative, stable light-space Z interval from the fitted bounding sphere.
/// The centroid's camera-relative depth offset is quantized independently of the camera origin.
/// A pure light-plane camera translation therefore leaves the quantized offset unchanged, while
/// the origin projection remains invariant because it is orthogonal to the light direction.
/// The outward-quantized radius encloses the sphere, and the half-quantum guard covers the
/// displacement introduced by snapping the relative center.
fn directional_shadow_depth_range(
    center_offset: Vec3,
    depth_origin: Vec3,
    radius: f32,
    light_direction: Vec3,
) -> (f32, f32) {
    let origin_depth = stable_light_projection(depth_origin, light_direction);
    let relative_center_depth =
        quantized_directional_depth_center(stable_light_projection(center_offset, light_direction));
    let center_depth = origin_depth + relative_center_depth;
    let depth_guard = 0.5 / DIRECTIONAL_DEPTH_CENTER_QUANTIZATION;
    let depth_half_extent = quantized_directional_radius(radius) + depth_guard;
    if !center_depth.is_finite() || !depth_half_extent.is_finite() || depth_half_extent <= 0.0 {
        return (center_depth, center_depth);
    }

    (
        center_depth - depth_half_extent,
        center_depth + depth_half_extent,
    )
}

/// Project a world-space AABB into the directional-light basis. All eight corners are
/// transformed explicitly so rotated, scaled, and sheared bounds remain conservative.
pub(super) fn directional_light_space_bounds(
    bounds: Aabb3,
    right: Vec3,
    up: Vec3,
    direction: Vec3,
) -> Option<Aabb3> {
    Aabb3::from_points(bounds.corners().into_iter().map(|point| {
        Vec3::new(
            stable_light_projection(point, right),
            stable_light_projection(point, up),
            stable_light_projection(point, direction),
        )
    }))
}

/// Round a lower light-space depth bound outward. This is deliberately floor-based rather than
/// nearest rounding: quantization must never move the near plane past a relevant caster.
fn outward_quantized_directional_depth_min(depth: f32) -> f32 {
    let quantum = DIRECTIONAL_DEPTH_CENTER_QUANTIZATION as f64;
    if !depth.is_finite() {
        return depth;
    }
    let rounded = ((depth as f64 * quantum).floor() / quantum) as f32;
    if rounded.is_finite() && rounded <= depth {
        rounded
    } else {
        depth
    }
}

/// Extend only the lower depth edge for upstream casters whose projected XY bounds can overlap
/// this cascade's receiver footprint. Inputs are already projected onto the shared light basis;
/// upstream casters therefore have lower light-space Z than their receiver surfaces.
struct DirectionalCasterDepthInput<'a> {
    receiver_min_depth: f32,
    nearest_receiver_depth: f32,
    snapped_x: f32,
    snapped_y: f32,
    half_extent: f32,
    texel_size: f32,
    light_space_caster_bounds: &'a [Aabb3],
    max_extrusion_distance: f32,
}

fn directional_caster_depth_extension(input: DirectionalCasterDepthInput<'_>) -> (f32, usize) {
    let DirectionalCasterDepthInput {
        receiver_min_depth,
        nearest_receiver_depth,
        snapped_x,
        snapped_y,
        half_extent,
        texel_size,
        light_space_caster_bounds,
        max_extrusion_distance,
    } = input;
    if !receiver_min_depth.is_finite()
        || !nearest_receiver_depth.is_finite()
        || !snapped_x.is_finite()
        || !snapped_y.is_finite()
        || !half_extent.is_finite()
        || !texel_size.is_finite()
        || !max_extrusion_distance.is_finite()
        || max_extrusion_distance <= 0.0
        || light_space_caster_bounds.is_empty()
    {
        return (receiver_min_depth, 0);
    }

    // The existing receiver fit already has a 1.5-texel guard. Include one additional texel in
    // the *selection* footprint for the 3x3 PCF edge taps; this does not enlarge the XY projection.
    let selection_half_extent = half_extent + texel_size;
    let min_x = snapped_x - selection_half_extent;
    let max_x = snapped_x + selection_half_extent;
    let min_y = snapped_y - selection_half_extent;
    let max_y = snapped_y + selection_half_extent;
    let extrusion_near = nearest_receiver_depth - max_extrusion_distance;
    if !min_x.is_finite()
        || !max_x.is_finite()
        || !min_y.is_finite()
        || !max_y.is_finite()
        || !extrusion_near.is_finite()
    {
        return (receiver_min_depth, 0);
    }

    let mut extended_min = receiver_min_depth;
    let mut selected = 0_usize;
    for light_bounds in light_space_caster_bounds.iter().copied() {
        if !light_bounds.is_valid() {
            continue;
        }
        let overlaps_xy = light_bounds.max.x >= min_x
            && light_bounds.min.x <= max_x
            && light_bounds.max.y >= min_y
            && light_bounds.min.y <= max_y;
        if !overlaps_xy {
            continue;
        }

        // A caster must have some geometry upstream of the nearest receiver depth and within
        // the configured extrusion band. Bounds fully behind the receiver or too far upstream
        // are negative controls and must not affect depth precision.
        if light_bounds.min.z >= nearest_receiver_depth || light_bounds.max.z < extrusion_near {
            continue;
        }
        let relevant_min = light_bounds.min.z.max(extrusion_near);
        if relevant_min.is_finite() {
            selected = selected.saturating_add(1);
            if relevant_min < extended_min {
                extended_min = relevant_min;
            }
        }
    }

    if extended_min < receiver_min_depth {
        (
            outward_quantized_directional_depth_min(extended_min),
            selected,
        )
    } else {
        (receiver_min_depth, 0)
    }
}

/// Build a conservative model-local pose AABB from precomputed joint influence bounds. The WGSL
/// path forms `sum(weight * joint_matrix * position)` and applies the instance model matrix once
/// afterward. With non-negative normalized weights this is inside the union of the transformed
/// joint bounds; a small permitted weight-sum deviation is conservatively handled by including
/// the local origin for sums below one and scaling outward for sums above one.
fn skinned_pose_local_bounds(
    joint_bounds: &[(u16, Aabb3)],
    unweighted_bounds: Option<Aabb3>,
    min_weight_sum: f32,
    max_weight_sum: f32,
    palette: &[Mat4],
) -> Option<Aabb3> {
    if !min_weight_sum.is_finite()
        || !max_weight_sum.is_finite()
        || min_weight_sum < 0.0
        || max_weight_sum > 1.001
    {
        return None;
    }

    let mut result: Option<Aabb3> = None;
    for (joint_index, influence_bounds) in joint_bounds {
        let joint_matrix = *palette.get(usize::from(*joint_index))?;
        let transformed = influence_bounds.transformed(joint_matrix)?;
        if let Some(result) = &mut result {
            result.include_bounds(transformed);
        } else {
            result = Some(transformed);
        }
    }

    if let Some(mut skinned_bounds) = result {
        if min_weight_sum < 1.0 {
            skinned_bounds.include_point(Vec3::ZERO);
        }
        if max_weight_sum > 1.0 {
            skinned_bounds = skinned_bounds.scaled_from_origin(max_weight_sum)?;
        }
        result = Some(skinned_bounds);
    }

    if let Some(unweighted) = unweighted_bounds {
        if let Some(result) = &mut result {
            result.include_bounds(unweighted);
        } else {
            result = Some(unweighted);
        }
    }
    result
}

fn practical_cascade_splits(near: f32, far: f32) -> [f32; DIRECTIONAL_CASCADE_COUNT] {
    std::array::from_fn(|index| {
        let fraction = (index + 1) as f32 / DIRECTIONAL_CASCADE_COUNT as f32;
        if index + 1 == DIRECTIONAL_CASCADE_COUNT {
            far
        } else {
            let logarithmic = near * (far / near).powf(fraction);
            let uniform = near + (far - near) * fraction;
            DIRECTIONAL_CASCADE_LAMBDA * logarithmic + (1.0 - DIRECTIONAL_CASCADE_LAMBDA) * uniform
        }
    })
}

fn split_frustum_corners(
    full_frustum: &[Vec3; 8],
    near_depth: f32,
    far_depth: f32,
    slice_near: f32,
    slice_far: f32,
) -> [Vec3; 8] {
    let depth_span = (far_depth - near_depth).max(1.0e-5);
    let near_t = ((slice_near - near_depth) / depth_span).clamp(0.0, 1.0);
    let far_t = ((slice_far - near_depth) / depth_span).clamp(near_t, 1.0);
    let mut result = [Vec3::ZERO; 8];
    for (corner, (near_corner, far_corner)) in full_frustum[..4]
        .iter()
        .zip(full_frustum[4..].iter())
        .enumerate()
    {
        result[corner] = near_corner.lerp(*far_corner, near_t);
        result[corner + 4] = near_corner.lerp(*far_corner, far_t);
    }
    result
}

#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub(super) struct DirectionalCascadeSnapDebug {
    pub(super) center: Vec3,
    pub(super) right: Vec3,
    pub(super) up: Vec3,
    pub(super) center_x: f32,
    pub(super) center_y: f32,
    pub(super) snapped_x: f32,
    pub(super) snapped_y: f32,
    pub(super) texel_size: f32,
    pub(super) depth_min: f32,
    pub(super) depth_max: f32,
    pub(super) depth_padding: f32,
}

/// Inspect the exact world-space snap grid used by one runtime cascade.
/// This mirrors the runtime's frustum slice and calls the same basis/extent/snap helpers.
#[cfg(test)]
pub(super) fn directional_cascade_snap_debug(
    scene: &Scene<'_>,
    cascade_index: usize,
) -> Option<DirectionalCascadeSnapDebug> {
    if cascade_index >= DIRECTIONAL_CASCADE_COUNT {
        return None;
    }
    let full_frustum = frustum_corners(scene.view_proj)?;
    let (_camera_forward, near_depth, far_depth) =
        camera_frustum_depth_range(scene, &full_frustum)?;
    let split_depths = practical_cascade_splits(near_depth.max(0.01), far_depth);
    let cascade_end = split_depths[cascade_index];
    let cascade_start = if cascade_index == 0 {
        near_depth
    } else {
        let previous = split_depths[cascade_index - 1];
        previous - (cascade_end - previous) * DIRECTIONAL_CASCADE_BLEND
    };
    let slice = split_frustum_corners(
        &full_frustum,
        near_depth,
        far_depth,
        cascade_start.max(near_depth),
        cascade_end,
    );
    let center_offset = average_offset_from_origin(scene.camera_position, &slice);
    let center = scene.camera_position + center_offset;
    let radius = slice
        .iter()
        .map(|corner| (*corner - scene.camera_position).distance(center_offset))
        .fold(0.0_f32, f32::max);
    if !center.is_finite() || !center_offset.is_finite() || !radius.is_finite() || radius <= 1.0e-4
    {
        return None;
    }
    let direction = finite_direction(scene.sun.direction);
    let (right, up) = directional_light_basis(direction)?;
    let (_half_extent, texel_size) = directional_shadow_extent(radius);
    let center_x = stable_light_projection(scene.camera_position, right)
        + stable_light_projection(center_offset, right);
    let center_y = stable_light_projection(scene.camera_position, up)
        + stable_light_projection(center_offset, up);
    let (depth_min, depth_max) =
        directional_shadow_depth_range(center_offset, scene.camera_position, radius, direction);
    Some(DirectionalCascadeSnapDebug {
        center,
        right,
        up,
        center_x,
        center_y,
        snapped_x: snap_light_space_coordinate(center_x, texel_size),
        snapped_y: snap_light_space_coordinate(center_y, texel_size),
        texel_size,
        depth_min,
        depth_max,
        depth_padding: directional_shadow_depth_padding(radius),
    })
}

impl MeshRenderer {
    /// Ensure the main color pass has a depth target matching the current drawable size.
    /// Recreate it only when the surface resolution changes; the TextureView keeps the
    /// underlying wgpu texture alive for the lifetime of the depth target.
    pub(super) fn ensure_depth(
        &mut self,
        device: &wgpu::Device,
        size: (u32, u32),
        sample_count: u32,
    ) {
        if self
            .depth
            .as_ref()
            .is_some_and(|depth| depth.size == size && depth.sample_count == sample_count)
        {
            return;
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("mesh depth"),
            size: wgpu::Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.depth = Some(DepthTarget {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            size,
            sample_count,
        });
        self.update_peak_resource_estimate();
    }

    /// Legacy full-frustum reference kept only for regression tests; runtime uses cascades.
    #[cfg(test)]
    pub(super) fn directional_shadow_matrix(scene: &Scene<'_>) -> Mat4 {
        let direction = finite_direction(scene.sun.direction);
        let Some(corners) = frustum_corners(scene.view_proj) else {
            return Self::fallback_directional_shadow_matrix(scene, direction);
        };
        Self::fit_directional_shadow_matrix(scene, &corners, direction)
    }

    /// Build four stable, overlapping camera-frustum slices for cascaded directional shadows.
    /// The split distances use a practical logarithmic/linear blend and every cascade is fitted
    /// independently with a rotation-invariant sphere and world-space texel snapping.
    #[cfg(test)]
    pub(super) fn directional_shadow_cascades(scene: &Scene<'_>) -> DirectionalShadowCascades {
        Self::directional_shadow_cascades_with_casters(scene, &[], 0.0)
    }

    /// Build receiver-fitted cascades and extend only their light-space depth for relevant
    /// upstream shadow casters. XY fitting and snapping remain receiver-only by design.
    pub(super) fn directional_shadow_cascades_with_casters(
        scene: &Scene<'_>,
        caster_bounds: &[Aabb3],
        max_extrusion_distance: f32,
    ) -> DirectionalShadowCascades {
        let direction = finite_direction(scene.sun.direction);
        let Some((right, up)) = directional_light_basis(direction) else {
            let fallback = Self::fallback_directional_shadow_matrix(scene, direction);
            return Self::fallback_cascades(scene, fallback);
        };
        let light_space_caster_bounds = caster_bounds
            .iter()
            .copied()
            .filter_map(|bounds| directional_light_space_bounds(bounds, right, up, direction))
            .collect::<Vec<_>>();
        Self::directional_shadow_cascades_with_light_space_casters(
            scene,
            &light_space_caster_bounds,
            max_extrusion_distance,
        )
    }

    /// Same cascade builder with caster bounds already projected into the shared light basis.
    /// The render path prepares these bounds once per frame, avoiding four repeated projections.
    pub(super) fn directional_shadow_cascades_with_light_space_casters(
        scene: &Scene<'_>,
        light_space_caster_bounds: &[Aabb3],
        max_extrusion_distance: f32,
    ) -> DirectionalShadowCascades {
        let fallback_direction = finite_direction(scene.sun.direction);
        let fallback_matrix = Self::fallback_directional_shadow_matrix(scene, fallback_direction);
        let Some(corners) = frustum_corners(scene.view_proj) else {
            return Self::fallback_cascades(scene, fallback_matrix);
        };

        let Some((camera_forward, near_depth, far_depth)) =
            camera_frustum_depth_range(scene, &corners)
        else {
            return Self::fallback_cascades(scene, fallback_matrix);
        };

        let split_depths = practical_cascade_splits(near_depth.max(0.01), far_depth);
        let direction = fallback_direction;
        let mut matrices = [fallback_matrix; DIRECTIONAL_CASCADE_COUNT];
        for (cascade_index, cascade_end) in split_depths.iter().copied().enumerate() {
            let cascade_start = if cascade_index == 0 {
                near_depth
            } else {
                let previous = split_depths[cascade_index - 1];
                previous - (cascade_end - previous) * DIRECTIONAL_CASCADE_BLEND
            };
            let slice = split_frustum_corners(
                &corners,
                near_depth,
                far_depth,
                cascade_start.max(near_depth),
                cascade_end,
            );
            matrices[cascade_index] = Self::fit_directional_shadow_matrix_with_casters(
                scene,
                &slice,
                direction,
                light_space_caster_bounds,
                max_extrusion_distance,
            );
        }

        DirectionalShadowCascades {
            matrices,
            split_depths,
            near_depth,
            camera_forward,
        }
    }

    fn fallback_cascades(scene: &Scene<'_>, fallback_matrix: Mat4) -> DirectionalShadowCascades {
        let camera_forward = if scene.camera_rotation.is_finite() {
            (scene.camera_rotation * Vec3::NEG_Z).normalize_or_zero()
        } else {
            Vec3::NEG_Z
        };
        let camera_forward = if camera_forward.is_finite() && camera_forward.length_squared() > 0.5
        {
            camera_forward
        } else {
            Vec3::NEG_Z
        };
        let near_depth = 0.05;
        let far_depth = 48.0;
        let split_depths = std::array::from_fn(|index| {
            near_depth
                + (far_depth - near_depth) * (index + 1) as f32 / DIRECTIONAL_CASCADE_COUNT as f32
        });
        DirectionalShadowCascades {
            matrices: [fallback_matrix; DIRECTIONAL_CASCADE_COUNT],
            split_depths,
            near_depth,
            camera_forward,
        }
    }

    #[cfg(test)]
    fn fit_directional_shadow_matrix(
        scene: &Scene<'_>,
        corners: &[Vec3; 8],
        direction: Vec3,
    ) -> Mat4 {
        Self::fit_directional_shadow_matrix_with_casters(scene, corners, direction, &[], 0.0)
    }

    fn fit_directional_shadow_matrix_with_casters(
        scene: &Scene<'_>,
        corners: &[Vec3; 8],
        direction: Vec3,
        light_space_caster_bounds: &[Aabb3],
        max_extrusion_distance: f32,
    ) -> Mat4 {
        let Some((right, up)) = directional_light_basis(direction) else {
            return Self::fallback_directional_shadow_matrix(scene, direction);
        };

        let center_offset = average_offset_from_origin(scene.camera_position, corners);
        let center = scene.camera_position + center_offset;
        let radius = corners
            .iter()
            .map(|corner| (*corner - scene.camera_position).distance(center_offset))
            .fold(0.0_f32, f32::max);
        if !center.is_finite()
            || !center_offset.is_finite()
            || !radius.is_finite()
            || radius <= 1.0e-4
        {
            return Self::fallback_directional_shadow_matrix(scene, direction);
        }

        // A sphere keeps XY scale stable as the camera rotates. Build its centroid relative to
        // the camera so a light-plane translation cancels before the light-space projections.
        let (half_extent, texel_size) = directional_shadow_extent(radius);
        let center_x = stable_light_projection(scene.camera_position, right)
            + stable_light_projection(center_offset, right);
        let center_y = stable_light_projection(scene.camera_position, up)
            + stable_light_projection(center_offset, up);
        let snapped_x = snap_light_space_coordinate(center_x, texel_size);
        let snapped_y = snap_light_space_coordinate(center_y, texel_size);

        // Use the same quantized bounding sphere that establishes the XY extent. Its radius
        // conservatively encloses every fitted corner along Z as well, and avoids unstable
        // independent min/max quantization when a corner lies on a fixed depth-grid boundary.
        let (receiver_min_depth, max_depth) =
            directional_shadow_depth_range(center_offset, scene.camera_position, radius, direction);
        let nearest_receiver_depth = corners
            .iter()
            .map(|corner| stable_light_projection(*corner, direction))
            .fold(f32::INFINITY, f32::min);
        let (min_depth, _selected_caster_count) =
            directional_caster_depth_extension(DirectionalCasterDepthInput {
                receiver_min_depth,
                nearest_receiver_depth,
                snapped_x,
                snapped_y,
                half_extent,
                texel_size,
                light_space_caster_bounds,
                max_extrusion_distance,
            });
        if !min_depth.is_finite() || !max_depth.is_finite() || max_depth <= min_depth {
            return Self::fallback_directional_shadow_matrix(scene, direction);
        }

        let depth_padding = directional_shadow_depth_padding(radius);
        let eye = right * snapped_x + up * snapped_y + direction * (min_depth - depth_padding);
        let view = Mat4::look_to_rh(eye, direction, up);
        let near = 0.05;
        let far = (max_depth - min_depth + depth_padding * 2.0).max(near + 0.1);
        let projection = Mat4::orthographic_rh(
            -half_extent,
            half_extent,
            -half_extent,
            half_extent,
            near,
            far,
        );
        let matrix = projection * view;
        if matrix.to_cols_array().iter().all(|value| value.is_finite()) {
            matrix
        } else {
            Self::fallback_directional_shadow_matrix(scene, direction)
        }
    }

    /// Build conservative world-space bounds for one member of the exact accepted shadow-draw
    /// list. Skinned draws use the current accepted model-local palette; bind-pose bounds are
    /// never substituted when palette data is missing or invalid.
    pub(super) fn directional_shadow_caster_bounds(
        &self,
        prepared: &PreparedDraw,
    ) -> Option<Aabb3> {
        let slot = self.meshes.get(prepared.draw.mesh.slot)?;
        if slot.generation != prepared.draw.mesh.generation {
            return None;
        }
        let mesh = slot.mesh.as_ref()?;
        let local_bounds = match mesh.vertex_layout {
            MeshVertexLayout::Static => mesh.local_bounds?,
            MeshVertexLayout::Skinned => {
                if let Some(palette) = prepared.palette.as_deref() {
                    if !mesh.skin_bounds_valid {
                        return None;
                    }
                    skinned_pose_local_bounds(
                        &mesh.joint_influence_bounds,
                        mesh.unweighted_bounds,
                        mesh.min_skin_weight_sum,
                        mesh.max_skin_weight_sum,
                        palette,
                    )?
                } else {
                    // A skinned mesh submitted without its palette is not safely bounded by
                    // its bind pose if any vertex has positive weights. Do not silently use the
                    // bind-pose AABB. Fully unweighted legacy streams remain static geometry.
                    if !mesh.skin_bounds_valid || !mesh.joint_influence_bounds.is_empty() {
                        return None;
                    }
                    mesh.local_bounds?
                }
            }
        };
        local_bounds.transformed(prepared.draw.model)
    }

    fn fallback_directional_shadow_matrix(scene: &Scene<'_>, direction: Vec3) -> Mat4 {
        // Defensive fallback for a singular/invalid camera matrix. The normal path always uses
        // the extracted frustum; this prevents malformed camera data from poisoning GPU uniforms.
        let focus = if scene.camera_position.is_finite() {
            scene.camera_position
        } else {
            Vec3::ZERO
        };
        let radius = 48.0;
        let eye = focus - direction * radius;
        let up = if direction.dot(Vec3::Y).abs() > 0.95 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.1, radius * 4.0)
            * Mat4::look_at_rh(eye, focus, up)
    }

    pub(super) fn spot_shadow_matrix(light: SpotLight) -> Mat4 {
        let direction = light.direction.normalize_or_zero();
        let direction = if direction.length_squared() > 0.5 {
            direction
        } else {
            Vec3::NEG_Z
        };
        let up = if direction.dot(Vec3::Y).abs() > 0.95 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let half_angle = light.outer_cone_angle.clamp(0.05, 1.48);
        Mat4::perspective_rh(half_angle * 2.0, 1.0, 0.05, light.range.max(0.1))
            * Mat4::look_at_rh(light.position, light.position + direction, up)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        average_offset_from_origin, directional_cascade_debug_sample,
        directional_cascade_snap_debug, directional_caster_depth_extension,
        directional_light_basis, directional_light_space_bounds, directional_shadow_depth_range,
        directional_shadow_extent, finite_direction, frustum_corners,
        outward_quantized_directional_depth_min, practical_cascade_splits,
        skinned_pose_local_bounds, snap_light_space_coordinate, split_frustum_corners,
        stable_light_projection, DirectionalCascadeSnapDebug, DirectionalCasterDepthInput,
        DIRECTIONAL_CASCADE_BLEND, DIRECTIONAL_CASCADE_COUNT, DIRECTIONAL_RADIUS_QUANTIZATION,
        DIRECTIONAL_SHADOW_RESOLUTION,
    };
    use crate::components::{Camera3D, FogSettings, SunLight, Transform3D};
    use crate::renderer::{Aabb3, MeshRenderer, Scene};
    use glam::{Mat4, Vec3};

    fn projection_at(position: Vec3, target: Vec3) -> Mat4 {
        let direction = (target - position).normalize();
        let view = Mat4::look_to_rh(position, direction, Vec3::Y);
        Camera3D::default().projection(16.0 / 9.0) * view
    }

    #[derive(Clone, Copy)]
    struct FullFrustumSnapProbe {
        right: Vec3,
        up: Vec3,
        center_x: f32,
        center_y: f32,
        snapped_x: f32,
        snapped_y: f32,
        texel_size: f32,
        radius: f32,
        quantized_radius: f32,
    }

    fn full_frustum_snap_probe(scene: &Scene<'_>) -> FullFrustumSnapProbe {
        let corners = frustum_corners(scene.view_proj).expect("valid full-frustum probe");
        let center_offset = average_offset_from_origin(scene.camera_position, &corners);
        let radius = corners
            .iter()
            .map(|corner| (*corner - scene.camera_position).distance(center_offset))
            .fold(0.0_f32, f32::max);
        let direction = finite_direction(scene.sun.direction);
        let (right, up) = directional_light_basis(direction).expect("valid light basis");
        let (_, texel_size) = directional_shadow_extent(radius);
        // Match the runtime's camera-relative projection order exactly. A direct dot of the
        // reconstructed absolute centroid can differ by a few f32 ULPs from this sum.
        let center_x = stable_light_projection(scene.camera_position, right)
            + stable_light_projection(center_offset, right);
        let center_y = stable_light_projection(scene.camera_position, up)
            + stable_light_projection(center_offset, up);
        FullFrustumSnapProbe {
            right,
            up,
            center_x,
            center_y,
            snapped_x: snap_light_space_coordinate(center_x, texel_size),
            snapped_y: snap_light_space_coordinate(center_y, texel_size),
            texel_size,
            radius,
            quantized_radius: super::quantized_directional_radius(radius),
        }
    }

    fn full_frustum_scene<'a>(eye: Vec3, target: Vec3, sun: &'a SunLight) -> Scene<'a> {
        Scene {
            view_proj: projection_at(eye, target),
            camera_position: eye,
            camera_rotation: glam::Quat::IDENTITY,
            sun,
            spot_lights: &[],
            point_lights: &[],
            fog: FogSettings::default(),
            draws: &[],
            particles: &[],
        }
    }

    /// Align the legacy full-frustum regression probe against the measured runtime snap grid.
    /// A fixed 0.1-texel translation from an arbitrary starting pose can cross a rounding
    /// boundary; in that case the test measures a legitimate one-texel snap, not in-cell jitter.
    fn align_full_frustum_to_phase(
        initial_eye: Vec3,
        initial_target: Vec3,
        sun: &SunLight,
        target_x: f32,
        target_y: f32,
    ) -> (Vec3, Vec3, FullFrustumSnapProbe) {
        let mut eye = initial_eye;
        let mut target = initial_target;
        for _ in 0..32 {
            let scene = full_frustum_scene(eye, target, sun);
            let probe = full_frustum_snap_probe(&scene);
            let x_fraction = (probe.center_x - probe.snapped_x) / probe.texel_size;
            let y_fraction = (probe.center_y - probe.snapped_y) / probe.texel_size;
            let error_x = target_x - x_fraction;
            let error_y = target_y - y_fraction;
            if error_x.abs().max(error_y.abs()) <= 2.0e-5 {
                return (eye, target, probe);
            }
            let correction = probe.right * (error_x * probe.texel_size * 0.5)
                + probe.up * (error_y * probe.texel_size * 0.5);
            eye += correction;
            target += correction;
        }

        let scene = full_frustum_scene(eye, target, sun);
        let probe = full_frustum_snap_probe(&scene);
        let x_fraction = (probe.center_x - probe.snapped_x) / probe.texel_size;
        let y_fraction = (probe.center_y - probe.snapped_y) / probe.texel_size;
        assert!(
            (x_fraction - target_x).abs() <= 0.02
                && (y_fraction - target_y).abs() <= 0.02,
            "full-frustum probe alignment failed: measured=({x_fraction}, {y_fraction}), target=({target_x}, {target_y})"
        );
        (eye, target, probe)
    }

    #[test]
    fn fitted_directional_matrix_covers_every_active_camera_frustum_corner() {
        let eye = Vec3::new(8.0, 5.0, 12.0);
        let view_proj = projection_at(eye, Vec3::new(0.0, 1.0, 0.0));
        let corners = frustum_corners(view_proj).expect("valid perspective frustum");
        let sun = SunLight::default();
        let scene = Scene {
            view_proj,
            camera_position: eye,
            camera_rotation: glam::Quat::IDENTITY,
            sun: &sun,
            spot_lights: &[],
            point_lights: &[],
            fog: FogSettings::default(),
            draws: &[],
            particles: &[],
        };
        let matrix = MeshRenderer::directional_shadow_matrix(&scene);
        assert!(matrix.to_cols_array().iter().all(|value| value.is_finite()));
        for corner in corners {
            let clip = matrix * corner.extend(1.0);
            assert!(clip.is_finite() && clip.w.abs() > 1.0e-7);
            let ndc = clip.truncate() / clip.w;
            assert!(
                ndc.x.abs() <= 1.001,
                "frustum corner outside shadow X: {ndc:?}"
            );
            assert!(
                ndc.y.abs() <= 1.001,
                "frustum corner outside shadow Y: {ndc:?}"
            );
            assert!(
                ndc.z >= -0.001 && ndc.z <= 1.001,
                "frustum corner outside shadow Z: {ndc:?}"
            );
        }
    }

    #[test]
    fn directional_shadow_projection_is_stable_under_sub_texel_camera_translation() {
        let eye = Vec3::new(0.0, 2.0, 8.0);
        let target = Vec3::new(0.0, 1.0, 0.0);
        let sun = SunLight::default();

        // Place both probes at measured fractions safely inside the same runtime snap cell.
        // This preserves the intended assertion (sub-texel movement does not alter the matrix)
        // without accidentally starting within 0.1 texel of a rounding boundary.
        let (base_eye, base_target, base_probe) =
            align_full_frustum_to_phase(eye, target, &sun, 0.15, 0.15);
        let base_scene = full_frustum_scene(base_eye, base_target, &sun);
        let base_matrix = MeshRenderer::directional_shadow_matrix(&base_scene);

        let base_x_fraction = (base_probe.center_x - base_probe.snapped_x) / base_probe.texel_size;
        let base_y_fraction = (base_probe.center_y - base_probe.snapped_y) / base_probe.texel_size;

        // Do not re-solve the second pose against a second measured grid. That makes the
        // measurement basis itself move while testing one-cell stability. Translate the
        // already-aligned camera rigidly by exactly 0.20 of the baseline texel instead.
        // The expected phase is therefore about 0.35, with 0.15 texel of margin to the
        // nearest rounding boundary on either side.
        let displacement = base_probe.right * (base_probe.texel_size * 0.20);
        let moved_eye = base_eye + displacement;
        let moved_target = base_target + displacement;
        let moved_scene = full_frustum_scene(moved_eye, moved_target, &sun);
        let moved_probe = full_frustum_snap_probe(&moved_scene);
        let moved_matrix = MeshRenderer::directional_shadow_matrix(&moved_scene);
        let moved_x_fraction =
            (moved_probe.center_x - moved_probe.snapped_x) / moved_probe.texel_size;
        let moved_y_fraction =
            (moved_probe.center_y - moved_probe.snapped_y) / moved_probe.texel_size;

        let snap_delta_x = (base_probe.snapped_x - moved_probe.snapped_x).abs();
        let snap_delta_y = (base_probe.snapped_y - moved_probe.snapped_y).abs();
        let same_snap_cell = snap_delta_x <= base_probe.texel_size * 1.0e-4
            && snap_delta_y <= base_probe.texel_size * 1.0e-4;
        assert!(
            same_snap_cell,
            "sub-texel probe changed snap cell: base_phase=({base_x_fraction:.6}, {base_y_fraction:.6}), moved_phase=({moved_x_fraction:.6}, {moved_y_fraction:.6}), base_center=({:.9}, {:.9}), moved_center=({:.9}, {:.9}), base_snap=({:.9}, {:.9}), moved_snap=({:.9}, {:.9}), base_texel={:.9}, moved_texel={:.9}, base_radius={:.9}, moved_radius={:.9}, base_quantized_radius={:.9}, moved_quantized_radius={:.9}, snap_delta=({snap_delta_x:.9}, {snap_delta_y:.9})",
            base_probe.center_x,
            base_probe.center_y,
            moved_probe.center_x,
            moved_probe.center_y,
            base_probe.snapped_x,
            base_probe.snapped_y,
            moved_probe.snapped_x,
            moved_probe.snapped_y,
            base_probe.texel_size,
            moved_probe.texel_size,
            base_probe.radius,
            moved_probe.radius,
            base_probe.quantized_radius,
            moved_probe.quantized_radius,
        );
        assert_eq!(base_probe.texel_size, moved_probe.texel_size);

        let max_delta = base_matrix
            .to_cols_array()
            .into_iter()
            .zip(moved_matrix.to_cols_array())
            .map(|(left, right)| (left - right).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            max_delta <= 1.0e-5,
            "sub-texel movement inside a verified snap cell changed shadow matrix by {max_delta}; base_phase=({base_x_fraction}, {base_y_fraction}), moved_phase=({moved_x_fraction}, {moved_y_fraction})"
        );
    }

    #[test]
    fn practical_cascade_splits_are_monotonic_and_reach_camera_far_plane() {
        let splits = practical_cascade_splits(0.1, 250.0);
        assert!(splits.as_slice().windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(splits[DIRECTIONAL_CASCADE_COUNT - 1], 250.0);
        assert!(splits[0] < 250.0 / DIRECTIONAL_CASCADE_COUNT as f32);
    }

    #[test]
    fn numeric_cascade_debug_samples_match_the_shader_transition_contract() {
        let near = 0.05;
        let splits = practical_cascade_splits(near, 80.0);
        let blend_fraction = DIRECTIONAL_CASCADE_BLEND;

        let mut split_near = near;
        for (cascade_index, split_far) in splits
            .iter()
            .copied()
            .take(DIRECTIONAL_CASCADE_COUNT - 1)
            .enumerate()
        {
            let span = split_far - split_near;
            let blend_start = split_far - span * blend_fraction;
            let blend_width = split_far - blend_start;

            let before = directional_cascade_debug_sample(
                blend_start - blend_width * 0.05,
                near,
                splits,
                blend_fraction,
            );
            assert_eq!(before.cascade_index, cascade_index);
            assert_eq!(before.blend_weight, 0.0);

            let at_start =
                directional_cascade_debug_sample(blend_start, near, splits, blend_fraction);
            assert_eq!(at_start.cascade_index, cascade_index);
            assert!(at_start.blend_weight.abs() <= 1.0e-6);

            let mut previous_weight = 0.0;
            for fraction in [0.25, 0.5, 0.95] {
                let sample = directional_cascade_debug_sample(
                    blend_start + blend_width * fraction,
                    near,
                    splits,
                    blend_fraction,
                );
                assert_eq!(sample.cascade_index, cascade_index);
                assert!(sample.blend_weight > previous_weight);
                assert!((sample.blend_weight - fraction).abs() <= 1.0e-4);
                assert_eq!(sample.split_near, split_near);
                assert_eq!(sample.split_far, split_far);
                assert!((sample.blend_start - blend_start).abs() <= 1.0e-5);
                previous_weight = sample.blend_weight;
            }

            let after = directional_cascade_debug_sample(
                split_far + span * 0.005,
                near,
                splits,
                blend_fraction,
            );
            assert_eq!(after.cascade_index, cascade_index + 1);
            assert_eq!(after.blend_weight, 0.0);
            split_near = split_far;
        }
    }

    #[test]
    fn next_cascade_fit_covers_the_shader_transition_window() {
        for (near, far) in [(0.1, 250.0), (0.05, 48.0), (1.0, 10_000.0)] {
            let splits = practical_cascade_splits(near, far);
            let mut current_start = near;
            for (cascade_index, (current_end, next_end)) in splits
                .iter()
                .copied()
                .zip(splits.iter().copied().skip(1))
                .enumerate()
            {
                let current_width = current_end - current_start;
                let next_width = next_end - current_end;
                let shader_blend_start = current_end - current_width * DIRECTIONAL_CASCADE_BLEND;
                let next_cascade_fit_start = current_end - next_width * DIRECTIONAL_CASCADE_BLEND;
                assert!(
                    next_cascade_fit_start <= shader_blend_start,
                    "next cascade does not cover transition for split {cascade_index}: fit starts at {next_cascade_fit_start}, blend starts at {shader_blend_start}"
                );
                current_start = current_end;
            }
        }
    }

    #[test]
    fn every_directional_cascade_matrix_covers_its_overlapped_frustum_slice() {
        let eye = Vec3::new(8.0, 5.0, 12.0);
        let view_proj = projection_at(eye, Vec3::new(0.0, 1.0, 0.0));
        let corners = frustum_corners(view_proj).expect("valid perspective frustum");
        let sun = SunLight::default();
        let scene = Scene {
            view_proj,
            camera_position: eye,
            camera_rotation: glam::Quat::IDENTITY,
            sun: &sun,
            spot_lights: &[],
            point_lights: &[],
            fog: FogSettings::default(),
            draws: &[],
            particles: &[],
        };
        let cascades = MeshRenderer::directional_shadow_cascades(&scene);
        assert!(cascades
            .split_depths
            .as_slice()
            .windows(2)
            .all(|pair| pair[0] < pair[1]));
        assert!(cascades.split_depths[DIRECTIONAL_CASCADE_COUNT - 1] > 0.0);
        for (cascade_index, (end, matrix)) in cascades
            .split_depths
            .iter()
            .copied()
            .zip(cascades.matrices.iter().copied())
            .enumerate()
        {
            let start = if cascade_index == 0 {
                cascades.near_depth
            } else {
                let previous = cascades.split_depths[cascade_index - 1];
                previous - (end - previous) * DIRECTIONAL_CASCADE_BLEND
            };
            let slice = split_frustum_corners(
                &corners,
                cascades.near_depth,
                cascades.split_depths[DIRECTIONAL_CASCADE_COUNT - 1],
                start.max(cascades.near_depth),
                end,
            );
            assert!(matrix.to_cols_array().iter().all(|value| value.is_finite()));
            for corner in slice {
                let clip = matrix * corner.extend(1.0);
                assert!(clip.is_finite() && clip.w.abs() > 1.0e-7);
                let ndc = clip.truncate() / clip.w;
                assert!(ndc.x.abs() <= 1.001, "cascade {cascade_index} X: {ndc:?}");
                assert!(ndc.y.abs() <= 1.001, "cascade {cascade_index} Y: {ndc:?}");
                assert!(
                    ndc.z >= -0.001 && ndc.z <= 1.001,
                    "cascade {cascade_index} Z: {ndc:?}"
                );
            }
        }
    }

    #[test]
    fn invalid_view_projection_produces_four_finite_fallback_cascades() {
        let sun = SunLight::default();
        let scene = Scene {
            view_proj: Mat4::ZERO,
            camera_position: Vec3::ZERO,
            camera_rotation: glam::Quat::IDENTITY,
            sun: &sun,
            spot_lights: &[],
            point_lights: &[],
            fog: FogSettings::default(),
            draws: &[],
            particles: &[],
        };
        let cascades = MeshRenderer::directional_shadow_cascades(&scene);
        assert!(cascades
            .split_depths
            .as_slice()
            .windows(2)
            .all(|pair| pair[0] < pair[1]));
        assert!(cascades
            .matrices
            .iter()
            .all(|matrix| matrix.to_cols_array().iter().all(|value| value.is_finite())));
    }

    fn scene_for_pose(eye: Vec3, rotation: glam::Quat, sun: &SunLight) -> Scene<'_> {
        let view = Mat4::from_rotation_translation(rotation, eye).inverse();
        let camera = Camera3D {
            fov_y: 70.0_f32.to_radians(),
            near: 0.05,
            far: 80.0,
        };
        Scene {
            view_proj: camera.projection(16.0 / 9.0) * view,
            camera_position: eye,
            camera_rotation: rotation,
            sun,
            spot_lights: &[],
            point_lights: &[],
            fog: FogSettings::default(),
            draws: &[],
            particles: &[],
        }
    }

    fn shadow_pixel_position(matrix: Mat4, world_position: Vec3) -> glam::Vec2 {
        let clip = matrix * world_position.extend(1.0);
        assert!(clip.is_finite() && clip.w.abs() > 1.0e-7);
        let ndc = clip.truncate() / clip.w;
        glam::Vec2::new(
            (ndc.x * 0.5 + 0.5) * DIRECTIONAL_SHADOW_RESOLUTION as f32,
            (0.5 - ndc.y * 0.5) * DIRECTIONAL_SHADOW_RESOLUTION as f32,
        )
    }

    fn matrix_max_delta(a: Mat4, b: Mat4) -> f32 {
        a.to_cols_array()
            .into_iter()
            .zip(b.to_cols_array())
            .map(|(left, right)| (left - right).abs())
            .fold(0.0_f32, f32::max)
    }

    /// Iteratively place a camera pose near a requested fraction of the *measured runtime* snap
    /// cell. The final fraction is read back from the runtime helper and is used as the actual
    /// baseline by the test; f32 inverse-view-projection reconstruction can leave a small residual
    /// that should not be mistaken for a snap-grid failure.
    fn align_camera_to_snap_fraction(
        initial_eye: Vec3,
        rotation: glam::Quat,
        sun: &SunLight,
        cascade_index: usize,
        target_x_fraction: f32,
        target_y_fraction: f32,
    ) -> (Vec3, DirectionalCascadeSnapDebug) {
        let mut eye = initial_eye;
        const ALIGNMENT_DAMPING: f32 = 0.5;
        for _ in 0..24 {
            let scene = scene_for_pose(eye, rotation, sun);
            let sample = directional_cascade_snap_debug(&scene, cascade_index)
                .expect("valid cascade snap parameters while aligning test camera");
            let x_fraction = (sample.center_x - sample.snapped_x) / sample.texel_size;
            let y_fraction = (sample.center_y - sample.snapped_y) / sample.texel_size;
            let error_x = target_x_fraction - x_fraction;
            let error_y = target_y_fraction - y_fraction;
            if error_x.abs().max(error_y.abs()) <= 2.0e-5 {
                return (eye, sample);
            }
            eye += sample.right * (error_x * sample.texel_size * ALIGNMENT_DAMPING)
                + sample.up * (error_y * sample.texel_size * ALIGNMENT_DAMPING);
        }

        let scene = scene_for_pose(eye, rotation, sun);
        let sample = directional_cascade_snap_debug(&scene, cascade_index)
            .expect("valid final cascade snap parameters after alignment");
        let x_fraction = (sample.center_x - sample.snapped_x) / sample.texel_size;
        let y_fraction = (sample.center_y - sample.snapped_y) / sample.texel_size;
        const MAX_ALIGNMENT_RESIDUAL: f32 = 0.02;
        assert!(
            (x_fraction - target_x_fraction).abs() <= MAX_ALIGNMENT_RESIDUAL
                && (y_fraction - target_y_fraction).abs() <= MAX_ALIGNMENT_RESIDUAL,
            "cascade {cascade_index} could not be aligned near snap-cell fraction ({target_x_fraction}, {target_y_fraction}); measured=({x_fraction}, {y_fraction}), allowed_residual={MAX_ALIGNMENT_RESIDUAL}"
        );
        (eye, sample)
    }

    #[test]
    fn directional_shadow_depth_range_is_conservative_and_stable_under_small_f32_noise() {
        let direction = finite_direction(Vec3::new(-0.65, -0.65, -0.35));
        let (right, up) = directional_light_basis(direction).expect("stable light basis");
        let center = Vec3::new(12.25, -8.5, 19.75);
        let moved_center = center + right * 0.003 + up * 0.002;
        let radius = 10.933_824_f32;
        let moved_radius = 10.930_577_f32;

        let base = directional_shadow_depth_range(center, Vec3::ZERO, radius, direction);
        let moved =
            directional_shadow_depth_range(moved_center, Vec3::ZERO, moved_radius, direction);
        let base_half_extent = (base.1 - base.0) * 0.5;
        let moved_half_extent = (moved.1 - moved.0) * 0.5;
        assert_eq!(
            super::quantized_directional_radius(radius),
            super::quantized_directional_radius(moved_radius)
        );
        assert!((base_half_extent - moved_half_extent).abs() <= 2.0e-5);
        assert_eq!(
            base, moved,
            "light-space depth interval must remain identical under a small XY-only camera move"
        );

        // Every point in the fitted sphere must remain inside the light-space depth interval.
        for offset in [
            Vec3::X * radius,
            -Vec3::X * radius,
            Vec3::Y * radius,
            -Vec3::Y * radius,
            Vec3::Z * radius,
            -Vec3::Z * radius,
            Vec3::new(1.0, 1.0, 1.0).normalize() * radius,
        ] {
            let depth = (center + offset).dot(direction);
            assert!(depth >= base.0 - 1.0e-5);
            assert!(depth <= base.1 + 1.0e-5);
        }
    }

    #[test]
    fn directional_shadow_depth_center_is_stable_for_small_gpu_probe_drift() {
        // Observed on the RTX 2050 Vulkan probe: XY texel snapping stayed fixed, but inverse-VP
        // reconstruction moved the raw light-space depth center by about 8.28e-4 world units.
        // Both values must resolve to the same 1/16-world-unit center cell.
        let base_center_depth = -1.377_031_3_f32;
        let moved_center_depth = -1.376_203_5_f32;
        let base_quantized = super::quantized_directional_depth_center(base_center_depth);
        let moved_quantized = super::quantized_directional_depth_center(moved_center_depth);
        assert_eq!(base_quantized, -1.375);
        assert_eq!(base_quantized, moved_quantized);

        let direction = finite_direction(Vec3::new(-0.65, -0.65, -0.35));
        let radius = 10.933_824_f32;
        let base_range = directional_shadow_depth_range(
            direction * base_center_depth,
            Vec3::ZERO,
            radius,
            direction,
        );
        let moved_range = directional_shadow_depth_range(
            direction * moved_center_depth,
            Vec3::ZERO,
            radius,
            direction,
        );
        assert_eq!(base_range, moved_range);

        // The added half-quantum guard plus the outward-quantized radius must still contain
        // every point in the unquantized fitted sphere along the light direction.
        let half_extent = (base_range.1 - base_range.0) * 0.5;
        let center_error = (base_quantized - base_center_depth).abs();
        assert!(center_error <= 0.5 / super::DIRECTIONAL_DEPTH_CENTER_QUANTIZATION + 1.0e-6);
        assert!(
            half_extent >= radius + center_error - 1.0e-5,
            "quantized depth interval must conservatively contain the original sphere"
        );
    }

    #[test]
    fn directional_shadow_depth_range_is_stable_for_light_plane_origin_translation() {
        let direction = finite_direction(Vec3::new(-0.65, -0.65, -0.35));
        let (right, up) = directional_light_basis(direction).expect("stable light basis");
        let origin = Vec3::new(25.0, -1.2, 4.2);
        let moved_origin = origin + right * 0.031 + up * 0.047;
        let center_offset = Vec3::new(0.75, -1.25, 8.5);
        let radius = 10.0;

        let base = directional_shadow_depth_range(center_offset, origin, radius, direction);
        let moved = directional_shadow_depth_range(center_offset, moved_origin, radius, direction);
        const DEPTH_RANGE_FLOAT_TOLERANCE: f32 = 5.0e-6;
        assert!(
            (base.0 - moved.0).abs() <= DEPTH_RANGE_FLOAT_TOLERANCE
                && (base.1 - moved.1).abs() <= DEPTH_RANGE_FLOAT_TOLERANCE,
            "light-plane origin translation changed the camera-relative depth interval: base={base:?}, moved={moved:?}, tolerance={DEPTH_RANGE_FLOAT_TOLERANCE}"
        );
    }

    #[test]
    fn directional_shadow_depth_padding_uses_the_same_quantized_radius_as_xy_extent() {
        let base_radius = 10.933_824_f32;
        let moved_radius = 10.930_577_f32;
        assert_eq!(
            super::directional_shadow_extent(base_radius),
            super::directional_shadow_extent(moved_radius)
        );
        assert_eq!(
            super::directional_shadow_depth_padding(base_radius),
            super::directional_shadow_depth_padding(moved_radius)
        );
    }

    #[test]
    fn directional_shadow_extent_keeps_texel_scale_stable_under_small_radius_noise() {
        // These radii are representative of the small f32 variation observed when an unchanged
        // frustum is reconstructed after a sub-texel camera move or a tiny camera rotation. Both
        // values must quantize to the same conservative base radius and final guarded extent.
        let raw_radius = 10.933824_f32;
        let moved_radius = 10.930577_f32;
        let (base_extent, base_texel) = super::directional_shadow_extent(raw_radius);
        let (moved_extent, moved_texel) = super::directional_shadow_extent(moved_radius);
        assert_eq!(base_extent, moved_extent);
        assert_eq!(base_texel, moved_texel);

        let quantized_base = (raw_radius * DIRECTIONAL_RADIUS_QUANTIZATION).round()
            / DIRECTIONAL_RADIUS_QUANTIZATION
            + 0.5 / DIRECTIONAL_RADIUS_QUANTIZATION;
        let nominal_texel = (2.0 * quantized_base) / DIRECTIONAL_SHADOW_RESOLUTION as f32;
        let expected_guarded_extent = quantized_base + nominal_texel * 1.5;
        assert!((base_extent - expected_guarded_extent).abs() <= 1.0e-6);
        assert!(base_extent >= raw_radius);
        assert!(base_extent >= moved_radius);
    }

    #[test]
    fn every_cascade_shadow_matrix_is_stable_inside_its_sub_texel_cell() {
        let eye = Vec3::new(0.0, 6.0, 20.0);
        let target = Vec3::new(0.0, 0.0, -20.0);
        let sun = SunLight {
            direction: Vec3::new(-0.65, -0.65, -0.35).normalize(),
            ..SunLight::default()
        };
        let initial_rotation = Transform3D::looking_at(eye, target, Vec3::Y).rotation;

        for cascade_index in 0..DIRECTIONAL_CASCADE_COUNT {
            // Use feedback against the measured runtime grid rather than a single analytic
            // correction. Keep a wider margin from the half-texel rounding boundary so f32
            // inverse-projection noise cannot make an intended in-cell sample ambiguous.
            const ALIGNED_FRACTION: f32 = 0.15;
            let (aligned_eye, aligned) = align_camera_to_snap_fraction(
                eye,
                initial_rotation,
                &sun,
                cascade_index,
                ALIGNED_FRACTION,
                ALIGNED_FRACTION,
            );
            let aligned_scene = scene_for_pose(aligned_eye, initial_rotation, &sun);
            let aligned_matrix =
                MeshRenderer::directional_shadow_cascades(&aligned_scene).matrices[cascade_index];
            // The solver's final measured fractions are authoritative. Use them as the baseline
            // so each perturbation is relative to the actual f32 runtime coordinate rather than
            // an idealized target that may not be exactly representable at this world position.
            let base_x_fraction = (aligned.center_x - aligned.snapped_x) / aligned.texel_size;
            let base_y_fraction = (aligned.center_y - aligned.snapped_y) / aligned.texel_size;
            assert!(
                (base_x_fraction - ALIGNED_FRACTION).abs() <= 0.02,
                "cascade {cascade_index} X cell fraction={base_x_fraction}"
            );
            assert!(
                (base_y_fraction - ALIGNED_FRACTION).abs() <= 0.02,
                "cascade {cascade_index} Y cell fraction={base_y_fraction}"
            );

            // Probe two positions that are safely inside the same quantization cell. The 0.42
            // target keeps at least 0.08 texel of margin from the half-texel boundary.
            for (fraction_x, fraction_y) in [
                (0.25, base_y_fraction),
                (0.42, base_y_fraction),
                (base_x_fraction, 0.25),
                (base_x_fraction, 0.42),
                (0.42, 0.42),
            ] {
                let delta = aligned.right * ((fraction_x - base_x_fraction) * aligned.texel_size)
                    + aligned.up * ((fraction_y - base_y_fraction) * aligned.texel_size);
                let sample_scene = scene_for_pose(aligned_eye + delta, initial_rotation, &sun);
                let sample = directional_cascade_snap_debug(&sample_scene, cascade_index)
                    .expect("sub-texel cascade snap parameters");
                assert!(
                    (sample.snapped_x - aligned.snapped_x).abs() <= aligned.texel_size * 1.0e-4
                );
                assert!(
                    (sample.snapped_y - aligned.snapped_y).abs() <= aligned.texel_size * 1.0e-4
                );
                assert_eq!(sample.texel_size, aligned.texel_size);
                // The sphere half-extent and padding are quantized and must remain identical.
                // The center may carry tiny f32 dot-product noise under an orthogonal translation;
                // the complete matrix assertion below is the strict stability gate for that.
                assert!(
                    ((sample.depth_max - sample.depth_min)
                        - (aligned.depth_max - aligned.depth_min))
                        .abs()
                        <= 2.0e-5,
                    "cascade {cascade_index} depth span drifted inside snap cell"
                );
                assert_eq!(sample.depth_padding, aligned.depth_padding);
                let sample_matrix = MeshRenderer::directional_shadow_cascades(&sample_scene)
                    .matrices[cascade_index];
                let delta = matrix_max_delta(aligned_matrix, sample_matrix);
                assert!(
                    delta <= 2.0e-5,
                    "cascade {cascade_index} changed matrix inside snap cell at ({fraction_x:.2}, {fraction_y:.2}): max_delta={delta}"
                );
            }

            // Move from the measured in-cell position to 0.65 texel in light-space X. This is
            // deliberately beyond the 0.5 rounding boundary, with enough margin to avoid f32
            // ambiguity, and must cross exactly one snap boundary.
            const CROSSING_FRACTION: f32 = 0.65;
            let crossing_scene = scene_for_pose(
                aligned_eye
                    + aligned.right * (aligned.texel_size * (CROSSING_FRACTION - base_x_fraction)),
                initial_rotation,
                &sun,
            );
            let crossing = directional_cascade_snap_debug(&crossing_scene, cascade_index)
                .expect("crossed cascade snap parameters");
            assert!(
                ((crossing.snapped_x - aligned.snapped_x).abs() - aligned.texel_size).abs()
                    <= aligned.texel_size * 2.0e-3,
                "cascade {cascade_index} did not advance exactly one X texel: before={}, after={}, texel={}",
                aligned.snapped_x,
                crossing.snapped_x,
                aligned.texel_size
            );
            assert!((crossing.snapped_y - aligned.snapped_y).abs() <= aligned.texel_size * 1.0e-4);
            let crossing_matrix =
                MeshRenderer::directional_shadow_cascades(&crossing_scene).matrices[cascade_index];
            let base_pixel = shadow_pixel_position(aligned_matrix, aligned.center);
            let moved_pixel = shadow_pixel_position(crossing_matrix, aligned.center);
            let pixel_delta = moved_pixel - base_pixel;
            assert!(
                (pixel_delta.x.abs() - 1.0).abs() <= 0.02 && pixel_delta.y.abs() <= 0.02,
                "cascade {cascade_index} snap boundary should move projection by one X texel, got {pixel_delta:?}"
            );
        }
    }

    #[test]
    fn cascade_texel_grid_remains_stable_for_small_camera_yaw_and_pitch() {
        let eye = Vec3::new(0.0, 6.0, 20.0);
        let target = Vec3::new(0.0, 0.0, -20.0);
        let sun = SunLight {
            direction: Vec3::new(-0.65, -0.65, -0.35).normalize(),
            ..SunLight::default()
        };
        let base_rotation = Transform3D::looking_at(eye, target, Vec3::Y).rotation;
        let base_scene = scene_for_pose(eye, base_rotation, &sun);

        for cascade_index in 0..DIRECTIONAL_CASCADE_COUNT {
            let initial = directional_cascade_snap_debug(&base_scene, cascade_index)
                .expect("valid cascade snap parameters");
            let alignment = initial.right
                * (initial.snapped_x + initial.texel_size * 0.1 - initial.center_x)
                + initial.up * (initial.snapped_y + initial.texel_size * 0.1 - initial.center_y);
            let aligned_eye = eye + alignment;
            let aligned_scene = scene_for_pose(aligned_eye, base_rotation, &sun);
            let aligned = directional_cascade_snap_debug(&aligned_scene, cascade_index)
                .expect("aligned cascade snap parameters");
            let base_matrix =
                MeshRenderer::directional_shadow_cascades(&aligned_scene).matrices[cascade_index];
            let center_distance = aligned.center.distance(aligned_eye).max(1.0);
            let angle = (aligned.texel_size * 0.05 / center_distance).min(0.0005);
            let yaw = glam::Quat::from_axis_angle(Vec3::Y, angle) * base_rotation;
            let pitch_axis = (base_rotation * Vec3::X).normalize();
            let pitch = glam::Quat::from_axis_angle(pitch_axis, angle) * base_rotation;

            for (label, rotated_pose) in [("yaw", yaw), ("pitch", pitch)] {
                let rotated_scene = scene_for_pose(aligned_eye, rotated_pose, &sun);
                let rotated = directional_cascade_snap_debug(&rotated_scene, cascade_index)
                    .expect("rotated cascade snap parameters");
                assert!(
                    (rotated.texel_size - aligned.texel_size).abs()
                        <= aligned.texel_size * 2.0e-4,
                    "cascade {cascade_index} texel scale changed under small {label}: before={}, after={}",
                    aligned.texel_size,
                    rotated.texel_size
                );
                assert!(
                    (rotated.snapped_x - aligned.snapped_x).abs() <= aligned.texel_size * 1.0e-4
                );
                assert!(
                    (rotated.snapped_y - aligned.snapped_y).abs() <= aligned.texel_size * 1.0e-4
                );
                let rotated_matrix = MeshRenderer::directional_shadow_cascades(&rotated_scene)
                    .matrices[cascade_index];
                let base_pixel = shadow_pixel_position(base_matrix, aligned.center);
                let rotated_pixel = shadow_pixel_position(rotated_matrix, aligned.center);
                let pixel_delta = rotated_pixel - base_pixel;
                assert!(
                    pixel_delta.length() <= 0.05,
                    "cascade {cascade_index} had sub-texel {label} projection drift: {pixel_delta:?}"
                );
            }
        }
    }

    #[test]
    fn far_cascade_covers_far_plane_under_camera_rotation() {
        let eye = Vec3::new(1.0, 5.0, 12.0);
        let sun = SunLight {
            direction: Vec3::new(-0.55, -0.72, -0.42).normalize(),
            ..SunLight::default()
        };
        let camera = Camera3D {
            fov_y: 70.0_f32.to_radians(),
            near: 0.05,
            far: 80.0,
        };
        let camera_poses: [(f32, f32); 5] = [
            (-35.0, -12.0),
            (-15.0, 8.0),
            (0.0, 0.0),
            (22.0, 13.0),
            (40.0, -8.0),
        ];
        for (yaw_degrees, pitch_degrees) in camera_poses {
            let rotation = (glam::Quat::from_rotation_y(yaw_degrees.to_radians())
                * glam::Quat::from_rotation_x(pitch_degrees.to_radians()))
            .normalize();
            let view = Mat4::from_rotation_translation(rotation, eye).inverse();
            let view_proj = camera.projection(16.0 / 9.0) * view;
            let scene = Scene {
                view_proj,
                camera_position: eye,
                camera_rotation: rotation,
                sun: &sun,
                spot_lights: &[],
                point_lights: &[],
                fog: FogSettings::default(),
                draws: &[],
                particles: &[],
            };
            let corners = frustum_corners(view_proj).expect("rotated camera frustum");
            let cascades = MeshRenderer::directional_shadow_cascades(&scene);
            let far_index = DIRECTIONAL_CASCADE_COUNT - 1;
            let previous = cascades.split_depths[far_index - 1];
            let far = cascades.split_depths[far_index];
            assert!(
                far.is_finite() && (far - camera.far).abs() <= 0.02,
                "far cascade split should reach the extracted camera far plane within f32 unprojection tolerance: expected={}, measured={far}, yaw={yaw_degrees}, pitch={pitch_degrees}",
                camera.far
            );
            let slice_start = previous - (far - previous) * DIRECTIONAL_CASCADE_BLEND;
            let slice = split_frustum_corners(
                &corners,
                cascades.near_depth,
                far,
                slice_start.max(cascades.near_depth),
                far,
            );
            let matrix = cascades.matrices[far_index];
            for corner in slice {
                let clip = matrix * corner.extend(1.0);
                assert!(clip.is_finite() && clip.w.abs() > 1.0e-7);
                let ndc = clip.truncate() / clip.w;
                assert!(
                    ndc.x.abs() <= 1.001,
                    "far cascade X at yaw={yaw_degrees}, pitch={pitch_degrees}: {ndc:?}"
                );
                assert!(
                    ndc.y.abs() <= 1.001,
                    "far cascade Y at yaw={yaw_degrees}, pitch={pitch_degrees}: {ndc:?}"
                );
                assert!(
                    ndc.z >= -0.001 && ndc.z <= 1.001,
                    "far cascade Z at yaw={yaw_degrees}, pitch={pitch_degrees}: {ndc:?}"
                );
            }
        }
    }

    #[test]
    fn invalid_view_projection_uses_finite_fallback_shadow_matrix() {
        let sun = SunLight::default();
        let scene = Scene {
            view_proj: Mat4::ZERO,
            camera_position: Vec3::ZERO,
            camera_rotation: glam::Quat::IDENTITY,
            sun: &sun,
            spot_lights: &[],
            point_lights: &[],
            fog: FogSettings::default(),
            draws: &[],
            particles: &[],
        };
        let matrix = MeshRenderer::directional_shadow_matrix(&scene);
        assert!(matrix.to_cols_array().iter().all(|value| value.is_finite()));
    }

    fn box_at(center: Vec3, half_extent: Vec3) -> Aabb3 {
        Aabb3::from_min_max(center - half_extent, center + half_extent).unwrap()
    }

    fn light_box(bounds: Aabb3, right: Vec3, up: Vec3, direction: Vec3) -> Aabb3 {
        directional_light_space_bounds(bounds, right, up, direction).unwrap()
    }

    #[test]
    fn upstream_direction_extends_depth_but_downstream_does_not() {
        // sun.direction is the direction light rays travel. The shader's surface-to-light vector
        // is its negation, so an upstream caster is located along -direction.
        let direction = Vec3::new(0.0, -1.0, 0.0);
        let (right, up) = directional_light_basis(direction).unwrap();
        let receiver_min = 0.0;
        let receiver_max = 10.0;
        let half_extent = 5.0;
        let texel = 0.01;
        let upstream = box_at(-direction * 4.0, Vec3::splat(0.25));
        let downstream = box_at(direction * 4.0, Vec3::splat(0.25));
        let (upstream_min, selected) =
            directional_caster_depth_extension(DirectionalCasterDepthInput {
                receiver_min_depth: receiver_min,
                nearest_receiver_depth: receiver_min,
                snapped_x: 0.0,
                snapped_y: 0.0,
                half_extent,
                texel_size: texel,
                light_space_caster_bounds: &[light_box(upstream, right, up, direction)],
                max_extrusion_distance: 10.0,
            });
        assert!(upstream_min < receiver_min);
        assert_eq!(selected, 1);

        let (downstream_min, selected) =
            directional_caster_depth_extension(DirectionalCasterDepthInput {
                receiver_min_depth: receiver_min,
                nearest_receiver_depth: receiver_min,
                snapped_x: 0.0,
                snapped_y: 0.0,
                half_extent,
                texel_size: texel,
                light_space_caster_bounds: &[light_box(downstream, right, up, direction)],
                max_extrusion_distance: 10.0,
            });
        assert_eq!(downstream_min, receiver_min);
        assert_eq!(selected, 0);
        assert!(receiver_max > receiver_min);
    }

    #[test]
    fn caster_outside_xy_or_extrusion_band_does_not_expand_depth() {
        let direction = Vec3::new(0.0, -1.0, 0.0);
        let (right, up) = directional_light_basis(direction).unwrap();
        let outside_xy = box_at(Vec3::new(100.0, 4.0, 0.0), Vec3::splat(0.5));
        let too_far_upstream = box_at(Vec3::new(0.0, 200.0, 0.0), Vec3::splat(0.5));
        for caster in [outside_xy, too_far_upstream] {
            let (minimum, selected) =
                directional_caster_depth_extension(DirectionalCasterDepthInput {
                    receiver_min_depth: 0.0,
                    nearest_receiver_depth: 0.0,
                    snapped_x: 0.0,
                    snapped_y: 0.0,
                    half_extent: 5.0,
                    texel_size: 0.01,
                    light_space_caster_bounds: &[light_box(caster, right, up, direction)],
                    max_extrusion_distance: 10.0,
                });
            assert_eq!(minimum, 0.0);
            assert_eq!(selected, 0);
        }
    }

    #[test]
    fn partially_overlapping_caster_is_conservatively_included_and_rounded_outward() {
        let direction = Vec3::new(0.0, -1.0, 0.0);
        let (right, up) = directional_light_basis(direction).unwrap();
        // With this basis X maps to +/- right. The box crosses the positive XY edge.
        let caster = box_at(Vec3::new(5.2, 4.0, 0.0), Vec3::new(0.5, 0.25, 0.5));
        let (minimum, selected) = directional_caster_depth_extension(DirectionalCasterDepthInput {
            receiver_min_depth: 0.0,
            nearest_receiver_depth: 0.0,
            snapped_x: 0.0,
            snapped_y: 0.0,
            half_extent: 5.0,
            texel_size: 0.1,
            light_space_caster_bounds: &[light_box(caster, right, up, direction)],
            max_extrusion_distance: 10.0,
        });
        assert!(minimum < 0.0);
        assert_eq!(selected, 1);
        let unrounded = -4.25_f32;
        let rounded = outward_quantized_directional_depth_min(unrounded);
        assert!(rounded <= unrounded);
    }

    #[test]
    fn zero_extrusion_and_invalid_caster_bounds_preserve_receiver_interval() {
        let direction = Vec3::new(0.0, -1.0, 0.0);
        let (right, up) = directional_light_basis(direction).unwrap();
        let upstream = box_at(Vec3::new(0.0, 4.0, 0.0), Vec3::splat(0.25));
        let (disabled_min, disabled_count) =
            directional_caster_depth_extension(DirectionalCasterDepthInput {
                receiver_min_depth: 0.0,
                nearest_receiver_depth: 0.0,
                snapped_x: 0.0,
                snapped_y: 0.0,
                half_extent: 5.0,
                texel_size: 0.01,
                light_space_caster_bounds: &[light_box(upstream, right, up, direction)],
                max_extrusion_distance: 0.0,
            });
        assert_eq!(disabled_min, 0.0);
        assert_eq!(disabled_count, 0);

        let invalid = Aabb3 {
            min: Vec3::splat(f32::NAN),
            max: Vec3::ONE,
        };
        let (invalid_min, invalid_count) =
            directional_caster_depth_extension(DirectionalCasterDepthInput {
                receiver_min_depth: 0.0,
                nearest_receiver_depth: 0.0,
                snapped_x: 0.0,
                snapped_y: 0.0,
                half_extent: 5.0,
                texel_size: 0.01,
                light_space_caster_bounds: &[invalid],
                max_extrusion_distance: 10.0,
            });
        assert_eq!(invalid_min, 0.0);
        assert_eq!(invalid_count, 0);

        let infinite = Aabb3 {
            min: Vec3::splat(f32::NEG_INFINITY),
            max: Vec3::splat(f32::INFINITY),
        };
        let (infinite_min, infinite_count) =
            directional_caster_depth_extension(DirectionalCasterDepthInput {
                receiver_min_depth: 0.0,
                nearest_receiver_depth: 0.0,
                snapped_x: 0.0,
                snapped_y: 0.0,
                half_extent: 5.0,
                texel_size: 0.01,
                light_space_caster_bounds: &[infinite],
                max_extrusion_distance: 10.0,
            });
        assert_eq!(infinite_min, 0.0);
        assert_eq!(infinite_count, 0);
    }

    #[test]
    fn caster_depth_extension_does_not_change_receiver_xy_fitting() {
        let sun = SunLight::default();
        let camera_position = Vec3::new(0.0, 3.0, 6.0);
        let view = Mat4::look_at_rh(camera_position, Vec3::ZERO, Vec3::Y);
        let view_proj = Camera3D::default().projection(16.0 / 9.0) * view;
        let scene = Scene {
            view_proj,
            camera_position,
            camera_rotation: glam::Quat::IDENTITY,
            sun: &sun,
            spot_lights: &[],
            point_lights: &[],
            fog: FogSettings::default(),
            draws: &[],
            particles: &[],
        };
        let direction = finite_direction(sun.direction);
        let upstream = box_at(-direction * 25.0, Vec3::splat(0.5));
        let receiver_only =
            MeshRenderer::directional_shadow_cascades_with_casters(&scene, &[], 0.0);
        let with_caster =
            MeshRenderer::directional_shadow_cascades_with_casters(&scene, &[upstream], 40.0);
        for index in 0..DIRECTIONAL_CASCADE_COUNT {
            let base = receiver_only.matrices[index];
            let extended = with_caster.matrices[index];
            for point in [Vec3::ZERO, camera_position, Vec3::new(2.0, 0.0, -3.0)] {
                let base_clip = base * point.extend(1.0);
                let caster_clip = extended * point.extend(1.0);
                assert!(base_clip.is_finite() && caster_clip.is_finite());
                assert!((base_clip.x - caster_clip.x).abs() <= 1.0e-5);
                assert!((base_clip.y - caster_clip.y).abs() <= 1.0e-5);
            }
        }
    }

    #[test]
    fn caster_aabb_transform_is_conservative_for_rotation_shear_and_negative_scale() {
        let local = box_at(Vec3::new(1.0, -2.0, 0.5), Vec3::new(0.5, 1.0, 2.0));
        let mut shear = Mat4::IDENTITY;
        shear.x_axis.y = 0.35;
        let transform = Mat4::from_translation(Vec3::new(4.0, 3.0, -2.0))
            * Mat4::from_rotation_y(0.7)
            * Mat4::from_scale(Vec3::new(-2.0, 0.5, 3.0))
            * shear;
        let world = local
            .transformed(transform)
            .expect("affine transform bounds");
        for corner in local.corners() {
            let point = transform.transform_point3(corner);
            assert!(point.cmpge(world.min - Vec3::splat(1.0e-5)).all());
            assert!(point.cmple(world.max + Vec3::splat(1.0e-5)).all());
        }
        let mut projective = Mat4::IDENTITY;
        projective.x_axis.w = 0.01;
        assert!(local.transformed(projective).is_none());
    }

    #[test]
    fn skinned_pose_bounds_follow_non_identity_joint_palettes_and_all_influences() {
        let first = box_at(Vec3::ZERO, Vec3::splat(0.1));
        let second = box_at(Vec3::new(1.0, 0.0, 0.0), Vec3::splat(0.1));
        let joint_bounds = [(0_u16, first), (7_u16, second)];
        let mut palette = vec![Mat4::IDENTITY; 8];
        palette[0] = Mat4::from_translation(Vec3::new(4.0, 0.0, 0.0));
        palette[7] = Mat4::from_translation(Vec3::new(0.0, 5.0, 0.0));
        let deformed = skinned_pose_local_bounds(&joint_bounds, None, 1.0, 1.0, &palette)
            .expect("valid current-pose bounds");
        assert!(deformed.max.x > 3.8);
        assert!(deformed.max.y > 4.8);
        assert!(deformed.max.x > first.max.x);
        assert!(deformed.max.y > second.max.y);
    }

    #[test]
    fn skinned_bounds_include_unweighted_vertices_and_reject_short_palettes() {
        let joint = box_at(Vec3::ZERO, Vec3::splat(0.1));
        let unweighted = box_at(Vec3::new(-3.0, 0.0, 0.0), Vec3::splat(0.1));
        let palette = [Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0))];
        let bounds =
            skinned_pose_local_bounds(&[(0, joint)], Some(unweighted), 1.0, 1.0, &palette).unwrap();
        assert!(bounds.min.x <= -3.1);
        assert!(bounds.max.x >= 2.1);
        assert!(skinned_pose_local_bounds(&[(1, joint)], None, 1.0, 1.0, &palette).is_none());
    }

    #[test]
    fn subnormalized_skin_weights_are_bounded_through_local_origin() {
        let joint = box_at(Vec3::new(2.0, 0.0, 0.0), Vec3::splat(0.1));
        let bounds =
            skinned_pose_local_bounds(&[(0, joint)], None, 0.9995, 0.9995, &[Mat4::IDENTITY])
                .unwrap();
        assert!(bounds.min.x <= 0.0 && bounds.max.x >= 2.1);
    }
}
