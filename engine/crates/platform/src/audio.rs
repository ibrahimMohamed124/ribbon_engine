//! Rodio playback for sound data supplied by the game package.

use std::collections::HashMap;
use std::io::Cursor;
use std::time::Instant;

use rodio::source::Source;
use voxelworks_core::audio::{AmbienceId, AudioBackend, SoundBank, SoundId};

/// Fully decoded sound samples, independent of any audio output device.
#[derive(Clone)]
pub struct DecodedSound {
    buffer: rodio::buffer::SamplesBuffer,
    channels: rodio::ChannelCount,
    sample_rate: rodio::SampleRate,
    sample_count: usize,
}

impl DecodedSound {
    pub fn channels(&self) -> u16 {
        self.channels.get()
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.get()
    }

    /// Number of interleaved samples, not frames.
    pub fn sample_count(&self) -> usize {
        self.sample_count
    }

    pub fn memory_bytes(&self) -> usize {
        self.sample_count * std::mem::size_of::<rodio::Sample>()
    }
}

/// Decode an embedded sound without opening an audio device.
pub fn decode_sound(data: &[u8]) -> Result<DecodedSound, String> {
    let decoder =
        rodio::Decoder::try_from(Cursor::new(data.to_vec())).map_err(|error| error.to_string())?;
    let channels = decoder.channels();
    let sample_rate = decoder.sample_rate();
    let samples: Vec<_> = decoder.collect();
    let sample_count = samples.len();
    if sample_count == 0 {
        return Err("decoded sound contains no samples".to_owned());
    }
    Ok(DecodedSound {
        buffer: rodio::buffer::SamplesBuffer::new(channels, sample_rate, samples),
        channels,
        sample_rate,
        sample_count,
    })
}

pub struct RodioBackend {
    output: rodio::MixerDeviceSink,
    ambience: Option<(AmbienceId, rodio::Player)>,
    bank: SoundBank,
    decoded: HashMap<SoundId, DecodedSound>,
}

impl RodioBackend {
    pub fn new(bank: SoundBank) -> Result<Self, rodio::DeviceSinkError> {
        let decode_start = Instant::now();
        let mut decoded = HashMap::new();
        let mut unique: Vec<(&'static [u8], DecodedSound)> = Vec::new();
        let mut failures = Vec::new();
        for asset in bank.sounds() {
            let id = asset.id;
            let buffer =
                if let Some((_, buffer)) = unique.iter().find(|(data, _)| *data == asset.data) {
                    buffer.clone()
                } else {
                    match decode_sound(asset.data) {
                        Ok(buffer) => {
                            unique.push((asset.data, buffer.clone()));
                            buffer
                        }
                        Err(error) => {
                            failures.push((id, error));
                            continue;
                        }
                    }
                };
            decoded.insert(id, buffer);
        }
        for (id, error) in failures {
            eprintln!("could not decode {id:?} sound; skipping it: {error}");
        }
        let memory_bytes: usize = unique.iter().map(|(_, sound)| sound.memory_bytes()).sum();
        eprintln!(
            "predecoded {} unique sounds in {:.2} ms ({:.1} KiB samples)",
            unique.len(),
            decode_start.elapsed().as_secs_f64() * 1000.0,
            memory_bytes as f64 / 1024.0
        );
        Ok(Self {
            output: rodio::DeviceSinkBuilder::open_default_sink()?,
            ambience: None,
            bank,
            decoded,
        })
    }
}

impl AudioBackend for RodioBackend {
    fn play(&mut self, sound: SoundId, volume: f32) {
        let Some(asset) = self.bank.sound(sound) else {
            return;
        };
        let Some(source) = self.decoded.get(&sound) else {
            return;
        };
        self.output.mixer().add(
            source
                .buffer
                .clone()
                .speed(asset.speed)
                .amplify(volume * asset.gain),
        );
    }

    fn play_spatial(&mut self, sound: SoundId, volume: f32, pan: f32) {
        let Some(asset) = self.bank.sound(sound) else {
            return;
        };
        if !asset.spatial {
            self.play(sound, volume);
            return;
        }
        let player = rodio::SpatialPlayer::connect_new(
            self.output.mixer(),
            [pan.clamp(-1.0, 1.0) * 0.6, 0.8, 0.0],
            [-0.12, 0.0, 0.0],
            [0.12, 0.0, 0.0],
        );
        let Some(source) = self.decoded.get(&sound) else {
            return;
        };
        player.append(
            source
                .buffer
                .clone()
                .speed(asset.speed)
                .amplify(volume * asset.gain),
        );
        player.detach();
    }

    fn set_ambience(&mut self, ambience: Option<AmbienceId>, volume: f32) {
        let ambience = ambience.filter(|_| volume > 0.0);
        let Some(ambience) = ambience else {
            if let Some((_, player)) = self.ambience.take() {
                player.stop();
            }
            return;
        };
        if let Some((current, player)) = &self.ambience {
            if *current == ambience {
                let gain = self.bank.ambience(ambience).map_or(1.0, |asset| asset.gain);
                player.set_volume(volume * gain);
                return;
            }
        }
        if let Some((_, player)) = self.ambience.take() {
            player.stop();
        }
        let Some(asset) = self.bank.ambience(ambience) else {
            return;
        };
        let player = rodio::Player::connect_new(self.output.mixer());
        player.set_volume(volume * asset.gain);
        match rodio::Decoder::try_from(Cursor::new(asset.data)) {
            Ok(source) => {
                player.append(source.repeat_infinite());
                self.ambience = Some((ambience, player));
            }
            Err(error) => eprintln!("could not decode {ambience:?} audio: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::decode_sound;

    fn wav(channels: u16, sample_rate: u32, samples: &[i16]) -> Vec<u8> {
        let data_len = (samples.len() * 2) as u32;
        let mut bytes = Vec::with_capacity(44 + data_len as usize);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&channels.to_le_bytes());
        bytes.extend_from_slice(&sample_rate.to_le_bytes());
        let byte_rate = sample_rate * channels as u32 * 2;
        bytes.extend_from_slice(&byte_rate.to_le_bytes());
        bytes.extend_from_slice(&(channels * 2).to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn decodes_mono_and_stereo_wav_without_an_output_device() {
        let mono = decode_sound(&wav(1, 22_050, &[0, 16384, -16384])).unwrap();
        assert_eq!(mono.channels(), 1);
        assert_eq!(mono.sample_rate(), 22_050);
        assert_eq!(mono.sample_count(), 3);
        assert_eq!(mono.memory_bytes(), 3 * std::mem::size_of::<f32>());

        let stereo = decode_sound(&wav(2, 48_000, &[0, 1, -1, 2])).unwrap();
        assert_eq!(stereo.channels(), 2);
        assert_eq!(stereo.sample_rate(), 48_000);
        assert_eq!(stereo.sample_count(), 4);
    }

    #[test]
    fn invalid_bytes_return_an_error() {
        assert!(decode_sound(b"not a wav").is_err());
    }
}
