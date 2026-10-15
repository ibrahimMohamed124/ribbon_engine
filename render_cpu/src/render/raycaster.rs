//! Camera setup and the parallel per-pixel rendering pipeline.

use rayon::prelude::*;

use super::entities::{EntHit, EntView};
use super::lighting::{
    cone_factor, light_with_style, shade_surface, SurfaceContext, SurfaceShader,
};
use super::post::{finish_pixel_styled_with_lut, PixelSample};
use engine_core::gfx::Framebuffer;
use engine_core::math::{forward_vec, Vect};
use engine_scene::scene::Scene;
use engine_scene::RENDER_DIST;

fn shade_pixel<S: SurfaceShader + Sync>(
    f: &Scene,
    entities: &[EntView<'_>],
    shader: &S,
    fwd: Vect,
    dir: Vect,
) -> f32 {
    let cone = cone_factor(dir.dot(fwd));
    let world_hit = f.world.cast(f.camera.eye, dir, RENDER_DIST, f.empty_voxel);
    let world_distance = world_hit.map_or(f32::INFINITY, |hit| hit.dist);

    let mut best: Option<EntHit> = None;
    for entity in entities {
        if let Some(hit) = entity.hit(dir) {
            if hit.t < world_distance && best.is_none_or(|current| hit.t < current.t) {
                best = Some(hit);
            }
        }
    }
    if let Some(hit) = best {
        let appearance = shader.entity_surface(hit.kind, hit.point, hit.axis, hit.sign, hit.part);
        if appearance.emissive {
            let fall = (1.0 - hit.t / (RENDER_DIST * 1.4)).clamp(0.0, 1.0);
            return appearance.gray * fall;
        }
        return appearance.gray
            * light_with_style(
                f.lighting.illumination,
                hit.t,
                hit.ndotl,
                cone,
                &f.lighting.style,
            );
    }
    match world_hit {
        Some(hit) => shade_surface(
            shader,
            f.world,
            SurfaceContext {
                illumination: f.lighting.illumination,
                hit: &hit,
                direction: dir,
                cone,
                empty_voxel: f.empty_voxel,
                style: &f.lighting.style,
            },
        ),
        None => 0.0,
    }
}

pub fn render<S: SurfaceShader + Sync>(fb: &mut Framebuffer, frame: &Scene, shader: &S) {
    let level_lut = frame.lighting.style.level_lut();
    let entities: Vec<_> = frame
        .entities
        .iter()
        .filter_map(|entity| EntView::new(*entity, frame.camera.eye, frame.entity_registry))
        .collect();
    let (width, height) = (fb.width, fb.height);
    let view = frame.camera.view;
    let forward = forward_vec(view);
    let right = Vect::new(-view.phi.sin(), view.phi.cos(), 0.0);
    let up = Vect::new(
        -view.psi.sin() * view.phi.cos(),
        -view.psi.sin() * view.phi.sin(),
        view.psi.cos(),
    );
    let tan_h = (frame.camera.hfov / 2.0).tan();
    let tan_v = tan_h * height as f32 / width as f32;

    fb.pixels
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            let v = 1.0 - ((y as f32 + 0.5) / height as f32) * 2.0;
            for (x, pixel) in row.iter_mut().enumerate() {
                let u = ((x as f32 + 0.5) / width as f32) * 2.0 - 1.0;
                let direction =
                    (forward + right.scale(u * tan_h) + up.scale(v * tan_v)).normalize();
                let gray = shade_pixel(frame, &entities, shader, forward, direction);
                *pixel = finish_pixel_styled_with_lut(
                    gray,
                    PixelSample {
                        u,
                        v,
                        x,
                        y,
                        frame: frame.lighting.frame,
                        intensity: frame.lighting.intensity,
                    },
                    &frame.lighting.style,
                    &level_lut,
                );
            }
        });
}
