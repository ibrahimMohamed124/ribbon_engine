//! Camera setup and the parallel per-pixel rendering pipeline.

use rayon::prelude::*;

use super::entities::{EntHit, EntView};
use super::lighting::{cone_factor, light, shade_surface, SurfaceShader};
use super::post::{finish_pixel_styled, PixelSample};
use super::style::RenderStyle;
use super::RENDER_DIST;
use crate::gfx::Framebuffer;
use crate::math::{forward_vec, Angles, Vect};
use crate::voxel::VoxelWorld;

pub struct Frame<'a> {
    pub world: &'a VoxelWorld,
    pub eye: Vect,
    pub view: Angles,
    pub hfov: f32,
    pub illumination: f32,
    pub empty_voxel: u8,
    pub ents: Vec<EntView>,
    pub frame: u32,
    pub intensity: f32,
    pub style: RenderStyle,
}

fn shade_pixel<S: SurfaceShader + Sync>(f: &Frame, shader: &S, fwd: Vect, dir: Vect) -> f32 {
    let cone = cone_factor(dir.dot(fwd));
    let world_hit = f.world.cast(f.eye, dir, RENDER_DIST, f.empty_voxel);
    let world_distance = world_hit.map_or(f32::INFINITY, |hit| hit.dist);

    let mut best: Option<EntHit> = None;
    for entity in &f.ents {
        if let Some(hit) = entity.hit(dir) {
            if hit.t < world_distance && best.is_none_or(|current| hit.t < current.t) {
                best = Some(hit);
            }
        }
    }
    if let Some(hit) = best {
        if hit.emissive {
            let fall = (1.0 - hit.t / (RENDER_DIST * 1.4)).clamp(0.0, 1.0);
            return hit.gray * fall;
        }
        return hit.gray * light(f.illumination, hit.t, hit.ndotl, cone);
    }
    match world_hit {
        Some(hit) => shade_surface(
            shader,
            f.world,
            f.illumination,
            &hit,
            dir,
            cone,
            f.empty_voxel,
        ),
        None => 0.0,
    }
}

pub fn render<S: SurfaceShader + Sync>(fb: &mut Framebuffer, frame: &Frame, shader: &S) {
    let (width, height) = (fb.width, fb.height);
    let view = frame.view;
    let forward = forward_vec(view);
    let right = Vect::new(-view.phi.sin(), view.phi.cos(), 0.0);
    let up = Vect::new(
        -view.psi.sin() * view.phi.cos(),
        -view.psi.sin() * view.phi.sin(),
        view.psi.cos(),
    );
    let tan_h = (frame.hfov / 2.0).tan();
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
                let gray = shade_pixel(frame, shader, forward, direction);
                *pixel = finish_pixel_styled(
                    gray,
                    PixelSample {
                        u,
                        v,
                        x,
                        y,
                        frame: frame.frame,
                        intensity: frame.intensity,
                    },
                    &frame.style,
                );
            }
        });
}
