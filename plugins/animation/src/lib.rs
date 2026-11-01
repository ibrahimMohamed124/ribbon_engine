//! Deterministic sprite and glTF skeletal animation data and update systems.
//!
//! Sprite clips advance atlas frames; skeletal clips evaluate glTF node transforms and publish
//! immutable joint palettes for the renderer's vertex-stage skinning path.

use glam::{Mat4, Quat, Vec4};
use ribbon_app::{App, Stage, Time};
use ribbon_assets::{AssetLoader, AssetMemoryBreakdown, LoadContext, LoadError, LoadErrorKind};
use ribbon_ecs::{Entity, World};
use ribbon_plugin_gltf::{
    AnimationInterpolation, AnimationProperty, AnimationTrack, ModelAsset, SkeletalAnimationClip,
};
use ribbon_plugin_render3d::{ModelInstance, SkeletalPose};
use ribbon_plugins::Plugin;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Sprite animation selection and fractional frame position.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpriteAnimator {
    pub state: u8,
    pub frame: f32,
    pub fps: f32,
}

/// Reusable CPU-side sprite animation data loaded through the engine AssetServer.
///
/// The clip stores atlas frame indices and timing, not GPU objects: sprite textures remain
/// ordinary renderer-managed texture assets and are warmed through the renderer path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpriteAnimationClip {
    pub name: String,
    pub frame_indices: Vec<u32>,
    pub frames_per_second: f32,
    pub looping: bool,
}

impl SpriteAnimationClip {
    /// Validate invariants required by deterministic frame advancement.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("animation clip name must not be empty".to_owned());
        }
        if self.frame_indices.is_empty() {
            return Err("animation clip must contain at least one frame".to_owned());
        }
        if !self.frames_per_second.is_finite() || self.frames_per_second <= 0.0 {
            return Err("animation clip FPS must be finite and greater than zero".to_owned());
        }
        Ok(())
    }

    /// Estimated decoded memory, including owned string and frame vector capacities.
    pub fn estimated_size_bytes(&self) -> u64 {
        (std::mem::size_of::<Self>() as u64)
            .saturating_add(self.name.capacity() as u64)
            .saturating_add(
                (self.frame_indices.capacity() as u64)
                    .saturating_mul(std::mem::size_of::<u32>() as u64),
            )
    }
}

/// Runtime playback state for an asset-backed sprite animation clip.
/// The clip handle is weak; the owning scene should keep an `AssetRef` lease while this component
/// is active so normal CPU cache eviction cannot remove the clip during playback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpriteClipPlayer {
    pub clip: ribbon_assets::Handle<SpriteAnimationClip>,
    pub elapsed_frames: f32,
    pub current_frame: u32,
    pub completed: bool,
}

impl SpriteClipPlayer {
    pub fn new(clip: ribbon_assets::Handle<SpriteAnimationClip>) -> Self {
        Self {
            clip,
            elapsed_frames: 0.0,
            current_frame: 0,
            completed: false,
        }
    }
}

/// Runtime player for a glTF skeletal animation clip on a `ModelInstance` entity.
///
/// The model handle is taken from the entity's `ModelInstance`, avoiding a second source of truth.
/// Animation evaluation is CPU-side; the resulting immutable joint palettes are consumed by the
/// renderer's vertex-stage skinning path. No GPU calls are made by this plugin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkeletalAnimationPlayer {
    pub clip_index: usize,
    pub elapsed_seconds: f32,
    pub playback_speed: f32,
    pub looping: bool,
    pub paused: bool,
}

impl Default for SkeletalAnimationPlayer {
    fn default() -> Self {
        Self {
            clip_index: 0,
            elapsed_seconds: 0.0,
            playback_speed: 1.0,
            looping: true,
            paused: false,
        }
    }
}

impl SkeletalAnimationPlayer {
    pub fn new(clip_index: usize) -> Self {
        Self {
            clip_index,
            ..Self::default()
        }
    }
}

/// Sample a glTF transform track at a clip-local timestamp.
/// Supports STEP, LINEAR (including quaternion slerp), and CUBICSPLINE Hermite interpolation.
pub fn sample_animation_track(
    track: &AnimationTrack,
    time_seconds: f32,
) -> Result<[f32; 4], String> {
    if track.times.is_empty()
        || track.times.len() != track.values.len()
        || track.times.iter().any(|value| !value.is_finite())
        || track.times.windows(2).any(|pair| pair[0] >= pair[1])
        || track
            .values
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
    {
        return Err("animation track has invalid keyframe data".to_owned());
    }
    if !time_seconds.is_finite() {
        return Err("animation sample time must be finite".to_owned());
    }
    if track.interpolation == AnimationInterpolation::CubicSpline
        && (track.in_tangents.len() != track.times.len()
            || track.out_tangents.len() != track.times.len()
            || track
                .in_tangents
                .iter()
                .flatten()
                .chain(track.out_tangents.iter().flatten())
                .any(|value| !value.is_finite()))
    {
        return Err("cubic-spline track has invalid tangent data".to_owned());
    }
    let last = track.times.len() - 1;
    if time_seconds <= track.times[0] {
        return normalize_track_value(track.property, track.values[0]);
    }
    if time_seconds >= track.times[last] {
        return normalize_track_value(track.property, track.values[last]);
    }
    let right = track.times.partition_point(|&time| time <= time_seconds);
    let left = right - 1;
    let t0 = track.times[left];
    let t1 = track.times[right];
    let dt = t1 - t0;
    let t = ((time_seconds - t0) / dt).clamp(0.0, 1.0);
    let value = match track.interpolation {
        AnimationInterpolation::Step => track.values[left],
        AnimationInterpolation::Linear if track.property == AnimationProperty::Rotation => {
            let a = quat_from_xyzw(track.values[left]);
            let b = quat_from_xyzw(track.values[right]);
            a.slerp(b, t).normalize().to_array()
        }
        AnimationInterpolation::Linear => (Vec4::from_array(track.values[left])
            + (Vec4::from_array(track.values[right]) - Vec4::from_array(track.values[left])) * t)
            .to_array(),
        AnimationInterpolation::CubicSpline => {
            let t2 = t * t;
            let t3 = t2 * t;
            let p0 = Vec4::from_array(track.values[left]);
            let p1 = Vec4::from_array(track.values[right]);
            let m0 = Vec4::from_array(track.out_tangents[left]) * dt;
            let m1 = Vec4::from_array(track.in_tangents[right]) * dt;
            (p0 * (2.0 * t3 - 3.0 * t2 + 1.0)
                + m0 * (t3 - 2.0 * t2 + t)
                + p1 * (-2.0 * t3 + 3.0 * t2)
                + m1 * (t3 - t2))
                .to_array()
        }
    };
    normalize_track_value(track.property, value)
}

fn quat_from_xyzw(value: [f32; 4]) -> Quat {
    let quat = Quat::from_xyzw(value[0], value[1], value[2], value[3]);
    if quat.length_squared() > 1.0e-12 && quat.is_finite() {
        quat.normalize()
    } else {
        Quat::IDENTITY
    }
}

fn normalize_track_value(property: AnimationProperty, value: [f32; 4]) -> Result<[f32; 4], String> {
    if value.iter().any(|value| !value.is_finite()) {
        return Err("animation produced a non-finite transform value".to_owned());
    }
    Ok(if property == AnimationProperty::Rotation {
        quat_from_xyzw(value).to_array()
    } else {
        value
    })
}

/// Evaluate an imported clip into one palette per model-node placement.
/// Palette matrices are mesh-local: inverse(mesh-node global) * joint global * inverse bind.
pub fn evaluate_skeletal_pose(
    model: &ModelAsset,
    clip: &SkeletalAnimationClip,
    time_seconds: f32,
    model_asset_id: u64,
    model_generation: u32,
) -> Result<SkeletalPose, String> {
    if model.skeleton_nodes.is_empty() {
        return Err("model contains no glTF node hierarchy for skeletal animation".to_owned());
    }
    let mut translations: Vec<[f32; 3]> = model
        .skeleton_nodes
        .iter()
        .map(|node| node.translation)
        .collect();
    let mut rotations: Vec<[f32; 4]> = model
        .skeleton_nodes
        .iter()
        .map(|node| node.rotation)
        .collect();
    let mut scales: Vec<[f32; 3]> = model.skeleton_nodes.iter().map(|node| node.scale).collect();
    for track in &clip.tracks {
        let target = track.node;
        if target >= model.skeleton_nodes.len() {
            return Err(format!("animation targets missing node {target}"));
        }
        let value = sample_animation_track(track, time_seconds)?;
        match track.property {
            AnimationProperty::Translation => translations[target] = [value[0], value[1], value[2]],
            AnimationProperty::Rotation => rotations[target] = value,
            AnimationProperty::Scale => scales[target] = [value[0], value[1], value[2]],
        }
    }

    let mut local = Vec::with_capacity(model.skeleton_nodes.len());
    for index in 0..model.skeleton_nodes.len() {
        let translation = glam::Vec3::from_array(translations[index]);
        let scale = glam::Vec3::from_array(scales[index]);
        let rotation = quat_from_xyzw(rotations[index]);
        if !translation.is_finite() || !scale.is_finite() {
            return Err(format!("node {index} has a non-finite animated transform"));
        }
        local.push(Mat4::from_scale_rotation_translation(
            scale,
            rotation,
            translation,
        ));
    }

    fn resolve_global(
        index: usize,
        nodes: &[ribbon_plugin_gltf::SkeletonNode],
        local: &[Mat4],
        states: &mut [u8],
        globals: &mut [Mat4],
    ) -> Result<Mat4, String> {
        match states[index] {
            2 => return Ok(globals[index]),
            1 => return Err(format!("cycle detected in glTF skeleton at node {index}")),
            _ => {}
        }
        states[index] = 1;
        let value = if let Some(parent) = nodes[index].parent {
            if parent >= nodes.len() {
                return Err(format!("node {index} references missing parent {parent}"));
            }
            resolve_global(parent, nodes, local, states, globals)? * local[index]
        } else {
            local[index]
        };
        if !value.to_cols_array().iter().all(|value| value.is_finite()) {
            return Err(format!(
                "node {index} produced a non-finite global transform"
            ));
        }
        globals[index] = value;
        states[index] = 2;
        Ok(value)
    }

    let mut states = vec![0_u8; local.len()];
    let mut globals = vec![Mat4::IDENTITY; local.len()];
    for index in 0..local.len() {
        let _ = resolve_global(
            index,
            &model.skeleton_nodes,
            &local,
            &mut states,
            &mut globals,
        )?;
    }

    let mut node_palettes = vec![None; model.nodes.len()];
    for (placement_index, placement) in model.nodes.iter().enumerate() {
        let Some(skin_index) = placement.skin else {
            continue;
        };
        let skin = model.skins.get(skin_index).ok_or_else(|| {
            format!(
                "model node {} references missing skin {skin_index}",
                placement.node
            )
        })?;
        let mesh_global = *globals.get(placement.node).ok_or_else(|| {
            format!(
                "model node references missing skeleton node {}",
                placement.node
            )
        })?;
        let determinant = mesh_global.determinant();
        if !determinant.is_finite() || determinant.abs() < 1.0e-8 {
            return Err(format!(
                "skinned mesh node {} has a singular transform",
                placement.node
            ));
        }
        let inverse_mesh = mesh_global.inverse();
        let mut palette = Vec::with_capacity(skin.joints.len());
        for (joint_index, inverse_bind) in skin.joints.iter().zip(&skin.inverse_bind_matrices) {
            let joint_global = *globals
                .get(*joint_index)
                .ok_or_else(|| format!("skin references missing joint node {joint_index}"))?;
            let inverse_bind = Mat4::from_cols_array_2d(inverse_bind);
            let matrix = inverse_mesh * joint_global * inverse_bind;
            if !matrix.to_cols_array().iter().all(|value| value.is_finite()) {
                return Err(format!(
                    "skin {skin_index} produced a non-finite joint matrix"
                ));
            }
            palette.push(matrix);
        }
        node_palettes[placement_index] = Some(Arc::<[Mat4]>::from(palette.into_boxed_slice()));
    }
    Ok(SkeletalPose {
        model_asset_id,
        model_generation,
        node_global_transforms: Arc::<[Mat4]>::from(globals.into_boxed_slice()),
        node_palettes: Arc::<[Option<Arc<[Mat4]>>]>::from(node_palettes.into_boxed_slice()),
    })
}

/// Advance skeletal players and publish immutable poses for the render-extraction stage.
/// CPU asset misses are transient and leave the last valid pose untouched; no synchronous waits occur.
pub fn advance_skeletal_animation_players(world: &mut World) {
    if world.try_resource::<ribbon_assets::AssetServer>().is_none() {
        return;
    }
    let dt = world
        .try_resource::<Time>()
        .map_or(0.0, |time| time.delta.as_secs_f32().max(0.0));
    world.resource_scope::<ribbon_assets::AssetServer, _>(|world, assets| {
        let instances: Vec<(
            Entity,
            ModelInstance,
            Option<SkeletalAnimationPlayer>,
            Option<SkeletalPose>,
        )> = world
            .query::<(
                &ModelInstance,
                Option<&SkeletalAnimationPlayer>,
                Option<&SkeletalPose>,
            )>()
            .iter()
            .map(|(entity, (model, player, pose))| (entity, *model, player.copied(), pose.cloned()))
            .collect();

        for (entity, model_instance, player, current_pose) in instances {
            let Some(model) = assets.get(model_instance.model) else {
                continue;
            };
            if !model.nodes.iter().any(|node| node.skin.is_some())
                && model.skeletal_animations.is_empty()
            {
                continue;
            }
            let generation_matches = current_pose.as_ref().is_some_and(|pose| {
                pose.model_asset_id == model_instance.model.id()
                    && pose.model_generation == model_instance.model.generation()
            });

            if let Some(mut player) = player {
                let Some(clip) = model.skeletal_animations.get(player.clip_index) else {
                    // Invalid clip selections do not silently switch animations or erase the last
                    // valid pose. Asset-generation mismatch is still rejected by render extraction.
                    continue;
                };
                let duration = clip.duration_seconds;
                if !duration.is_finite() || duration < 0.0 {
                    continue;
                }
                if !player.elapsed_seconds.is_finite() {
                    player.elapsed_seconds = 0.0;
                }
                if !player.paused && dt.is_finite() && dt > 0.0 {
                    let speed = if player.playback_speed.is_finite() {
                        player.playback_speed
                    } else {
                        0.0
                    };
                    let advanced = player.elapsed_seconds + dt * speed;
                    player.elapsed_seconds = if advanced.is_finite() { advanced } else { 0.0 };
                }
                let sample_time = if duration <= f32::EPSILON {
                    player.elapsed_seconds = 0.0;
                    0.0
                } else if player.looping {
                    player.elapsed_seconds = player.elapsed_seconds.rem_euclid(duration);
                    player.elapsed_seconds
                } else {
                    player.elapsed_seconds = player.elapsed_seconds.clamp(0.0, duration);
                    player.elapsed_seconds
                };
                if let Ok(pose) = evaluate_skeletal_pose(
                    model,
                    clip,
                    sample_time,
                    model_instance.model.id(),
                    model_instance.model.generation(),
                ) {
                    let _ = world.insert(entity, pose);
                }
                let _ = world.insert(entity, player);
            } else if !generation_matches {
                // Models without an explicit player still use the imported bind pose. This avoids
                // drawing weighted vertices with zero or stale joint matrices.
                if let Ok(pose) = evaluate_skeletal_pose(
                    model,
                    &SkeletalAnimationClip::default(),
                    0.0,
                    model_instance.model.id(),
                    model_instance.model.generation(),
                ) {
                    let _ = world.insert(entity, pose);
                }
            }
        }
    });
}

/// RON loader for `.ranim` sprite animation clips.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpriteAnimationClipLoader;

impl AssetLoader for SpriteAnimationClipLoader {
    type Asset = SpriteAnimationClip;

    fn extensions(&self) -> &[&str] {
        &["ranim"]
    }

    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
        let clip: SpriteAnimationClip = ron::de::from_bytes(bytes).map_err(|error| {
            LoadError::new(
                LoadErrorKind::Decode,
                &context.path,
                format!("invalid RON sprite animation clip: {error}"),
            )
        })?;
        clip.validate()
            .map_err(|message| LoadError::new(LoadErrorKind::Decode, &context.path, message))?;
        Ok(clip)
    }

    fn estimated_size_bytes(&self, asset: &Self::Asset) -> u64 {
        asset.estimated_size_bytes()
    }

    fn memory_breakdown(&self, asset: &Self::Asset) -> AssetMemoryBreakdown {
        AssetMemoryBreakdown {
            animation_bytes: asset.estimated_size_bytes(),
            ..AssetMemoryBreakdown::default()
        }
    }
}

/// Advance every animator by the current app delta, preserving fractional frames.
pub fn advance_sprite_animators(world: &mut World) {
    let dt = world.resource::<Time>().delta.as_secs_f32();
    for (_, animator) in world.query_mut::<&mut SpriteAnimator>() {
        animator.frame += dt * animator.fps;
    }
}

/// Advance asset-backed clips. Missing/not-yet-loaded clips are left unchanged; AssetServer
/// completion remains asynchronous and no wait is introduced into the game loop.
pub fn advance_sprite_clip_players(world: &mut World) {
    let dt = world.resource::<Time>().delta.as_secs_f32();
    if !dt.is_finite() || dt <= 0.0 || world.try_resource::<ribbon_assets::AssetServer>().is_none()
    {
        return;
    }
    world.resource_scope::<ribbon_assets::AssetServer, _>(|world, assets| {
        for (_, player) in world.query_mut::<&mut SpriteClipPlayer>() {
            let Some(clip) = assets.get(player.clip) else {
                continue;
            };
            if clip.validate().is_err() || player.completed && !clip.looping {
                continue;
            }
            if !player.elapsed_frames.is_finite() || player.elapsed_frames < 0.0 {
                player.elapsed_frames = 0.0;
            }
            player.elapsed_frames += dt * clip.frames_per_second;
            if !player.elapsed_frames.is_finite() {
                player.elapsed_frames = 0.0;
            }
            let frame_count = clip.frame_indices.len();
            if clip.looping && player.elapsed_frames >= frame_count as f32 {
                // Keep long-running loops numerically stable rather than allowing an
                // unbounded frame accumulator to lose single-frame precision.
                player.elapsed_frames %= frame_count as f32;
            }
            let frame_index = player.elapsed_frames.floor().max(0.0) as usize;
            let selected = if clip.looping {
                frame_index.min(frame_count - 1)
            } else {
                if frame_index >= frame_count {
                    player.completed = true;
                }
                frame_index.min(frame_count - 1)
            };
            player.current_frame = clip.frame_indices[selected];
        }
    });
}

#[derive(Default)]
struct AnimationAssetLoaderInstalled(bool);

fn ensure_animation_asset_loader(world: &mut World) {
    if world
        .try_resource::<AnimationAssetLoaderInstalled>()
        .is_none()
    {
        world.insert_resource(AnimationAssetLoaderInstalled::default());
    }
    if world
        .try_resource::<AnimationAssetLoaderInstalled>()
        .is_some_and(|installed| installed.0)
        || world.try_resource::<ribbon_assets::AssetServer>().is_none()
    {
        return;
    }
    world.resource_scope::<ribbon_assets::AssetServer, _>(|world, assets| {
        assets.register_loader(SpriteAnimationClipLoader);
        world.resource_mut::<AnimationAssetLoaderInstalled>().0 = true;
    });
}

/// Registers sprite and skeletal animation advancement plus the `.ranim` sprite clip loader.
/// glTF skeletal clips are embedded in `ModelAsset`; their CPU evaluation publishes immutable
/// palettes for the renderer and never performs GPU work from this plugin. The sprite loader is
/// installed as soon as AssetPlugin has placed AssetServer in the World.
pub struct AnimationPlugin;

impl Plugin for AnimationPlugin {
    fn name(&self) -> &'static str {
        "AnimationPlugin"
    }

    fn build(&self, app: &mut App) {
        if app
            .world
            .try_resource::<AnimationAssetLoaderInstalled>()
            .is_none()
        {
            app.insert_resource(AnimationAssetLoaderInstalled::default());
        }
        let animation_loader_installed = app
            .world
            .try_resource::<ribbon_assets::AssetServer>()
            .is_some();
        if animation_loader_installed {
            app.world
                .resource_mut::<ribbon_assets::AssetServer>()
                .register_loader(SpriteAnimationClipLoader);
        }
        if animation_loader_installed {
            app.world.resource_mut::<AnimationAssetLoaderInstalled>().0 = true;
        }
        app.add_system(Stage::First, ensure_animation_asset_loader);
        app.add_system(Stage::Update, advance_sprite_animators);
        app.add_system(Stage::Update, advance_sprite_clip_players);
        app.add_system(Stage::Update, advance_skeletal_animation_players);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        advance_sprite_animators, sample_animation_track, AnimationInterpolation,
        AnimationProperty, AnimationTrack, SpriteAnimator,
    };
    use ribbon_app::{Time, World};
    use std::time::Duration;

    #[test]
    fn advances_fractional_sprite_frames_from_virtual_time() {
        let mut world = World::new();
        world.insert_resource(Time {
            delta: Duration::from_millis(250),
            ..Time::default()
        });
        let entity = world.spawn((SpriteAnimator {
            state: 3,
            frame: 2.0,
            fps: 8.0,
        },));
        advance_sprite_animators(&mut world);
        assert_eq!(world.get::<SpriteAnimator>(entity).unwrap().frame, 4.0);
        assert_eq!(world.get::<SpriteAnimator>(entity).unwrap().state, 3);
    }

    #[test]
    fn sprite_animation_clip_asset_loads_and_reports_animation_memory() {
        use super::{SpriteAnimationClip, SpriteAnimationClipLoader};
        use ribbon_assets::{AssetServer, MemorySource};

        let source = MemorySource::default();
        source.insert(
            "animations/run.ranim",
            r#"(name: "run", frame_indices: [0, 1, 2, 3], frames_per_second: 12.0, looping: true)"#
                .as_bytes()
                .to_vec(),
        );
        let mut assets = AssetServer::default();
        assets.add_source(0, source);
        assets.register_loader(SpriteAnimationClipLoader);
        let handle = assets
            .load::<SpriteAnimationClip>("animations/run.ranim")
            .unwrap();
        let expected_animation_bytes = {
            let clip = assets
                .get(handle)
                .expect("clip is available after synchronous load");
            assert_eq!(clip.frame_indices, vec![0, 1, 2, 3]);
            assert!(clip.looping);
            clip.estimated_size_bytes()
        };
        let mut world = World::new();
        world.insert_resource(Time {
            delta: Duration::from_millis(250),
            ..Time::default()
        });
        world.insert_resource(assets);
        let player_entity = world.spawn((super::SpriteClipPlayer::new(handle),));
        super::advance_sprite_clip_players(&mut world);
        assert_eq!(
            world
                .get::<super::SpriteClipPlayer>(player_entity)
                .unwrap()
                .current_frame,
            3
        );
        let assets = world
            .remove_resource::<AssetServer>()
            .expect("asset server restored");
        let stats = assets.cpu_memory_stats();
        assert_eq!(stats.animation_bytes_estimate, expected_animation_bytes);
        assert_eq!(stats.other_bytes_estimate, 0);
    }

    #[test]
    fn sprite_animation_clip_loader_rejects_empty_or_invalid_timing() {
        use super::SpriteAnimationClip;
        assert!(SpriteAnimationClip {
            name: "idle".into(),
            frame_indices: vec![],
            frames_per_second: 10.0,
            looping: true,
        }
        .validate()
        .is_err());
        assert!(SpriteAnimationClip {
            name: "idle".into(),
            frame_indices: vec![0],
            frames_per_second: f32::NAN,
            looping: true,
        }
        .validate()
        .is_err());
    }

    #[test]
    fn skeletal_tracks_support_step_linear_and_cubic_spline_interpolation() {
        let track = |interpolation, values, ins, outs| AnimationTrack {
            node: 0,
            property: AnimationProperty::Translation,
            interpolation,
            times: vec![0.0, 1.0],
            values,
            in_tangents: ins,
            out_tangents: outs,
        };
        let step = track(
            AnimationInterpolation::Step,
            vec![[0.0, 0.0, 0.0, 0.0], [4.0, 0.0, 0.0, 0.0]],
            vec![],
            vec![],
        );
        assert_eq!(sample_animation_track(&step, 0.5).unwrap()[0], 0.0);
        let linear = track(
            AnimationInterpolation::Linear,
            vec![[0.0, 0.0, 0.0, 0.0], [4.0, 0.0, 0.0, 0.0]],
            vec![],
            vec![],
        );
        assert!((sample_animation_track(&linear, 0.25).unwrap()[0] - 1.0).abs() < 1.0e-6);
        let cubic = track(
            AnimationInterpolation::CubicSpline,
            vec![[0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]],
            vec![[0.0; 4], [0.0; 4]],
            vec![[0.0; 4], [0.0; 4]],
        );
        assert!((sample_animation_track(&cubic, 0.5).unwrap()[0] - 0.5).abs() < 1.0e-6);
    }

    #[test]
    fn cubic_spline_single_key_clip_is_a_constant_value() {
        let one_key = AnimationTrack {
            node: 0,
            property: AnimationProperty::Translation,
            interpolation: AnimationInterpolation::CubicSpline,
            times: vec![0.25],
            values: vec![[3.0, 2.0, 1.0, 0.0]],
            in_tangents: vec![[0.0; 4]],
            out_tangents: vec![[0.0; 4]],
        };
        assert_eq!(
            sample_animation_track(&one_key, -1.0).unwrap(),
            [3.0, 2.0, 1.0, 0.0]
        );
        assert_eq!(
            sample_animation_track(&one_key, 10.0).unwrap(),
            [3.0, 2.0, 1.0, 0.0]
        );
    }

    #[test]
    fn skeletal_pose_evaluation_builds_mesh_local_joint_palettes() {
        use super::evaluate_skeletal_pose;
        use glam::Mat4;
        use ribbon_plugin_gltf::{
            ModelAsset, ModelNode, SkeletalAnimationClip, SkeletonNode, SkinData,
        };

        let model = ModelAsset {
            nodes: vec![ModelNode {
                primitive: 0,
                transform: Mat4::IDENTITY,
                node: 0,
                skin: Some(0),
            }],
            skeleton_nodes: vec![
                SkeletonNode {
                    parent: None,
                    translation: [0.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0; 3],
                },
                SkeletonNode {
                    parent: Some(0),
                    translation: [0.0, 1.0, 0.0],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                    scale: [1.0; 3],
                },
            ],
            skins: vec![SkinData {
                joints: vec![1],
                inverse_bind_matrices: vec![Mat4::IDENTITY.to_cols_array_2d()],
            }],
            ..ModelAsset::default()
        };
        let clip = SkeletalAnimationClip {
            name: "move_joint".into(),
            duration_seconds: 1.0,
            tracks: vec![AnimationTrack {
                node: 1,
                property: AnimationProperty::Translation,
                interpolation: AnimationInterpolation::Linear,
                times: vec![0.0, 1.0],
                values: vec![[0.0, 1.0, 0.0, 0.0], [0.0, 3.0, 0.0, 0.0]],
                in_tangents: vec![],
                out_tangents: vec![],
            }],
        };
        let pose = evaluate_skeletal_pose(&model, &clip, 0.5, 77, 3).unwrap();
        assert_eq!(pose.model_asset_id, 77);
        assert_eq!(pose.model_generation, 3);
        let matrix = pose.node_palettes[0].as_ref().unwrap()[0];
        assert!((matrix.w_axis.y - 2.0).abs() < 1.0e-5);
        assert!((pose.node_global_transforms[1].w_axis.y - 2.0).abs() < 1.0e-5);
    }

    #[test]
    fn skeletal_pose_bind_pose_is_available_without_an_animation_player() {
        use super::evaluate_skeletal_pose;
        use glam::Mat4;
        use ribbon_plugin_gltf::{
            ModelAsset, ModelNode, SkeletalAnimationClip, SkeletonNode, SkinData,
        };
        let model = ModelAsset {
            nodes: vec![ModelNode {
                primitive: 0,
                transform: Mat4::IDENTITY,
                node: 0,
                skin: Some(0),
            }],
            skeleton_nodes: vec![SkeletonNode {
                parent: None,
                translation: [0.0; 3],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
            }],
            skins: vec![SkinData {
                joints: vec![0],
                inverse_bind_matrices: vec![Mat4::IDENTITY.to_cols_array_2d()],
            }],
            ..ModelAsset::default()
        };
        let pose =
            evaluate_skeletal_pose(&model, &SkeletalAnimationClip::default(), 0.0, 1, 0).unwrap();
        assert_eq!(pose.node_palettes[0].as_ref().unwrap().len(), 1);
        assert!(pose.node_palettes[0].as_ref().unwrap()[0].abs_diff_eq(Mat4::IDENTITY, 1.0e-6));
    }
}
