//! Deterministic entity and sprite animation data and update systems.
//!
//! This plugin advances normalized sprite frames only; game-specific animation states remain game-owned.

use ribbon_app::{App, Stage, Time};
use ribbon_assets::{AssetLoader, AssetMemoryBreakdown, LoadContext, LoadError, LoadErrorKind};
use ribbon_ecs::World;
use ribbon_plugins::Plugin;
use serde::{Deserialize, Serialize};

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

/// Registers sprite animation advancement and the `.ranim` asset loader. The loader is installed
/// as soon as AssetPlugin has placed AssetServer in the World, regardless of plugin build order.
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
    }
}

#[cfg(test)]
mod tests {
    use super::{advance_sprite_animators, SpriteAnimator};
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
}
