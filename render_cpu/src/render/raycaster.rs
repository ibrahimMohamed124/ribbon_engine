//! Camera setup and the parallel per-pixel rendering pipeline.

use rayon::prelude::*;

use super::entities::{EntHit, EntView, SpriteView};
use super::lighting::{
    cone_factor, light_with_style, shade_surface, SurfaceContext, SurfaceShader,
};
use super::post::{finish_pixel_styled_with_lut, PixelSample};
use engine_core::gfx::Framebuffer;
use engine_core::math::{forward_vec, Vect};
use engine_scene::entities::EntityRenderMode;
use engine_scene::material::MaterialTable;
use engine_scene::scene::Scene;
use engine_scene::RENDER_DIST;

const EMPTY_MATERIALS: MaterialTable = MaterialTable::new(&[]);

fn shade_pixel<S: SurfaceShader + Sync>(
    f: &Scene,
    entities: &[EntView<'_>],
    sprites: &[SpriteView],
    shader: &S,
    materials: &MaterialTable,
    fwd: Vect,
    dir: Vect,
) -> f32 {
    let cone = cone_factor(dir.dot(fwd));
    let world_hit = f.world.cast(f.camera.eye, dir, RENDER_DIST, f.empty_voxel);
    let world_distance = world_hit.map_or(f32::INFINITY, |hit| hit.dist);

    if f.lighting.style.billboards_enabled {
        if let Some(atlas) = materials.sprite_atlas.filter(|atlas| atlas.is_valid()) {
            let mut sprite_hit = None;
            for sprite in sprites {
                if let Some(hit) = sprite.hit(dir, atlas) {
                    if hit.t < world_distance
                        && sprite_hit
                            .is_none_or(|current: super::entities::SpriteHit| hit.t < current.t)
                    {
                        sprite_hit = Some(hit);
                    }
                }
            }
            if let Some(hit) = sprite_hit {
                if let Some((albedo, normal, emissive)) =
                    // The pose atlas is stored top-down while sprite hit V is
                    // measured upward from the entity's feet.
                    atlas.texel(hit.view, hit.state, hit.frame, hit.u, 1.0 - hit.v)
                {
                    if albedo[3] >= 128 {
                        let linear = |v: u8| {
                            let x = v as f32 / 255.0;
                            if x <= 0.04045 {
                                x / 12.92
                            } else {
                                ((x + 0.055) / 1.055).powf(2.4)
                            }
                        };
                        let base = 0.2126 * linear(albedo[0])
                            + 0.7152 * linear(albedo[1])
                            + 0.0722 * linear(albedo[2]);
                        let n = Vect::new(
                            normal[0] as f32 / 127.5 - 1.0,
                            normal[1] as f32 / 127.5 - 1.0,
                            normal[2] as f32 / 127.5 - 1.0,
                        )
                        .normalize();
                        let ndotl = n.dot(dir.scale(-1.0)).max(0.0);
                        let cone = cone_factor(dir.dot(fwd));
                        let lit = base
                            * light_with_style(
                                f.lighting.light_level,
                                hit.t,
                                ndotl,
                                cone,
                                &f.lighting.style,
                            );
                        return (lit + emissive as f32 / 255.0 * 0.85).clamp(0.0, 2.0);
                    }
                }
            }
        }
    }

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
                f.lighting.light_level,
                hit.t,
                hit.ndotl,
                cone,
                &f.lighting.style,
            );
    }
    match world_hit {
        Some(hit) => {
            let shaded = shade_surface(
                shader,
                f.world,
                SurfaceContext {
                    light_level: f.lighting.light_level,
                    hit: &hit,
                    direction: dir,
                    cone,
                    empty_voxel: f.empty_voxel,
                    style: &f.lighting.style,
                    materials,
                },
            );
            if hit.normal[2] > 0 {
                let shadow = sprites
                    .iter()
                    .map(|sprite| {
                        sprite.floor_shadow_strength([hit.point.x, hit.point.y, hit.point.z])
                    })
                    .fold(0.0_f32, f32::max);
                shaded * (1.0 - shadow)
            } else {
                shaded
            }
        }
        None => 0.0,
    }
}

pub fn render<S: SurfaceShader + Sync>(fb: &mut Framebuffer, frame: &Scene, shader: &S) {
    render_with_materials(fb, frame, shader, &EMPTY_MATERIALS);
}

pub fn render_with_materials<S: SurfaceShader + Sync>(
    fb: &mut Framebuffer,
    frame: &Scene,
    shader: &S,
    materials: &MaterialTable,
) {
    let level_lut = frame.lighting.style.level_lut();
    let entities: Vec<_> = frame
        .entities
        .iter()
        .filter(|entity| {
            !(frame.lighting.style.billboards_enabled
                && materials.sprite_atlas.is_some()
                && frame
                    .entity_registry
                    .get(entity.kind)
                    .is_some_and(|definition| {
                        matches!(definition.render_mode, EntityRenderMode::Billboard { .. })
                    }))
        })
        .filter_map(|entity| EntView::new(*entity, frame.camera.eye, frame.entity_registry))
        .collect();
    let sprites: Vec<_> =
        if frame.lighting.style.billboards_enabled && materials.sprite_atlas.is_some() {
            frame
                .entities
                .iter()
                .filter_map(|entity| {
                    let definition = frame.entity_registry.get(entity.kind)?;
                    matches!(definition.render_mode, EntityRenderMode::Billboard { .. })
                        .then(|| SpriteView::new(*entity, definition, frame.camera.eye))
                })
                .collect()
        } else {
            Vec::new()
        };
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
                let warp = engine_scene::style::screen_warp(
                    u,
                    v,
                    frame.lighting.frame as f32 * 0.016,
                    if frame.lighting.style.limit_flicker {
                        0.0
                    } else {
                        frame.lighting.intensity
                    },
                );
                let direction = (forward
                    + right.scale((u + warp[0]) * tan_h)
                    + up.scale((v + warp[1]) * tan_v))
                .normalize();
                let gray = shade_pixel(
                    frame, &entities, &sprites, shader, materials, forward, direction,
                );
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
