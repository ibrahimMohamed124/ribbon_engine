use super::*;

/// The directional shadow allocation is fixed at construction time. Keep this value in sync
/// with `renderer/setup.rs`; texel snapping uses the actual texture resolution.
pub(super) const DIRECTIONAL_SHADOW_RESOLUTION: u32 = 1024;

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

fn finite_direction(direction: Vec3) -> Vec3 {
    let normalized = direction.normalize_or_zero();
    if normalized.is_finite() && normalized.length_squared() > 0.5 {
        normalized
    } else {
        Vec3::new(-0.4, -0.8, -0.45).normalize()
    }
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

    /// Fit a stable orthographic directional-shadow projection around the active camera frustum.
    ///
    /// The fit uses a rotation-invariant bounding sphere in light space for its XY extent. Its
    /// center is snapped in world-space light-basis coordinates to the shadow texel grid, while
    /// the depth range is fitted to the eight actual frustum corners. This keeps camera-only
    /// translation below one texel from continuously moving the shadow projection.
    pub(super) fn directional_shadow_matrix(scene: &Scene<'_>) -> Mat4 {
        let direction = finite_direction(scene.sun.direction);
        let reference_up = if direction.dot(Vec3::Y).abs() > 0.95 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let right = direction.cross(reference_up).normalize_or_zero();
        let up = right.cross(direction).normalize_or_zero();
        if !right.is_finite()
            || !up.is_finite()
            || right.length_squared() < 0.5
            || up.length_squared() < 0.5
        {
            return Self::fallback_directional_shadow_matrix(scene, direction);
        }

        let Some(corners) = frustum_corners(scene.view_proj) else {
            return Self::fallback_directional_shadow_matrix(scene, direction);
        };
        let center = corners.iter().copied().sum::<Vec3>() / corners.len() as f32;
        let radius = corners
            .iter()
            .map(|corner| corner.distance(center))
            .fold(0.0_f32, f32::max);
        if !center.is_finite() || !radius.is_finite() || radius <= 1.0e-4 {
            return Self::fallback_directional_shadow_matrix(scene, direction);
        }

        // A sphere gives the same XY scale while the camera rotates, avoiding the extent
        // changes caused by fitting a tight light-space AABB every frame.
        let base_half_extent = radius.max(0.5);
        let nominal_texel = (2.0 * base_half_extent) / DIRECTIONAL_SHADOW_RESOLUTION as f32;
        let half_extent = base_half_extent + nominal_texel * 1.5;
        let texel_size = (2.0 * half_extent) / DIRECTIONAL_SHADOW_RESOLUTION as f32;
        let center_x = center.dot(right);
        let center_y = center.dot(up);
        let snapped_x = (center_x / texel_size).round() * texel_size;
        let snapped_y = (center_y / texel_size).round() * texel_size;

        let min_depth = corners
            .iter()
            .map(|corner| corner.dot(direction))
            .fold(f32::INFINITY, f32::min);
        let max_depth = corners
            .iter()
            .map(|corner| corner.dot(direction))
            .fold(f32::NEG_INFINITY, f32::max);
        if !min_depth.is_finite() || !max_depth.is_finite() || max_depth <= min_depth {
            return Self::fallback_directional_shadow_matrix(scene, direction);
        }

        let depth_padding = (radius * 0.08).max(1.0);
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
    use super::{frustum_corners, DIRECTIONAL_SHADOW_RESOLUTION};
    use crate::components::{Camera3D, FogSettings, SunLight};
    use crate::renderer::{MeshRenderer, Scene};
    use glam::{Mat4, Vec3};

    fn projection_at(position: Vec3, target: Vec3) -> Mat4 {
        let direction = (target - position).normalize();
        let view = Mat4::look_to_rh(position, direction, Vec3::Y);
        Camera3D::default().projection(16.0 / 9.0) * view
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
        let base_view_proj = projection_at(eye, target);
        let corners = frustum_corners(base_view_proj).expect("valid frustum");
        let center = corners.iter().copied().sum::<Vec3>() / 8.0;
        let radius = corners
            .iter()
            .map(|p| p.distance(center))
            .fold(0.0_f32, f32::max);
        let nominal_texel = (2.0 * radius.max(0.5)) / DIRECTIONAL_SHADOW_RESOLUTION as f32;
        let direction = SunLight::default().direction.normalize();
        let reference_up = if direction.dot(Vec3::Y).abs() > 0.95 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        let right = direction.cross(reference_up).normalize();
        let shift = right * (nominal_texel * 0.1);
        let moved_view_proj = projection_at(eye + shift, target + shift);
        let sun = SunLight::default();
        let base_scene = Scene {
            view_proj: base_view_proj,
            camera_position: eye,
            camera_rotation: glam::Quat::IDENTITY,
            sun: &sun,
            spot_lights: &[],
            point_lights: &[],
            fog: FogSettings::default(),
            draws: &[],
            particles: &[],
        };
        let moved_scene = Scene {
            view_proj: moved_view_proj,
            camera_position: eye + shift,
            camera_rotation: glam::Quat::IDENTITY,
            sun: &sun,
            spot_lights: &[],
            point_lights: &[],
            fog: FogSettings::default(),
            draws: &[],
            particles: &[],
        };
        let a = MeshRenderer::directional_shadow_matrix(&base_scene);
        let b = MeshRenderer::directional_shadow_matrix(&moved_scene);
        let max_delta = a
            .to_cols_array()
            .into_iter()
            .zip(b.to_cols_array())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0_f32, f32::max);
        assert!(
            max_delta <= 1.0e-5,
            "sub-texel movement changed shadow matrix by {max_delta}"
        );
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
}
