//! Optional Steam Audio direct-path and HRTF/binaural processors.
//!
//! This module is feature-gated because Steam Audio links a native SDK. Kira voices can use a
//! realtime direct-occlusion processor and optional HRTF processor; the engine still owns world
//! geometry, room membership, material queries and any expensive scene simulation.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

use audionimbus::{
    AudioBufferMut, AudioBufferRef, AudioSettings, BinauralEffect, BinauralEffectParams,
    BinauralEffectSettings, Context, DirectEffect, DirectEffectParams, DirectEffectSettings,
    Direction, Hrtf, HrtfInterpolation, HrtfSettings,
};
use kira::effect::{Effect, EffectBuilder};
use kira::info::Info;
use kira::Frame;

struct SharedSteamAudioResources {
    context: Context,
    settings: AudioSettings,
    hrtf: Hrtf,
}

/// Steam Audio's context and default HRTF are process-wide resources. Reusing them avoids
/// repeatedly constructing expensive native contexts/HRTFs for every voice.
fn shared_steam_audio_resources() -> Result<Arc<SharedSteamAudioResources>, String> {
    static RESOURCES: OnceLock<Result<Arc<SharedSteamAudioResources>, String>> = OnceLock::new();
    match RESOURCES.get_or_init(|| {
        let context = Context::default();
        let settings = AudioSettings::default();
        let hrtf = Hrtf::try_new(&context, &settings, &HrtfSettings::default())
            .map_err(|error| format!("Steam Audio HRTF initialization failed: {error}"))?;
        Ok(Arc::new(SharedSteamAudioResources {
            context,
            settings,
            hrtf,
        }))
    }) {
        Ok(resources) => Ok(Arc::clone(resources)),
        Err(error) => Err(error.clone()),
    }
}

/// Build a Steam Audio direct-path occlusion effect for a stereo voice track.
///
/// The effect receives the engine's material/portal occlusion amount through an atomic handle.
/// A value of zero means an unobstructed path and one means fully occluded; this is inverted to
/// match Steam Audio's direct-effect convention. Material transmission gain remains applied by
/// the existing voice gain path so it is never multiplied twice.
pub struct SteamAudioDirectEffectBuilder {
    resources: Arc<SharedSteamAudioResources>,
    settings: AudioSettings,
    effect: DirectEffect,
    occlusion_bits: Arc<AtomicU32>,
}

/// Thread-safe update handle for Steam Audio direct-path occlusion.
#[derive(Clone)]
pub struct SteamAudioDirectEffectHandle {
    occlusion_bits: Arc<AtomicU32>,
}

impl SteamAudioDirectEffectHandle {
    pub fn set_occlusion(&self, occlusion: f32) -> Result<(), String> {
        if !occlusion.is_finite() {
            return Err("Steam Audio occlusion must be finite".into());
        }
        self.occlusion_bits
            .store(occlusion.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        Ok(())
    }
}

impl SteamAudioDirectEffectBuilder {
    pub fn new() -> Result<Self, String> {
        let resources = shared_steam_audio_resources()?;
        let settings = resources.settings;
        let effect = DirectEffect::try_new(
            &resources.context,
            &settings,
            &DirectEffectSettings { num_channels: 2 },
        )
        .map_err(|error| format!("Steam Audio direct-effect initialization failed: {error}"))?;
        Ok(Self {
            resources,
            settings,
            effect,
            occlusion_bits: Arc::new(AtomicU32::new(0.0f32.to_bits())),
        })
    }
}

impl EffectBuilder for SteamAudioDirectEffectBuilder {
    type Handle = SteamAudioDirectEffectHandle;

    fn build(self) -> (Box<dyn Effect>, Self::Handle) {
        let frames = self.settings.frame_size as usize;
        let handle = SteamAudioDirectEffectHandle {
            occlusion_bits: Arc::clone(&self.occlusion_bits),
        };
        let effect = SteamAudioDirectKiraEffect {
            _resources: self.resources,
            settings: self.settings,
            effect: self.effect,
            occlusion_bits: self.occlusion_bits,
            stereo_scratch: vec![0.0; frames.saturating_mul(2)],
            enabled: frames > 0 && frames <= 16_384,
        };
        (Box::new(effect), handle)
    }
}

struct SteamAudioDirectKiraEffect {
    _resources: Arc<SharedSteamAudioResources>,
    settings: AudioSettings,
    effect: DirectEffect,
    occlusion_bits: Arc<AtomicU32>,
    // Planar scratch: all left samples followed by all right samples, as required by AudioNimbus.
    stereo_scratch: Vec<f32>,
    enabled: bool,
}

impl SteamAudioDirectKiraEffect {
    fn configure_audio_format(&mut self, sample_rate: u32, internal_buffer_size: usize) {
        let Ok(frame_size) = u32::try_from(internal_buffer_size) else {
            self.enabled = false;
            return;
        };
        if sample_rate == 0 || frame_size == 0 || frame_size > 16_384 {
            self.enabled = false;
            return;
        }
        if sample_rate != self.settings.sampling_rate || frame_size != self.settings.frame_size {
            self.settings.sampling_rate = sample_rate;
            self.settings.frame_size = frame_size;
            match DirectEffect::try_new(
                &self._resources.context,
                &self.settings,
                &DirectEffectSettings { num_channels: 2 },
            ) {
                Ok(effect) => {
                    self.effect = effect;
                    self.enabled = true;
                }
                Err(_) => self.enabled = false,
            }
        }
        let frames = self.settings.frame_size as usize;
        self.enabled &= frames > 0 && frames <= 16_384;
        self.stereo_scratch.resize(frames.saturating_mul(2), 0.0);
    }
}

impl Effect for SteamAudioDirectKiraEffect {
    fn init(&mut self, sample_rate: u32, internal_buffer_size: usize) {
        self.configure_audio_format(sample_rate, internal_buffer_size);
    }

    fn on_change_sample_rate(&mut self, sample_rate: u32) {
        let frame_size = self.settings.frame_size as usize;
        self.configure_audio_format(sample_rate, frame_size);
    }

    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &Info<'_>) {
        if !self.enabled || self.stereo_scratch.is_empty() {
            return;
        }
        let block_frames = self.stereo_scratch.len() / 2;
        // Keep ordinary stereo voices stereo. The direct-path effect operates on a two-channel
        // planar buffer; only the optional following BinauralEffect deliberately downmixes to mono.
        let occlusion = f32::from_bits(self.occlusion_bits.load(Ordering::Relaxed)).clamp(0.0, 1.0);
        let params = DirectEffectParams {
            occlusion: Some(1.0 - occlusion),
            ..DirectEffectParams::default()
        };
        for chunk in input.chunks_mut(block_frames) {
            self.stereo_scratch.fill(0.0);
            for (index, frame) in chunk.iter().enumerate() {
                self.stereo_scratch[index] = frame.left;
                self.stereo_scratch[block_frames + index] = frame.right;
            }
            let Ok(mut buffer) = AudioBufferMut::try_new(&mut self.stereo_scratch, 2) else {
                continue;
            };
            if self.effect.apply_in_place(&params, &mut buffer).is_err() {
                continue;
            }
            drop(buffer);
            for (index, frame) in chunk.iter_mut().enumerate() {
                frame.left = self.stereo_scratch[index];
                frame.right = self.stereo_scratch[block_frames + index];
            }
        }
    }
}

/// Build a Steam Audio HRTF effect that can be attached directly to a Kira `TrackBuilder`.
///
/// Kira owns the processing object on its audio thread. The returned handle only writes three
/// atomic direction components, so moving the source does not require locking or rebuilding the
/// effect. The effect converts the track's stereo input to mono before point-source rendering.
pub struct SteamAudioBinauralEffectBuilder {
    resources: Arc<SharedSteamAudioResources>,
    settings: AudioSettings,
    hrtf: Hrtf,
    effect: BinauralEffect,
    direction_bits: Arc<[AtomicU32; 3]>,
}

/// Thread-safe live direction control for a Kira Steam Audio binaural effect.
#[derive(Clone)]
pub struct SteamAudioBinauralHandle {
    direction_bits: Arc<[AtomicU32; 3]>,
}

impl SteamAudioBinauralHandle {
    /// Update a source direction in listener-local coordinates (right, up, behind).
    pub fn set_direction(&self, direction: [f32; 3]) -> Result<(), String> {
        let normalized = normalize_direction(direction)?;
        for (slot, value) in self.direction_bits.iter().zip(normalized) {
            slot.store(value.to_bits(), Ordering::Relaxed);
        }
        Ok(())
    }
}

impl SteamAudioBinauralEffectBuilder {
    pub fn new(direction: [f32; 3]) -> Result<Self, String> {
        let direction = normalize_direction(direction)?;
        let resources = shared_steam_audio_resources()?;
        let settings = resources.settings;
        let hrtf = resources.hrtf.clone();
        let effect = BinauralEffect::try_new(
            &resources.context,
            &settings,
            &BinauralEffectSettings { hrtf: hrtf.clone() },
        )
        .map_err(|error| format!("Steam Audio binaural effect initialization failed: {error}"))?;
        let direction_bits = Arc::new(direction.map(|value| AtomicU32::new(value.to_bits())));
        Ok(Self {
            resources,
            settings,
            hrtf,
            effect,
            direction_bits,
        })
    }
}

impl EffectBuilder for SteamAudioBinauralEffectBuilder {
    type Handle = SteamAudioBinauralHandle;

    fn build(self) -> (Box<dyn Effect>, Self::Handle) {
        let frames = self.settings.frame_size as usize;
        let handle = SteamAudioBinauralHandle {
            direction_bits: Arc::clone(&self.direction_bits),
        };
        let effect = SteamAudioKiraEffect {
            _resources: self.resources,
            settings: self.settings,
            hrtf: self.hrtf,
            effect: self.effect,
            direction_bits: self.direction_bits,
            mono_input: vec![0.0; frames],
            planar_output: vec![0.0; frames.saturating_mul(2)],
            enabled: frames > 0 && frames <= 16_384,
        };
        (Box::new(effect), handle)
    }
}

struct SteamAudioKiraEffect {
    _resources: Arc<SharedSteamAudioResources>,
    settings: AudioSettings,
    hrtf: Hrtf,
    effect: BinauralEffect,
    direction_bits: Arc<[AtomicU32; 3]>,
    mono_input: Vec<f32>,
    planar_output: Vec<f32>,
    enabled: bool,
}

impl SteamAudioKiraEffect {
    fn configure_audio_format(&mut self, sample_rate: u32, internal_buffer_size: usize) {
        let Ok(frame_size) = u32::try_from(internal_buffer_size) else {
            self.enabled = false;
            return;
        };
        if sample_rate == 0 || frame_size == 0 || frame_size > 16_384 {
            self.enabled = false;
            return;
        }
        if sample_rate == self.settings.sampling_rate && frame_size == self.settings.frame_size {
            return;
        }
        self.settings.sampling_rate = sample_rate;
        self.settings.frame_size = frame_size;
        match Hrtf::try_new(
            &self._resources.context,
            &self.settings,
            &HrtfSettings::default(),
        ) {
            Ok(hrtf) => match BinauralEffect::try_new(
                &self._resources.context,
                &self.settings,
                &BinauralEffectSettings { hrtf: hrtf.clone() },
            ) {
                Ok(effect) => {
                    self.hrtf = hrtf;
                    self.effect = effect;
                    self.enabled = true;
                }
                Err(_) => self.enabled = false,
            },
            Err(_) => self.enabled = false,
        }
        let frames = self.settings.frame_size as usize;
        self.enabled &= frames > 0 && frames <= 16_384;
        self.mono_input.resize(frames, 0.0);
        self.planar_output.resize(frames.saturating_mul(2), 0.0);
    }

    fn direction(&self) -> [f32; 3] {
        let values = [
            f32::from_bits(self.direction_bits[0].load(Ordering::Relaxed)),
            f32::from_bits(self.direction_bits[1].load(Ordering::Relaxed)),
            f32::from_bits(self.direction_bits[2].load(Ordering::Relaxed)),
        ];
        normalize_direction(values).unwrap_or([0.0, 0.0, 1.0])
    }
}

impl Effect for SteamAudioKiraEffect {
    fn init(&mut self, sample_rate: u32, internal_buffer_size: usize) {
        self.configure_audio_format(sample_rate, internal_buffer_size);
    }

    fn on_change_sample_rate(&mut self, sample_rate: u32) {
        let frame_size = self.settings.frame_size as usize;
        self.configure_audio_format(sample_rate, frame_size);
    }

    fn process(&mut self, input: &mut [Frame], _dt: f64, _info: &Info<'_>) {
        if !self.enabled || self.mono_input.is_empty() {
            return;
        }
        let block_frames = self.mono_input.len();
        let [x, y, z] = self.direction();
        let params = BinauralEffectParams {
            direction: Direction::new(x, y, z),
            interpolation: HrtfInterpolation::Bilinear,
            spatial_blend: 1.0,
            hrtf: self.hrtf.clone(),
            peak_delays: None,
        };
        for chunk in input.chunks_mut(block_frames) {
            self.mono_input.fill(0.0);
            for (mono, frame) in self.mono_input.iter_mut().zip(chunk.iter()) {
                *mono = (frame.left + frame.right) * 0.5;
            }
            let Ok(input_buffer) = AudioBufferRef::try_from(&self.mono_input[..]) else {
                continue;
            };
            let Ok(mut output_buffer) = AudioBufferMut::try_new(&mut self.planar_output, 2) else {
                continue;
            };
            if self
                .effect
                .apply(&params, &input_buffer, &mut output_buffer)
                .is_err()
            {
                continue;
            }
            drop(output_buffer);
            for (index, frame) in chunk.iter_mut().enumerate() {
                frame.left = self.planar_output[index];
                frame.right = self.planar_output[block_frames + index];
            }
        }
    }
}

fn normalize_direction(direction: [f32; 3]) -> Result<[f32; 3], String> {
    if direction.iter().any(|value| !value.is_finite()) {
        return Err("Steam Audio source direction must contain finite values".into());
    }
    let length_squared = direction.iter().map(|value| value * value).sum::<f32>();
    if length_squared <= f32::EPSILON {
        return Err("Steam Audio source direction must be non-zero".into());
    }
    let inverse_length = length_squared.sqrt().recip();
    Ok([
        direction[0] * inverse_length,
        direction[1] * inverse_length,
        direction[2] * inverse_length,
    ])
}

/// Steam Audio HRTF renderer for fixed-size mono input blocks and interleaved stereo output.
/// The output scratch buffer is allocated once in `new`; `process_mono` does not allocate.
pub struct SteamAudioBinauralRenderer {
    _resources: Arc<SharedSteamAudioResources>,
    settings: AudioSettings,
    hrtf: Hrtf,
    effect: BinauralEffect,
    planar_output: Vec<f32>,
}

impl SteamAudioBinauralRenderer {
    /// Create an HRTF processor using AudioNimbus/Steam Audio's default audio settings.
    pub fn new() -> Result<Self, String> {
        let resources = shared_steam_audio_resources()?;
        let settings = resources.settings;
        let hrtf = resources.hrtf.clone();
        let effect = BinauralEffect::try_new(
            &resources.context,
            &settings,
            &BinauralEffectSettings { hrtf: hrtf.clone() },
        )
        .map_err(|error| format!("Steam Audio binaural effect initialization failed: {error}"))?;
        let frames = settings.frame_size as usize;
        if frames == 0 || frames > 16_384 {
            return Err("Steam Audio returned an unsupported frame size".into());
        }
        Ok(Self {
            _resources: resources,
            settings,
            hrtf,
            effect,
            planar_output: vec![0.0; frames.saturating_mul(2)],
        })
    }

    pub fn frame_size(&self) -> usize {
        self.settings.frame_size as usize
    }
    pub fn sample_rate(&self) -> u32 {
        self.settings.sampling_rate
    }

    /// Process one mono block. `direction` is source direction in listener coordinates.
    /// Output is interleaved stereo (L,R,L,R,...) and must contain exactly 2 * frame_size samples.
    pub fn process_mono(
        &mut self,
        direction: [f32; 3],
        input: &[f32],
        output_interleaved: &mut [f32],
    ) -> Result<(), String> {
        let frames = self.frame_size();
        if input.len() != frames || output_interleaved.len() != frames * 2 {
            return Err(format!(
                "Steam Audio block size mismatch: expected {frames} mono input and {} stereo output samples",
                frames * 2,
            ));
        }
        let normalized = normalize_direction(direction)?;
        let input_buffer = AudioBufferRef::try_from(input)
            .map_err(|error| format!("Steam Audio input buffer rejected: {error}"))?;
        let mut output_buffer = AudioBufferMut::try_new(&mut self.planar_output, 2)
            .map_err(|error| format!("Steam Audio output buffer rejected: {error}"))?;
        let params = BinauralEffectParams {
            direction: Direction::new(normalized[0], normalized[1], normalized[2]),
            interpolation: HrtfInterpolation::Bilinear,
            spatial_blend: 1.0,
            hrtf: self.hrtf.clone(),
            peak_delays: None,
        };
        self.effect
            .apply(&params, &input_buffer, &mut output_buffer)
            .map_err(|error| format!("Steam Audio binaural processing failed: {error}"))?;
        drop(output_buffer);
        for frame in 0..frames {
            output_interleaved[frame * 2] = self.planar_output[frame];
            output_interleaved[frame * 2 + 1] = self.planar_output[frames + frame];
        }
        Ok(())
    }

    /// Clear Steam Audio's internal HRTF history after a discontinuity or device re-open.
    pub fn reset(&mut self) {
        self.effect.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_effect_builder_exposes_finite_live_occlusion_control() {
        let builder = SteamAudioDirectEffectBuilder::new()
            .expect("Steam Audio direct effect should initialize");
        let (mut effect, handle) = builder.build();
        effect.init(48_000, 128);
        assert!(handle.set_occlusion(0.0).is_ok());
        assert!(handle.set_occlusion(f32::NAN).is_err());
        let info = kira::info::MockInfoBuilder::new().build();
        let mut frames = vec![Frame::new(0.5, 0.2); 128];
        effect.process(&mut frames, 128.0 / 48_000.0, &info);
        assert!(frames
            .iter()
            .all(|frame| frame.left.is_finite() && frame.right.is_finite()));
        assert!(
            frames
                .iter()
                .any(|frame| frame.left.abs() > frame.right.abs() + 1.0e-5),
            "direct-path processing must preserve distinct left and right channels"
        );
    }

    #[test]
    fn binaural_processor_uses_fixed_bounded_blocks() {
        let renderer = SteamAudioBinauralRenderer::new().expect("Steam Audio SDK must initialize");
        assert!(renderer.frame_size() > 0);
        assert!(renderer.frame_size() <= 16_384);
        assert!(renderer.sample_rate() >= 8_000);
    }

    #[test]
    fn binaural_effect_builder_exposes_atomic_live_direction_control() {
        let builder = SteamAudioBinauralEffectBuilder::new([0.0, 0.0, 1.0])
            .expect("Steam Audio Kira effect should initialize");
        let (_effect, handle) = builder.build();
        assert!(handle.set_direction([1.0, 0.0, 0.0]).is_ok());
        assert!(handle.set_direction([0.0, 0.0, 0.0]).is_err());
    }
}
