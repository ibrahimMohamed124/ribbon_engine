//! Conservative container metadata parsers used to admit embedded sounds to the static PCM cache.
//!
//! The estimate is made before handing compressed bytes to a full decoder. Unknown or malformed
//! containers are not fully decoded at startup; the playback path falls back to chunked streaming.

const PCM_BYTES_PER_FRAME: usize = 8; // Kira's stereo f32 frame.

pub fn estimate_static_pcm_bytes(bytes: &[u8]) -> Option<usize> {
    let frames = if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        wav_frames(bytes)?
    } else if bytes.starts_with(b"FORM")
        && (bytes.get(8..12) == Some(&b"AIFF"[..]) || bytes.get(8..12) == Some(&b"AIFC"[..]))
    {
        aiff_frames(bytes)?
    } else if bytes.starts_with(b"fLaC") {
        flac_frames(bytes)?
    } else if bytes.starts_with(b"OggS") {
        ogg_vorbis_frames(bytes)?
    } else {
        mp3_frames(bytes)?
    };
    frames.checked_mul(PCM_BYTES_PER_FRAME)
}

fn wav_frames(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 12 {
        return None;
    }
    let mut offset = 12usize;
    let mut expected_block_align = None;
    let mut data_bytes = 0usize;
    while offset.checked_add(8)? <= bytes.len() {
        let id = bytes.get(offset..offset + 4)?;
        let size = u32::from_le_bytes(bytes.get(offset + 4..offset + 8)?.try_into().ok()?) as usize;
        let begin = offset.checked_add(8)?;
        let end = begin.checked_add(size)?;
        let chunk = bytes.get(begin..end)?;
        if id == b"fmt " {
            if chunk.len() < 16 {
                return None;
            }
            let format = u16::from_le_bytes(chunk[0..2].try_into().ok()?);
            let channels = u16::from_le_bytes(chunk[2..4].try_into().ok()?);
            let sample_rate = u32::from_le_bytes(chunk[4..8].try_into().ok()?);
            let align = u16::from_le_bytes(chunk[12..14].try_into().ok()?) as usize;
            let bits = u16::from_le_bytes(chunk[14..16].try_into().ok()?);
            // PCM, IEEE float, and WAVE_FORMAT_EXTENSIBLE (PCM/float subformats only).
            let extensible_format = if format == 0xfffe && chunk.len() >= 40 {
                u16::from_le_bytes(chunk[24..26].try_into().ok()?)
            } else {
                format
            };
            if !matches!(extensible_format, 1 | 3)
                || !(1..=8).contains(&channels)
                || !(8..=64).contains(&bits)
                || sample_rate == 0
            {
                return None;
            }
            // Reject inconsistent PCM layouts. Trusting a smaller-than-required block alignment
            // would undercount decoded frames and could admit an oversized asset to the cache.
            let bytes_per_sample = (bits as usize).checked_add(7)?.checked_div(8)?;
            let expected_align = (channels as usize).checked_mul(bytes_per_sample)?;
            if align != expected_align {
                return None;
            }
            expected_block_align = Some(expected_align);
        } else if id == b"data" {
            data_bytes = data_bytes.checked_add(chunk.len())?;
        }
        offset = end.checked_add(size & 1)?;
    }
    let align = expected_block_align?;
    if data_bytes == 0 {
        return None;
    }
    // Round up a partial final frame so malformed/truncated data cannot understate PCM demand.
    Some(data_bytes.checked_add(align.saturating_sub(1))? / align)
}

fn aiff_frames(bytes: &[u8]) -> Option<usize> {
    let mut offset = 12usize;
    while offset.checked_add(8)? <= bytes.len() {
        let id = bytes.get(offset..offset + 4)?;
        let size = u32::from_be_bytes(bytes.get(offset + 4..offset + 8)?.try_into().ok()?) as usize;
        let begin = offset.checked_add(8)?;
        let end = begin.checked_add(size)?;
        let chunk = bytes.get(begin..end)?;
        if id == b"COMM" {
            if chunk.len() < 18 {
                return None;
            }
            let channels = u16::from_be_bytes(chunk[0..2].try_into().ok()?);
            let frames = u32::from_be_bytes(chunk[2..6].try_into().ok()?) as usize;
            let bits = u16::from_be_bytes(chunk[6..8].try_into().ok()?);
            if !(1..=8).contains(&channels) || !(8..=64).contains(&bits) {
                return None;
            }
            return Some(frames);
        }
        offset = end.checked_add(size & 1)?;
    }
    None
}

fn flac_frames(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 42 {
        return None;
    }
    let mut offset = 4usize;
    while offset.checked_add(4)? <= bytes.len() {
        let header = bytes[offset];
        let block_type = header & 0x7f;
        let last = header & 0x80 != 0;
        let length = ((bytes[offset + 1] as usize) << 16)
            | ((bytes[offset + 2] as usize) << 8)
            | bytes[offset + 3] as usize;
        let begin = offset + 4;
        let end = begin.checked_add(length)?;
        let block = bytes.get(begin..end)?;
        if block_type == 0 {
            if block.len() != 34 {
                return None;
            }
            let packed = u64::from_be_bytes(block[10..18].try_into().ok()?);
            let channels = ((packed >> 41) & 0x7) as usize + 1;
            let samples = usize::try_from(packed & 0x0f_ffff_ffff).ok()?;
            if !(1..=8).contains(&channels) || samples == 0 {
                return None;
            }
            return Some(samples);
        }
        if last {
            break;
        }
        offset = end;
    }
    None
}

fn ogg_vorbis_frames(bytes: &[u8]) -> Option<usize> {
    let mut offset = 0usize;
    let mut packet_prefix = Vec::with_capacity(30);
    let mut max_granule = None;
    let mut stream_serial = None;
    let mut expected_page_sequence = 0u32;
    while offset.checked_add(27)? <= bytes.len() {
        if bytes.get(offset..offset + 4)? != b"OggS" {
            return None;
        }
        if bytes[offset + 4] != 0 {
            return None;
        }
        // Chained or multiplexed Ogg streams need per-stream accounting. Until the static
        // estimator supports that safely, leave them to streaming instead of underestimating.
        let serial = u32::from_le_bytes(bytes[offset + 14..offset + 18].try_into().ok()?);
        if stream_serial.is_some_and(|known| known != serial) {
            return None;
        }
        stream_serial = Some(serial);
        let page_sequence = u32::from_le_bytes(bytes[offset + 18..offset + 22].try_into().ok()?);
        if page_sequence != expected_page_sequence {
            return None;
        }
        expected_page_sequence = expected_page_sequence.checked_add(1)?;
        let granule = u64::from_le_bytes(bytes[offset + 6..offset + 14].try_into().ok()?);
        if granule != u64::MAX {
            max_granule = Some(max_granule.map_or(granule, |old: u64| old.max(granule)));
        }
        let segment_count = bytes[offset + 26] as usize;
        let table_start = offset + 27;
        let table_end = table_start.checked_add(segment_count)?;
        let lacing = bytes.get(table_start..table_end)?;
        let payload_len: usize = lacing.iter().map(|v| *v as usize).sum();
        let payload_start = table_end;
        let payload_end = payload_start.checked_add(payload_len)?;
        let payload = bytes.get(payload_start..payload_end)?;
        let mut cursor = 0usize;
        for segment in lacing {
            let count = *segment as usize;
            if packet_prefix.len() < 30 {
                let needed = 30 - packet_prefix.len();
                packet_prefix.extend_from_slice(&payload[cursor..cursor + count.min(needed)]);
            }
            cursor += count;
            // Continue walking the page's lacing table after the first packet boundary. The
            // identification packet may end before later packets in the same page; stopping here
            // would fail to reach the final granule position on common short Ogg files.
        }
        offset = payload_end;
    }
    if packet_prefix.len() < 16 || packet_prefix[0] != 1 || &packet_prefix[1..7] != b"vorbis" {
        return None;
    }
    let channels = packet_prefix[11];
    let frames = max_granule?;
    if !(1..=8).contains(&channels) || frames == 0 || frames > usize::MAX as u64 {
        return None;
    }
    Some(frames as usize)
}

fn mp3_frames(bytes: &[u8]) -> Option<usize> {
    let mut offset = 0usize;
    if bytes.starts_with(b"ID3") {
        if bytes.len() < 10 {
            return None;
        }
        let size = ((bytes[6] as usize & 0x7f) << 21)
            | ((bytes[7] as usize & 0x7f) << 14)
            | ((bytes[8] as usize & 0x7f) << 7)
            | (bytes[9] as usize & 0x7f);
        offset = 10usize.checked_add(size)?;
        if bytes[5] & 0x10 != 0 {
            offset = offset.checked_add(10)?;
        }
    }
    const MPEG1_L3: [usize; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    const MPEG2_L3: [usize; 16] = [
        0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ];
    let mut total_frames = 0usize;
    let mut found = false;
    while offset.checked_add(4)? <= bytes.len() {
        if bytes.get(offset..offset + 3) == Some(b"TAG") {
            break;
        }
        if bytes.get(offset..offset + 8) == Some(b"APETAGEX") {
            break;
        }
        let header = u32::from_be_bytes(bytes[offset..offset + 4].try_into().ok()?);
        if header >> 21 != 0x7ff {
            if found {
                return None;
            }
            offset += 1;
            continue;
        }
        let version = (header >> 19) & 0x3;
        let layer = (header >> 17) & 0x3;
        let bitrate_index = ((header >> 12) & 0xf) as usize;
        let sample_index = ((header >> 10) & 0x3) as usize;
        let padding = ((header >> 9) & 1) as usize;
        if version == 1
            || layer != 1
            || bitrate_index == 0
            || bitrate_index == 15
            || sample_index == 3
        {
            offset += 1;
            continue;
        }
        let bitrate = if version == 3 {
            MPEG1_L3[bitrate_index]
        } else {
            MPEG2_L3[bitrate_index]
        } * 1000;
        let base_rate = [44_100usize, 48_000, 32_000][sample_index];
        let rate = match version {
            3 => base_rate,
            2 => base_rate / 2,
            0 => base_rate / 4,
            _ => unreachable!(),
        };
        let samples_per_frame = if version == 3 { 1_152 } else { 576 };
        let frame_len = (if version == 3 { 144usize } else { 72usize })
            .checked_mul(bitrate)?
            .checked_div(rate)?
            .checked_add(padding)?;
        if frame_len < 24 || offset.checked_add(frame_len)? > bytes.len() {
            return None;
        }
        total_frames = total_frames.checked_add(samples_per_frame)?;
        offset += frame_len;
        found = true;
    }
    if found {
        Some(total_frames)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_pcm_wav_metadata_without_decoding() {
        let wav = include_bytes!("../examples/assets/tone.wav");
        assert!(estimate_static_pcm_bytes(wav).is_some());
    }

    #[test]
    fn estimates_all_enabled_decoder_container_families() {
        let fixtures: [(&str, &[u8]); 5] = [
            ("WAV/PCM", include_bytes!("../examples/assets/tone.wav")),
            (
                "AIFF/PCM",
                include_bytes!("../examples/assets/decoder_fixtures/tone.aiff"),
            ),
            (
                "FLAC",
                include_bytes!("../examples/assets/decoder_fixtures/tone.flac"),
            ),
            (
                "MP3",
                include_bytes!("../examples/assets/decoder_fixtures/tone.mp3"),
            ),
            (
                "Ogg Vorbis",
                include_bytes!("../examples/assets/decoder_fixtures/tone.ogg"),
            ),
        ];
        for (format, bytes) in fixtures {
            let estimate = estimate_static_pcm_bytes(bytes)
                .unwrap_or_else(|| panic!("{format} metadata was not recognized"));
            assert!(estimate > 0, "{format} must estimate non-empty PCM storage");
        }
    }

    #[test]
    fn wav_estimator_rejects_inconsistent_block_alignment_and_zero_sample_rate() {
        let mut bad_alignment = include_bytes!("../examples/assets/tone.wav").to_vec();
        // Canonical PCM fmt chunk fields: sample rate at 24..28, block align at 32..34.
        bad_alignment[32..34].copy_from_slice(&1u16.to_le_bytes());
        assert_eq!(wav_frames(&bad_alignment), None);

        let mut zero_sample_rate = include_bytes!("../examples/assets/tone.wav").to_vec();
        zero_sample_rate[24..28].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(wav_frames(&zero_sample_rate), None);
    }

    #[test]
    fn malformed_and_unrecognized_inputs_are_not_admitted_to_static_cache() {
        assert_eq!(estimate_static_pcm_bytes(b"not audio"), None);
        assert_eq!(estimate_static_pcm_bytes(b"RIFF\0\0\0\0WAVE"), None);
    }
}
