use super::*;

impl MeshRenderer {
    pub(super) fn ensure_depth(&mut self, device: &wgpu::Device, size: (u32, u32)) {
        if self.depth.as_ref().is_some_and(|depth| depth.size == size) {
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
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        self.depth = Some(DepthTarget {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            size,
        });
        self.update_peak_resource_estimate();
    }

    pub(super) fn directional_shadow_matrix(scene: &Scene<'_>) -> Mat4 {
        let direction = scene.sun.direction.normalize_or_zero();
        let direction = if direction.length_squared() > 0.5 {
            direction
        } else {
            Vec3::new(-0.4, -0.8, -0.45).normalize()
        };
        let focus = scene.camera_position;
        let eye = focus - direction * 32.0;
        let up = if direction.dot(Vec3::Y).abs() > 0.95 {
            Vec3::Z
        } else {
            Vec3::Y
        };
        Mat4::orthographic_rh(-24.0, 24.0, -24.0, 24.0, 0.1, 80.0)
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
