//! ECS adapters for moving emitters, transform-backed listeners, lightweight occlusion,
//! and trigger-volume environmental acoustics.
//!
//! `AudioZoneVolume` is an axis-aligned box centered on `GlobalTransform.translation`.
//! Physics engines can provide occlusion through `AudioOcclusionQuery`; the audio plugin does
//! not depend on a particular collision system. Queries are staggered across frames.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ribbon_ecs::{Entity, World};
use ribbon_scene::GlobalTransform;

use crate::spatial::{self, AcousticZone, Emitter3d};
use crate::{
    AudioBus, AudioEffect, AudioListener, AudioOutput, AudioVec3, SoundId, SpatialAudioListener,
    VoiceId,
};

const OCCLUSION_REFRESH_STRIDE: u64 = 4;
const OCCLUSION_CACHE_MAX_AGE: u64 = 60;
const EFFECT_TWEEN_SECONDS: f32 = 0.12;

/// A looping or one-shot sound attached to an entity's world transform.
///
/// Requires `ribbon_scene::ScenePlugin` (or another system) to keep `GlobalTransform` current.
/// Audio coordinates follow Ribbon's right-handed world convention: +Z is up and listener
/// forward follows `Angles { psi, phi }` through `ribbon_math::forward_vec`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioEmitter {
    pub sound: SoundId,
    pub gain: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    pub rolloff: f32,
    pub velocity: AudioVec3,
    pub looping: bool,
    pub priority: u8,
    pub bus: AudioBus,
    /// Minimum occlusion applied even if no query provider is installed, in `[0, 1]`.
    pub occlusion: f32,
}

impl Default for AudioEmitter {
    fn default() -> Self {
        Self {
            sound: SoundId(0),
            gain: 1.0,
            min_distance: 1.0,
            max_distance: 100.0,
            rolloff: 1.0,
            velocity: AudioVec3::ZERO,
            looping: true,
            priority: 128,
            bus: AudioBus::Sfx,
            occlusion: 0.0,
        }
    }
}

/// Output from a physics-backed source-to-listener visibility test.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioOcclusionResult {
    /// 0 means clear line of sight; 1 means heavily occluded.
    pub occlusion: f32,
}

impl AudioOcclusionResult {
    pub fn sanitized(self) -> Self {
        Self {
            occlusion: if self.occlusion.is_finite() {
                self.occlusion.clamp(0.0, 1.0)
            } else {
                0.0
            },
        }
    }
}

/// Adapter point for physics/scene collision systems. Implementations should be fast and must
/// not block on asset or device operations. The audio plugin staggers calls to avoid ray storms.
pub trait AudioOcclusionQuery: Send + Sync + 'static {
    fn query_occlusion(&self, source: AudioVec3, listener: AudioVec3) -> AudioOcclusionResult;
}

/// Optional world resource wrapping an application-provided occlusion query.
#[derive(Default, Clone)]
pub struct AudioOcclusionProvider(pub Option<Arc<dyn AudioOcclusionQuery>>);

/// Environmental acoustic region centered on its entity's `GlobalTransform`.
///
/// Zones are axis-aligned boxes. Overlap resolves by higher `priority`, then greater blend
/// weight, then lower stable `id`; transition filtering is applied in the Kira mixer backend.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioZoneVolume {
    pub id: u32,
    pub priority: i16,
    pub half_extents: AudioVec3,
    /// Additional distance outside the box over which the zone smoothly fades out.
    pub blend_distance: f32,
    pub acoustics: AcousticZone,
}

impl AudioZoneVolume {
    pub fn sanitized(self) -> Self {
        Self {
            id: self.id,
            priority: self.priority,
            half_extents: AudioVec3::new(
                finite_clamp(self.half_extents.x.abs(), 0.01, 1_000_000.0, 1.0),
                finite_clamp(self.half_extents.y.abs(), 0.01, 1_000_000.0, 1.0),
                finite_clamp(self.half_extents.z.abs(), 0.01, 1_000_000.0, 1.0),
            ),
            blend_distance: finite_clamp(self.blend_distance, 0.0, 10_000.0, 0.0),
            acoustics: self.acoustics.sanitized(),
        }
    }

    fn weight(self, center: AudioVec3, point: AudioVec3) -> f32 {
        let zone = self.sanitized();
        let outside_x = (point.x - center.x).abs() - zone.half_extents.x;
        let outside_y = (point.y - center.y).abs() - zone.half_extents.y;
        let outside_z = (point.z - center.z).abs() - zone.half_extents.z;
        let outside = outside_x
            .max(0.0)
            .max(outside_y.max(0.0))
            .max(outside_z.max(0.0));
        if outside <= 0.0 {
            1.0
        } else if zone.blend_distance > 0.0 && outside < zone.blend_distance {
            1.0 - outside / zone.blend_distance
        } else {
            0.0
        }
    }
}

#[derive(Clone, Copy)]
struct EmitterSnapshot {
    entity: Entity,
    emitter: AudioEmitter,
    transform: GlobalTransform,
}

#[derive(Clone, Copy)]
struct ZoneSnapshot {
    entity: Entity,
    zone: AudioZoneVolume,
    center: AudioVec3,
}

#[derive(Clone, Copy)]
struct RuntimeEmitter {
    sound: SoundId,
    looping: bool,
    bus: AudioBus,
    priority: u8,
    voice: Option<VoiceId>,
}

#[derive(Clone, Copy)]
struct CachedOcclusion {
    target: f32,
    smoothed: f32,
    last_refresh: u64,
}

#[derive(Default)]
pub(super) struct AudioEmitterRuntime {
    frame: u64,
    emitters: HashMap<Entity, RuntimeEmitter>,
    occlusion: HashMap<Entity, CachedOcclusion>,
    zone_signature: Option<(u32, u64, i32, i16, i16, i16)>,
}

/// Synchronize transform-attached emitters and blend the active listener's acoustic zone.
/// A stable voice handle is created once per emitter; transform changes update that voice instead
/// of repeatedly starting new players. Components without `GlobalTransform` are ignored.
pub fn audio_emitter_system(world: &mut World) {
    if world.try_resource::<AudioEmitterRuntime>().is_none() {
        return;
    }
    let emitters: Vec<EmitterSnapshot> = world
        .query::<(&AudioEmitter, &GlobalTransform)>()
        .iter()
        .map(|(entity, (emitter, transform))| EmitterSnapshot {
            entity,
            emitter: *emitter,
            transform: *transform,
        })
        .collect();
    let zones: Vec<ZoneSnapshot> = world
        .query::<(&AudioZoneVolume, &GlobalTransform)>()
        .iter()
        .map(|(entity, (zone, transform))| ZoneSnapshot {
            entity,
            zone: *zone,
            center: AudioVec3::new(
                transform.translation.x,
                transform.translation.y,
                transform.translation.z,
            ),
        })
        .collect();
    let listener = world
        .query::<(&SpatialAudioListener, &GlobalTransform)>()
        .iter()
        .min_by_key(|(entity, _)| entity.to_bits())
        .map(|(_, (listener, transform))| {
            let forward = ribbon_math::forward_vec(transform.rotation);
            crate::Listener3d {
                position: AudioVec3::new(
                    transform.translation.x,
                    transform.translation.y,
                    transform.translation.z,
                ),
                forward: AudioVec3::new(forward.x, forward.y, forward.z),
                up: AudioVec3::new(0.0, 0.0, 1.0),
                velocity: listener.0.velocity,
            }
        })
        .or_else(|| {
            world
                .query::<&SpatialAudioListener>()
                .iter()
                .min_by_key(|(entity, _)| entity.to_bits())
                .map(|(_, listener)| listener.0)
        })
        .or_else(|| {
            world
                .query::<&AudioListener>()
                .iter()
                .min_by_key(|(entity, _)| entity.to_bits())
                .map(|(_, listener)| crate::Listener3d {
                    position: AudioVec3::new(
                        listener.position.x,
                        listener.position.y,
                        listener.position.z,
                    ),
                    forward: AudioVec3::new(listener.yaw.sin(), listener.yaw.cos(), 0.0),
                    up: AudioVec3::new(0.0, 0.0, 1.0),
                    velocity: AudioVec3::ZERO,
                })
        });
    let occlusion_provider = world
        .try_resource::<AudioOcclusionProvider>()
        .and_then(|provider| provider.0.clone());

    let frame = world.resource::<AudioEmitterRuntime>().frame;
    let mut active_entities = HashSet::with_capacity(emitters.len());
    active_entities.extend(emitters.iter().map(|emitter| emitter.entity));

    world.resource_scope::<AudioEmitterRuntime, _>(|world, runtime| {
        world.resource_scope::<AudioOutput, _>(|_, output| {
            let Some(sink) = output.0.as_deref_mut() else {
                return;
            };

            // Stop voices for despawned or detached emitters and drop their cached query data.
            let stale: Vec<Entity> = runtime
                .emitters
                .keys()
                .filter(|entity| !active_entities.contains(entity))
                .copied()
                .collect();
            for entity in stale {
                if let Some(active) = runtime.emitters.remove(&entity) {
                    if let Some(voice) = active.voice {
                        sink.stop_managed_emitter(voice, 0.1);
                    }
                }
                runtime.occlusion.remove(&entity);
            }

            let mut chosen_zone: Option<(i16, f32, u64, u32, AcousticZone)> = None;
            if let Some(listener) = listener {
                for candidate in &zones {
                    let zone = candidate.zone.sanitized();
                    let weight = candidate.zone.weight(candidate.center, listener.position);
                    if weight <= 0.0 {
                        continue;
                    }
                    let entity_bits = candidate.entity.to_bits().get();
                    let should_replace = chosen_zone.as_ref().is_none_or(
                        |(priority, old_weight, old_entity, old_id, _)| {
                            zone.priority > *priority
                                || (zone.priority == *priority && weight > *old_weight)
                                || (zone.priority == *priority
                                    && (weight - *old_weight).abs() < f32::EPSILON
                                    && (entity_bits, zone.id) < (*old_entity, *old_id))
                        },
                    );
                    if should_replace {
                        chosen_zone =
                            Some((zone.priority, weight, entity_bits, zone.id, zone.acoustics));
                    }
                }
            }
            let zone_signature = chosen_zone.map(|(_, weight, entity_bits, id, zone)| {
                // Quantization prevents per-frame queue traffic while still making the blend smooth.
                let cutoff = 20_000.0 + (zone.low_pass_hz - 20_000.0) * weight;
                let wet = zone.wet_mix * weight;
                (
                    id,
                    entity_bits,
                    (cutoff / 200.0).round() as i32,
                    (wet * 20.0).round() as i16,
                    ((1.0 + (zone.ambience_gain - 1.0) * weight) * 100.0).round() as i16,
                    (zone.reverb_seconds * weight * 10.0).round() as i16,
                )
            });
            if zone_signature != runtime.zone_signature {
                let effects = chosen_zone
                    .map(|(_, weight, _, _, zone)| {
                        let cutoff = 20_000.0 + (zone.low_pass_hz - 20_000.0) * weight;
                        let wet = zone.wet_mix * weight;
                        vec![
                            AudioEffect::LowPass { cutoff_hz: cutoff },
                            AudioEffect::Reverb {
                                wet,
                                decay_seconds: zone.reverb_seconds * weight,
                            },
                        ]
                    })
                    .unwrap_or_default();
                sink.set_bus_effects(AudioBus::Sfx, &effects);
                sink.set_bus_effects(AudioBus::Ambience, &effects);
                let ambience_scale = chosen_zone
                    .map(|(_, weight, _, _, zone)| 1.0 + (zone.ambience_gain - 1.0) * weight)
                    .unwrap_or(1.0);
                sink.set_bus_zone_gain(AudioBus::Ambience, ambience_scale, EFFECT_TWEEN_SECONDS);
                runtime.zone_signature = zone_signature;
            }

            let Some(listener) = listener else {
                runtime.frame = runtime.frame.saturating_add(1);
                return;
            };

            for snapshot in &emitters {
                let source = AudioVec3::new(
                    snapshot.transform.translation.x,
                    snapshot.transform.translation.y,
                    snapshot.transform.translation.z,
                );
                let mut occlusion = finite_clamp(snapshot.emitter.occlusion, 0.0, 1.0, 0.0);
                if let Some(provider) = &occlusion_provider {
                    let refresh = snapshot.entity.to_bits().get() % OCCLUSION_REFRESH_STRIDE
                        == frame % OCCLUSION_REFRESH_STRIDE;
                    let cached =
                        runtime
                            .occlusion
                            .entry(snapshot.entity)
                            .or_insert(CachedOcclusion {
                                target: occlusion,
                                smoothed: occlusion,
                                last_refresh: frame,
                            });
                    if refresh {
                        cached.target = provider
                            .query_occlusion(source, listener.position)
                            .sanitized()
                            .occlusion
                            .max(occlusion);
                        cached.last_refresh = frame;
                    } else if frame.saturating_sub(cached.last_refresh) > OCCLUSION_CACHE_MAX_AGE {
                        cached.target = occlusion;
                    }
                    cached.smoothed += (cached.target - cached.smoothed) * 0.25;
                    occlusion = cached.smoothed.max(occlusion);
                }
                let emitter = Emitter3d {
                    position: source,
                    velocity: snapshot.emitter.velocity,
                    min_distance: snapshot.emitter.min_distance,
                    max_distance: snapshot.emitter.max_distance,
                    rolloff: snapshot.emitter.rolloff,
                    gain: snapshot.emitter.gain,
                    occlusion,
                };
                let spatial = spatial::calculate(listener, emitter, 343.0);
                let rate = spatial.doppler_ratio;
                let gain = spatial.gain;
                let entity = snapshot.entity;
                let sound = snapshot.emitter.sound;
                let entry = runtime.emitters.get(&entity).copied();
                match entry {
                    Some(active)
                        if active.sound == sound
                            && active.looping == snapshot.emitter.looping
                            && active.bus == snapshot.emitter.bus
                            && active.priority == snapshot.emitter.priority =>
                    {
                        if let Some(voice) = active.voice {
                            sink.update_managed_emitter(voice, gain, spatial.pan, rate, occlusion);
                        }
                    }
                    Some(active) => {
                        if let Some(voice) = active.voice {
                            sink.stop_managed_emitter(voice, 0.08);
                        }
                        let voice = sink.play_managed_emitter(crate::ManagedEmitterPlayback {
                            sound,
                            gain,
                            pan: spatial.pan,
                            playback_rate: rate,
                            looping: snapshot.emitter.looping,
                            priority: snapshot.emitter.priority,
                            bus: snapshot.emitter.bus,
                        });
                        runtime.emitters.insert(
                            entity,
                            RuntimeEmitter {
                                sound,
                                looping: snapshot.emitter.looping,
                                bus: snapshot.emitter.bus,
                                priority: snapshot.emitter.priority,
                                voice,
                            },
                        );
                    }
                    None => {
                        let voice = sink.play_managed_emitter(crate::ManagedEmitterPlayback {
                            sound,
                            gain,
                            pan: spatial.pan,
                            playback_rate: rate,
                            looping: snapshot.emitter.looping,
                            priority: snapshot.emitter.priority,
                            bus: snapshot.emitter.bus,
                        });
                        runtime.emitters.insert(
                            entity,
                            RuntimeEmitter {
                                sound,
                                looping: snapshot.emitter.looping,
                                bus: snapshot.emitter.bus,
                                priority: snapshot.emitter.priority,
                                voice,
                            },
                        );
                    }
                }
            }
            runtime.frame = runtime.frame.saturating_add(1);
        });
    });
}

fn finite_clamp(value: f32, min: f32, max: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AudioSink;

    #[test]
    fn zone_blending_has_full_weight_inside_and_fades_outside() {
        let zone = AudioZoneVolume {
            id: 7,
            priority: 1,
            half_extents: AudioVec3::new(2.0, 2.0, 2.0),
            blend_distance: 4.0,
            acoustics: AcousticZone {
                id: 7,
                wet_mix: 0.5,
                low_pass_hz: 1_000.0,
                reverb_seconds: 2.0,
                ambience_gain: 1.0,
            },
        };
        assert!((zone.weight(AudioVec3::ZERO, AudioVec3::new(1.0, 0.0, 0.0)) - 1.0).abs() < 1e-6);
        assert!((zone.weight(AudioVec3::ZERO, AudioVec3::new(4.0, 0.0, 0.0)) - 0.5).abs() < 1e-6);
        assert_eq!(
            zone.weight(AudioVec3::ZERO, AudioVec3::new(7.0, 0.0, 0.0)),
            0.0
        );
    }

    #[test]
    fn moving_emitter_reuses_one_voice_and_stops_when_despawned() {
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct State {
            starts: usize,
            updates: usize,
            stops: usize,
        }
        struct FakeSink(Arc<Mutex<State>>);
        impl AudioSink for FakeSink {
            fn play(&mut self, _sound: SoundId, _gain: f32, _pan: f32) {}
            fn set_ambience(&mut self, _id: Option<crate::AmbienceId>, _gain: f32) {}
            fn play_managed_emitter(
                &mut self,
                _playback: crate::ManagedEmitterPlayback,
            ) -> Option<VoiceId> {
                let mut state = self.0.lock().unwrap();
                state.starts += 1;
                Some(VoiceId(77))
            }
            fn update_managed_emitter(
                &mut self,
                _voice: VoiceId,
                _gain: f32,
                _pan: f32,
                _rate: f32,
                _occlusion: f32,
            ) {
                self.0.lock().unwrap().updates += 1;
            }
            fn stop_managed_emitter(&mut self, _voice: VoiceId, _fade_seconds: f32) {
                self.0.lock().unwrap().stops += 1;
            }
        }

        let state = Arc::new(Mutex::new(State::default()));
        let mut world = World::new();
        world.insert_resource(AudioEmitterRuntime::default());
        world.insert_resource(AudioOutput(Some(Box::new(FakeSink(Arc::clone(&state))))));
        world.insert_resource(AudioOcclusionProvider::default());
        world.spawn((
            SpatialAudioListener(crate::Listener3d::default()),
            GlobalTransform::default(),
        ));
        let emitter = world.spawn((
            AudioEmitter {
                sound: SoundId(8),
                ..AudioEmitter::default()
            },
            GlobalTransform::default(),
        ));

        audio_emitter_system(&mut world);
        let mut transform = *world.get::<GlobalTransform>(emitter).unwrap();
        transform.translation = ribbon_math::Vect::new(5.0, 0.0, 0.0);
        *world.get_mut::<GlobalTransform>(emitter).unwrap() = transform;
        audio_emitter_system(&mut world);
        {
            let state = state.lock().unwrap();
            assert_eq!(
                state.starts, 1,
                "a moving emitter must not restart its sound"
            );
            assert_eq!(
                state.updates, 1,
                "a moving emitter must update its existing voice"
            );
        }
        world.despawn(emitter).unwrap();
        audio_emitter_system(&mut world);
        assert_eq!(
            state.lock().unwrap().stops,
            1,
            "despawning an emitter must stop its voice"
        );
    }

    #[test]
    fn invalid_occlusion_is_sanitized_to_clear_line_of_sight() {
        assert_eq!(
            AudioOcclusionResult {
                occlusion: f32::NAN
            }
            .sanitized()
            .occlusion,
            0.0
        );
        assert_eq!(
            AudioOcclusionResult { occlusion: 2.0 }
                .sanitized()
                .occlusion,
            1.0
        );
    }
}
