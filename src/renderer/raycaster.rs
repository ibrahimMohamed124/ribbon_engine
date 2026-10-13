//! Camera setup and the per-pixel rendering pipeline.

use rayon::prelude::*;

use super::entities::{EntHit, EntView};
use super::lighting::{light, shade_world, spot_factor};
use super::post::finish_pixel;
use super::{Framebuffer, RENDER_DIST};
use crate::math::{forward_vec, Angles, Vect};
use crate::world::World;

/// Everything the renderer needs to draw one frame of the 3D view.
pub struct Frame<'a> {
    pub world: &'a World,
    pub eye: Vect,
    pub view: Angles,
    pub hfov: f32,  // horizontal field of view, radians
    pub flash: f32, // 0 = off, otherwise flicker multiplier
    pub ents: Vec<EntView>,
    pub frame: u32,
    pub prox: f32, // 0..1, how close the enemy is
}

fn shade_pixel(f: &Frame, fwd: Vect, dir: Vect) -> f32 {
    let spot = spot_factor(dir.dot(fwd));

    let world_hit = f.world.cast(f.eye, dir, RENDER_DIST);
    let wt = world_hit.map_or(f32::INFINITY, |h| h.dist);

    let mut best: Option<EntHit> = None;
    for e in &f.ents {
        if let Some(h) = e.hit(dir) {
            if h.t < wt && best.is_none_or(|b| h.t < b.t) {
                best = Some(h);
            }
        }
    }
    if let Some(h) = best {
        if h.emissive {
            let fall = (1.0 - h.t / (RENDER_DIST * 1.4)).clamp(0.0, 1.0);
            return h.gray * fall;
        }
        return h.gray * light(f.flash, h.t, h.ndotl, spot);
    }
    match world_hit {
        Some(hit) => shade_world(f.world, f.flash, &hit, dir, spot),
        None => 0.0,
    }
}

/// Render the 3D view into `fb`, one row per rayon task.
pub fn render(fb: &mut Framebuffer, f: &Frame) {
    let (w, h) = (fb.width, fb.height);
    let view = f.view;
    let fwd = forward_vec(view);
    let right = Vect::new(-view.phi.sin(), view.phi.cos(), 0.0);
    let up = Vect::new(
        -view.psi.sin() * view.phi.cos(),
        -view.psi.sin() * view.phi.sin(),
        view.psi.cos(),
    );
    let tan_h = (f.hfov / 2.0).tan();
    let tan_v = tan_h * h as f32 / w as f32;

    fb.pixels
        .par_chunks_mut(w)
        .enumerate()
        .for_each(|(y, row)| {
            let v = 1.0 - ((y as f32 + 0.5) / h as f32) * 2.0;
            for (x, px) in row.iter_mut().enumerate() {
                let u = ((x as f32 + 0.5) / w as f32) * 2.0 - 1.0;
                let dir = (fwd + right.scale(u * tan_h) + up.scale(v * tan_v)).normalize();
                let g = shade_pixel(f, fwd, dir);
                *px = finish_pixel(g, u, v, x, y, f.frame, f.prox);
            }
        });
}
