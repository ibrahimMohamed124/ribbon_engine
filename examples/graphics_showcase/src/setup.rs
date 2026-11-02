use super::scene::{spawn_cube, spawn_model};
use super::*;

pub(super) fn add_scene(
    world: &mut World,
    checker: ribbon_assets::Handle<ModelAsset>,
    checker_center: Vec3,
    triangle: ribbon_assets::Handle<ModelAsset>,
    triangle_center: Vec3,
    alpha_showcase: ribbon_assets::Handle<ModelAsset>,
) {
    // Ground slab and a subtle grid make perspective, scale, and camera movement easy to read.
    spawn_cube(
        world,
        Vec3::new(0.0, -FLOOR_HALF_EXTENTS.y, 0.0),
        FLOOR_HALF_EXTENTS,
        [0.105, 0.13, 0.18],
        None,
    );
    for i in -8..=8 {
        let x = i as f32 * 0.8;
        spawn_cube(
            world,
            Vec3::new(x, 0.006, 0.0),
            Vec3::new(0.008, 0.009, 9.8),
            [0.16, 0.20, 0.26],
            None,
        );
    }
    for i in -6..=6 {
        let z = i as f32 * 0.8;
        spawn_cube(
            world,
            Vec3::new(0.0, 0.006, z),
            Vec3::new(13.4, 0.009, 0.008),
            [0.16, 0.20, 0.26],
            None,
        );
    }

    // Central display plinth and three instances of the same textured GLB asset.
    spawn_cube(
        world,
        Vec3::new(0.0, 0.11, 0.0),
        Vec3::new(3.25, 0.22, 2.25),
        [0.18, 0.23, 0.30],
        None,
    );
    spawn_model(
        world,
        checker,
        checker_center,
        Vec3::new(0.0, 1.05, 0.0),
        1.75,
        0.35,
        0.0,
        0.12,
    );
    spawn_model(
        world,
        checker,
        checker_center,
        Vec3::new(-2.15, 0.82, -0.15),
        1.12,
        -0.52,
        1.3,
        0.20,
    );
    spawn_model(
        world,
        checker,
        checker_center,
        Vec3::new(2.15, 0.82, -0.15),
        1.12,
        0.58,
        2.4,
        -0.16,
    );

    // A separately imported untextured triangle demonstrates a minimal glTF primitive.
    spawn_cube(
        world,
        Vec3::new(5.2, 0.12, -0.9),
        Vec3::new(1.65, 0.24, 1.65),
        [0.20, 0.26, 0.33],
        None,
    );
    spawn_model(
        world,
        triangle,
        triangle_center,
        Vec3::new(5.2, 1.08, -0.9),
        1.65,
        0.42,
        0.8,
        0.16,
    );

    // A dedicated multi-primitive glTF asset displays opaque, alpha-cutout,
    // overlapping transparent, and reversed-winding double-sided materials.
    // It is deliberately static so the back-to-front ordering is easy to inspect.
    world.spawn((
        Transform3D::default(),
        ModelInstance {
            model: alpha_showcase,
        },
    ));

    // A row of differently colored instanced cubes, with varied size and motion.
    let palette: [[f32; 3]; 7] = [
        [0.95, 0.26, 0.13],
        [0.98, 0.58, 0.16],
        [0.88, 0.83, 0.30],
        [0.20, 0.78, 0.57],
        [0.18, 0.62, 0.94],
        [0.42, 0.36, 0.95],
        [0.83, 0.30, 0.73],
    ];
    for (index, color) in palette.into_iter().enumerate() {
        let x = (index as f32 - 3.0) * 1.65;
        let size = 0.54 + (index % 3) as f32 * 0.12;
        spawn_cube(
            world,
            Vec3::new(x, 0.12, 3.45),
            Vec3::new(0.84, 0.16, 0.84),
            [0.15, 0.19, 0.25],
            None,
        );
        spawn_cube(
            world,
            Vec3::new(x, 0.48 + size * 0.5, 3.45),
            Vec3::splat(size),
            color,
            Some(CubeMotion {
                base_position: Vec3::new(x, 0.48 + size * 0.5, 3.45),
                axis: Vec3::new(0.25 + index as f32 * 0.11, 1.0, 0.2),
                radians_per_second: if index % 2 == 0 { 0.65 } else { -0.48 },
                bob_amplitude: 0.14 + (index % 3) as f32 * 0.035,
                phase: index as f32 * 0.7,
            }),
        );
    }

    // Staggered block columns give the scene more depth and catch the moving light.
    let column_colors = [
        [0.24, 0.40, 0.54],
        [0.28, 0.55, 0.61],
        [0.47, 0.43, 0.67],
        [0.63, 0.36, 0.42],
    ];
    for (column, x) in [-5.7_f32, -3.9, 3.9, 5.7].into_iter().enumerate() {
        let height = 2 + (column % 3);
        spawn_cube(
            world,
            Vec3::new(x, 0.10, -3.2),
            Vec3::new(1.05, 0.2, 1.05),
            [0.13, 0.17, 0.22],
            None,
        );
        for level in 0..height {
            let y = 0.515 + level as f32 * 0.67;
            spawn_cube(
                world,
                Vec3::new(x, y, -3.2),
                Vec3::splat(0.63),
                column_colors[(column + level) % column_colors.len()],
                Some(CubeMotion {
                    base_position: Vec3::new(x, y, -3.2),
                    axis: if level % 2 == 0 {
                        Vec3::Y
                    } else {
                        Vec3::new(1.0, 0.3, 0.0)
                    },
                    radians_per_second: if level % 2 == 0 { 0.18 } else { -0.24 },
                    bob_amplitude: 0.025,
                    phase: column as f32 + level as f32 * 0.4,
                }),
            );
        }
    }
}
