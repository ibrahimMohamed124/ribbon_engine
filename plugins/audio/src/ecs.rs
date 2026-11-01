//! ECS adapters for moving emitters, transform-backed listeners, lightweight occlusion,
//! and trigger-volume environmental acoustics.
//!
//! `AudioZoneVolume` is an oriented box centered on `GlobalTransform.translation`.
//! Physics engines can provide occlusion through `AudioOcclusionQuery`; the audio plugin does
//! not depend on a particular collision system. Queries are staggered across frames.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ribbon_ecs::{Entity, World};
use ribbon_scene::GlobalTransform;

use crate::spatial::{self, AcousticZone, Emitter3d};
use crate::{
    AudioBus, AudioEffect, AudioListener, AudioOutput, AudioVec3, ManagedEmitterAcoustics, SoundId,
    SpatialAudioListener, VoiceId,
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

/// Material-aware response supplied by a scene or physics adapter.
///
/// `transmission_gain` is the linear gain remaining after a sound passes through intervening
/// geometry. `low_pass_cutoff_hz` describes its frequency-dependent transmission. This keeps the
/// audio plugin independent of any particular collision or material implementation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AudioAcousticResponse {
    pub occlusion: f32,
    pub transmission_gain: f32,
    pub low_pass_cutoff_hz: f32,
}

impl AudioAcousticResponse {
    pub fn from_occlusion(occlusion: f32) -> Self {
        let occlusion = if occlusion.is_finite() {
            occlusion.clamp(0.0, 1.0)
        } else {
            0.0
        };
        Self {
            occlusion,
            // Preserve the legacy occlusion gain law while keeping gain and filtering as
            // independent material response values for newer adapters.
            transmission_gain: 1.0 - 0.85 * occlusion,
            low_pass_cutoff_hz: 20_000.0 * (1.0 - occlusion) + 700.0 * occlusion,
        }
    }

    pub fn sanitized(self) -> Self {
        let fallback = Self::from_occlusion(self.occlusion);
        Self {
            occlusion: fallback.occlusion,
            transmission_gain: if self.transmission_gain.is_finite() {
                self.transmission_gain.clamp(0.0, 1.0)
            } else {
                // Preserve the legacy occlusion-derived attenuation when a newer material
                // adapter returns a malformed transmission value.
                fallback.transmission_gain
            },
            low_pass_cutoff_hz: if self.low_pass_cutoff_hz.is_finite() {
                self.low_pass_cutoff_hz.clamp(20.0, 20_000.0)
            } else {
                fallback.low_pass_cutoff_hz
            },
        }
    }
}

/// Adapter point for physics/scene collision systems. Implementations should be fast and must
/// not block on asset or device operations. The audio plugin staggers calls to avoid ray storms.
pub trait AudioOcclusionQuery: Send + Sync + 'static {
    fn query_occlusion(&self, source: AudioVec3, listener: AudioVec3) -> AudioOcclusionResult;

    /// Optionally return material-dependent transmission and filtering. Existing adapters remain
    /// source-compatible and receive the baseline attenuation-derived low-pass response.
    fn query_acoustics(&self, source: AudioVec3, listener: AudioVec3) -> AudioAcousticResponse {
        AudioAcousticResponse::from_occlusion(
            self.query_occlusion(source, listener).sanitized().occlusion,
        )
    }
}

/// Optional world resource wrapping an application-provided occlusion query.
#[derive(Default, Clone)]
pub struct AudioOcclusionProvider(pub Option<Arc<dyn AudioOcclusionQuery>>);

/// Environmental acoustic region centered on its entity's `GlobalTransform`.
///
/// The region is an oriented box. Its local +X axis follows entity-right, local +Y follows entity
/// forward, and +Z follows entity-up. Overlapping zones at the highest active priority are blended
/// by their influence weights; lower-priority zones are ignored while a higher tier is active.
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

    fn weight_oriented(
        self,
        center: AudioVec3,
        rotation: ribbon_math::Angles,
        point: AudioVec3,
    ) -> f32 {
        let zone = self.sanitized();
        let delta = point - center;
        // Ribbon's `Angles` convention is psi=pitch, phi=yaw; `forward_vec` is the
        // authoritative scene-space definition (default forward is +X, +Z is up).
        let rotation = ribbon_math::Angles {
            psi: if rotation.psi.is_finite() {
                rotation.psi
            } else {
                0.0
            },
            phi: if rotation.phi.is_finite() {
                rotation.phi
            } else {
                0.0
            },
        };
        let forward_value = ribbon_math::forward_vec(rotation);
        let forward =
            AudioVec3::new(forward_value.x, forward_value.y, forward_value.z).normalized();
        let (sin_yaw, cos_yaw) = rotation.phi.sin_cos();
        let right = AudioVec3::new(sin_yaw, -cos_yaw, 0.0).normalized();
        let up = right.cross(forward).normalized();
        let local = AudioVec3::new(delta.dot(right), delta.dot(forward), delta.dot(up));
        let outside_x = local.x.abs() - zone.half_extents.x;
        let outside_y = local.y.abs() - zone.half_extents.y;
        let outside_z = local.z.abs() - zone.half_extents.z;
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
    rotation: ribbon_math::Angles,
}

#[derive(Clone, Copy)]
struct RuntimeEmitter {
    sound: SoundId,
    looping: bool,
    bus: AudioBus,
    priority: u8,
    voice: Option<VoiceId>,
    consecutive_missing_snapshots: u8,
    retry_after_frame: u64,
}

#[derive(Clone, Copy)]
struct CachedOcclusion {
    target_transmission_gain: f32,
    smoothed_transmission_gain: f32,
    target_low_pass_cutoff_hz: f32,
    smoothed_low_pass_cutoff_hz: f32,
    last_refresh: u64,
}

#[derive(Default)]
pub(super) struct AudioEmitterRuntime {
    frame: u64,
    emitters: HashMap<Entity, RuntimeEmitter>,
    occlusion: HashMap<Entity, CachedOcclusion>,
    zone_signature: Option<(u32, u64, i32, i16, i16, i16)>,
}

/// Reconcile transform-attached emitter handles with worker completion state. Explicit completion
/// IDs are immediate; an authoritative active snapshot uses the same two-poll grace as event slots
/// to tolerate a snapshot queued before a just-submitted Play command.
pub(super) fn reconcile_emitter_completions(
    world: &mut World,
    completions: &crate::VoiceCompletions,
) {
    if world.try_resource::<AudioEmitterRuntime>().is_none() {
        return;
    }
    let completed: HashSet<VoiceId> = completions.voices.iter().copied().collect();
    let active: Option<HashSet<VoiceId>> = completions
        .active_voices
        .as_ref()
        .map(|voices| voices.iter().copied().collect());
    world.resource_scope::<AudioEmitterRuntime, _>(|_, runtime| {
        let retry_after = runtime.frame.saturating_add(OCCLUSION_REFRESH_STRIDE);
        for emitter in runtime.emitters.values_mut() {
            let Some(voice) = emitter.voice else { continue };
            if completed.contains(&voice) {
                emitter.voice = None;
                emitter.consecutive_missing_snapshots = 0;
                emitter.retry_after_frame = retry_after;
                continue;
            }
            if let Some(active) = &active {
                if active.contains(&voice) {
                    emitter.consecutive_missing_snapshots = 0;
                } else {
                    emitter.consecutive_missing_snapshots =
                        emitter.consecutive_missing_snapshots.saturating_add(1);
                    if emitter.consecutive_missing_snapshots >= 2 {
                        emitter.voice = None;
                        emitter.consecutive_missing_snapshots = 0;
                        emitter.retry_after_frame = retry_after;
                    }
                }
            }
        }
    });
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
            rotation: transform.rotation,
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
                .filter_map(|entity| (!active_entities.contains(entity)).then_some(*entity))
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
                let mut active = Vec::new();
                for candidate in &zones {
                    let zone = candidate.zone.sanitized();
                    let weight = candidate.zone.weight_oriented(
                        candidate.center,
                        candidate.rotation,
                        listener.position,
                    );
                    if weight <= 0.0 {
                        continue;
                    }
                    let entity_bits = candidate.entity.to_bits().get();
                    active.push((zone.priority, weight, entity_bits, zone.id, zone.acoustics));
                }
                if let Some(highest_priority) = active.iter().map(|zone| zone.0).max() {
                    active.retain(|zone| zone.0 == highest_priority);
                    active.sort_by_key(|zone| (zone.2, zone.3));
                    let total_weight: f32 = active.iter().map(|zone| zone.1).sum();
                    let weight = 1.0
                        - active
                            .iter()
                            .fold(1.0_f32, |remaining, zone| remaining * (1.0 - zone.1));
                    if total_weight > f32::EPSILON {
                        let blend = |value: fn(AcousticZone) -> f32| {
                            active
                                .iter()
                                .map(|zone| value(zone.4) * zone.1)
                                .sum::<f32>()
                                / total_weight
                        };
                        let anchor = active[0];
                        let blended = AcousticZone {
                            id: anchor.3,
                            wet_mix: blend(|zone| zone.wet_mix),
                            low_pass_hz: blend(|zone| zone.low_pass_hz),
                            reverb_seconds: blend(|zone| zone.reverb_seconds),
                            ambience_gain: blend(|zone| zone.ambience_gain),
                        }
                        .sanitized();
                        chosen_zone = Some((
                            highest_priority,
                            weight.clamp(0.0, 1.0),
                            anchor.2,
                            anchor.3,
                            blended,
                        ));
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
                let mut accepted = true;
                if sink.try_set_bus_effects(AudioBus::Sfx, &effects).is_err() {
                    accepted = false;
                }
                if sink
                    .try_set_bus_effects(AudioBus::Ambience, &effects)
                    .is_err()
                {
                    accepted = false;
                }
                let ambience_scale = chosen_zone
                    .map(|(_, weight, _, _, zone)| 1.0 + (zone.ambience_gain - 1.0) * weight)
                    .unwrap_or(1.0);
                if sink
                    .try_set_bus_zone_gain(AudioBus::Ambience, ambience_scale, EFFECT_TWEEN_SECONDS)
                    .is_err()
                {
                    accepted = false;
                }
                // Do not suppress retries after bounded queue saturation. Re-submit the complete
                // zone state on the next update until every operation has been accepted.
                if accepted {
                    runtime.zone_signature = zone_signature;
                }
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
                let component_occlusion = finite_clamp(snapshot.emitter.occlusion, 0.0, 1.0, 0.0);
                let occlusion = component_occlusion;
                let mut transmission_gain = 1.0;
                let mut low_pass_cutoff_hz = occlusion_filter_cutoff(component_occlusion);
                if let Some(provider) = &occlusion_provider {
                    let refresh = snapshot.entity.to_bits().get() % OCCLUSION_REFRESH_STRIDE
                        == frame % OCCLUSION_REFRESH_STRIDE;
                    let cached =
                        runtime
                            .occlusion
                            .entry(snapshot.entity)
                            .or_insert(CachedOcclusion {
                                target_transmission_gain: 1.0,
                                smoothed_transmission_gain: 1.0,
                                target_low_pass_cutoff_hz: low_pass_cutoff_hz,
                                smoothed_low_pass_cutoff_hz: low_pass_cutoff_hz,
                                last_refresh: frame,
                            });
                    if refresh {
                        let response = provider
                            .query_acoustics(source, listener.position)
                            .sanitized();
                        cached.target_transmission_gain = response.transmission_gain;
                        cached.target_low_pass_cutoff_hz = response.low_pass_cutoff_hz;
                        cached.last_refresh = frame;
                    } else if frame.saturating_sub(cached.last_refresh) > OCCLUSION_CACHE_MAX_AGE {
                        cached.target_transmission_gain = 1.0;
                        cached.target_low_pass_cutoff_hz =
                            occlusion_filter_cutoff(component_occlusion);
                    }
                    cached.smoothed_transmission_gain += (cached.target_transmission_gain
                        - cached.smoothed_transmission_gain)
                        * 0.25;
                    cached.smoothed_low_pass_cutoff_hz += (cached.target_low_pass_cutoff_hz
                        - cached.smoothed_low_pass_cutoff_hz)
                        * 0.25;
                    transmission_gain = cached.smoothed_transmission_gain;
                    // The adapter's material response owns its frequency curve. Only the
                    // emitter's explicit baseline occlusion is layered on top; reapplying the
                    // adapter's aggregate occlusion here would incorrectly clamp a permissive
                    // material (for example glass) to the generic wall response.
                    low_pass_cutoff_hz = cached
                        .smoothed_low_pass_cutoff_hz
                        .min(occlusion_filter_cutoff(component_occlusion));
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
                let gain = spatial.gain * transmission_gain;
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
                            sink.update_managed_emitter_acoustics(
                                voice,
                                ManagedEmitterAcoustics {
                                    gain: spatial.gain,
                                    pan: spatial.pan,
                                    playback_rate: rate,
                                    occlusion,
                                    transmission_gain,
                                    low_pass_cutoff_hz,
                                },
                            );
                        } else if sink.supports_managed_voices()
                            && runtime.frame >= active.retry_after_frame
                        {
                            // An asynchronous backend may have rejected the prior start, or its
                            // completion snapshot may have invalidated the voice. Retry at a
                            // bounded interval rather than spamming a full command queue every frame.
                            let voice = sink.play_managed_emitter(crate::ManagedEmitterPlayback {
                                sound,
                                gain,
                                pan: spatial.pan,
                                playback_rate: rate,
                                looping: snapshot.emitter.looping,
                                priority: snapshot.emitter.priority,
                                bus: snapshot.emitter.bus,
                            });
                            if let Some(voice) = voice {
                                sink.update_managed_emitter_acoustics(
                                    voice,
                                    ManagedEmitterAcoustics {
                                        gain: spatial.gain,
                                        pan: spatial.pan,
                                        playback_rate: rate,
                                        occlusion,
                                        transmission_gain,
                                        low_pass_cutoff_hz,
                                    },
                                );
                            }
                            runtime.emitters.insert(
                                entity,
                                RuntimeEmitter {
                                    voice,
                                    consecutive_missing_snapshots: 0,
                                    retry_after_frame: runtime
                                        .frame
                                        .saturating_add(OCCLUSION_REFRESH_STRIDE),
                                    ..active
                                },
                            );
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
                        if let Some(voice) = voice {
                            sink.update_managed_emitter_acoustics(
                                voice,
                                ManagedEmitterAcoustics {
                                    gain: spatial.gain,
                                    pan: spatial.pan,
                                    playback_rate: rate,
                                    occlusion,
                                    transmission_gain,
                                    low_pass_cutoff_hz,
                                },
                            );
                        }
                        runtime.emitters.insert(
                            entity,
                            RuntimeEmitter {
                                sound,
                                looping: snapshot.emitter.looping,
                                bus: snapshot.emitter.bus,
                                priority: snapshot.emitter.priority,
                                voice,
                                consecutive_missing_snapshots: 0,
                                retry_after_frame: runtime
                                    .frame
                                    .saturating_add(OCCLUSION_REFRESH_STRIDE),
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
                        if let Some(voice) = voice {
                            sink.update_managed_emitter_acoustics(
                                voice,
                                ManagedEmitterAcoustics {
                                    gain: spatial.gain,
                                    pan: spatial.pan,
                                    playback_rate: rate,
                                    occlusion,
                                    transmission_gain,
                                    low_pass_cutoff_hz,
                                },
                            );
                        }
                        runtime.emitters.insert(
                            entity,
                            RuntimeEmitter {
                                sound,
                                looping: snapshot.emitter.looping,
                                bus: snapshot.emitter.bus,
                                priority: snapshot.emitter.priority,
                                voice,
                                consecutive_missing_snapshots: 0,
                                retry_after_frame: runtime
                                    .frame
                                    .saturating_add(OCCLUSION_REFRESH_STRIDE),
                            },
                        );
                    }
                }
            }
            runtime.frame = runtime.frame.saturating_add(1);
        });
    });
}

fn occlusion_filter_cutoff(occlusion: f32) -> f32 {
    20_000.0 * (1.0 - finite_clamp(occlusion, 0.0, 1.0, 0.0))
        + 700.0 * finite_clamp(occlusion, 0.0, 1.0, 0.0)
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
        assert!(
            (zone.weight_oriented(
                AudioVec3::ZERO,
                ribbon_math::Angles::default(),
                AudioVec3::new(1.0, 0.0, 0.0),
            ) - 1.0)
                .abs()
                < 1e-6
        );
        assert!(
            (zone.weight_oriented(
                AudioVec3::ZERO,
                ribbon_math::Angles::default(),
                AudioVec3::new(4.0, 0.0, 0.0),
            ) - 0.5)
                .abs()
                < 1e-6
        );
        assert_eq!(
            zone.weight_oriented(
                AudioVec3::ZERO,
                ribbon_math::Angles::default(),
                AudioVec3::new(7.0, 0.0, 0.0),
            ),
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
            fn supports_managed_voices(&self) -> bool {
                true
            }
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
        {
            let state = state.lock().unwrap();
            assert_eq!(
                state.starts, 1,
                "the emitter must create exactly one managed voice"
            );
            assert_eq!(
                state.updates, 1,
                "a newly created voice must receive its initial acoustic state"
            );
        }

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
                state.updates, 2,
                "a moving emitter must update its existing voice after its initial setup"
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

    #[test]
    fn legacy_occlusion_adapter_preserves_gain_and_filter_behavior() {
        let response = AudioAcousticResponse::from_occlusion(0.5);
        assert!((response.transmission_gain - 0.575).abs() < 1.0e-6);
        assert!((response.low_pass_cutoff_hz - 10_350.0).abs() < 1.0e-3);
    }

    #[test]
    fn material_acoustic_response_sanitizes_transmission_and_filtering() {
        let response = AudioAcousticResponse {
            occlusion: f32::NAN,
            transmission_gain: 2.0,
            low_pass_cutoff_hz: f32::INFINITY,
        }
        .sanitized();
        assert_eq!(response.occlusion, 0.0);
        assert_eq!(response.transmission_gain, 1.0);
        assert_eq!(response.low_pass_cutoff_hz, 20_000.0);

        let malformed_material_gain = AudioAcousticResponse {
            occlusion: 0.5,
            transmission_gain: f32::NAN,
            low_pass_cutoff_hz: 16_000.0,
        }
        .sanitized();
        assert!((malformed_material_gain.transmission_gain - 0.575).abs() < 1.0e-6);

        let material = AudioAcousticResponse {
            occlusion: 0.4,
            transmission_gain: 0.25,
            low_pass_cutoff_hz: 650.0,
        }
        .sanitized();
        assert_eq!(material.occlusion, 0.4);
        assert_eq!(material.transmission_gain, 0.25);
        assert_eq!(material.low_pass_cutoff_hz, 650.0);
    }

    #[test]
    fn material_filter_is_not_overridden_by_aggregate_occlusion() {
        let material_cutoff: f32 = 16_000.0;
        assert_eq!(
            material_cutoff.min(occlusion_filter_cutoff(0.0)),
            material_cutoff,
            "a material adapter must be able to preserve high frequencies through glass"
        );
        assert!(
            material_cutoff.min(occlusion_filter_cutoff(0.5)) < material_cutoff,
            "explicit emitter occlusion must still attenuate frequencies"
        );
    }

    #[test]
    fn zone_basis_has_positive_world_up_at_default_orientation() {
        let forward = ribbon_math::forward_vec(ribbon_math::Angles::default());
        let forward = AudioVec3::new(forward.x, forward.y, forward.z).normalized();
        let right = AudioVec3::new(0.0, -1.0, 0.0);
        let up = right.cross(forward).normalized();
        assert!(up.z > 0.999);
    }

    #[test]
    fn oriented_zone_weight_changes_with_entity_rotation() {
        let zone = AudioZoneVolume {
            id: 9,
            priority: 1,
            half_extents: AudioVec3::new(0.5, 3.0, 1.0),
            blend_distance: 0.0,
            acoustics: AcousticZone {
                id: 9,
                wet_mix: 0.5,
                low_pass_hz: 2_000.0,
                reverb_seconds: 1.0,
                ambience_gain: 1.0,
            },
        };
        let along_world_x = AudioVec3::new(2.0, 0.0, 0.0);
        assert_eq!(
            zone.weight_oriented(
                AudioVec3::ZERO,
                ribbon_math::Angles::default(),
                along_world_x,
            ),
            1.0
        );
        // Rotate yaw by 90 degrees: local forward moves from +X to +Y, so +X
        // is now the narrow right axis and lies outside the half extent.
        let rotated = ribbon_math::Angles {
            psi: 0.0,
            phi: std::f32::consts::FRAC_PI_2,
        };
        assert_eq!(
            zone.weight_oriented(AudioVec3::ZERO, rotated, along_world_x),
            0.0
        );
    }
}
