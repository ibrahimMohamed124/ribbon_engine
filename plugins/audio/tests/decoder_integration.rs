#![cfg(feature = "audio")]

//! End-to-end tests for the enabled Symphonia/Kira static decoder feature set.
//!
//! Each fixture is decoded through Kira's public static-sound API, not through the
//! metadata estimator, so a decoder removed from the Cargo feature set fails its own test.

use std::io::Cursor;

use kira::sound::static_sound::StaticSoundData;

fn assert_fixture_decodes(format: &str, encoded: &[u8]) {
    let decoded = StaticSoundData::from_cursor(Cursor::new(encoded.to_vec()))
        .unwrap_or_else(|error| panic!("Kira {format} decoder rejected its fixture: {error}"));
    let frames = decoded.frames.as_ref();
    assert!(!frames.is_empty(), "{format} decoded no PCM frames");

    let mut peak = 0.0f32;
    for frame in frames {
        assert!(
            frame.left.is_finite(),
            "{format} produced non-finite left PCM"
        );
        assert!(
            frame.right.is_finite(),
            "{format} produced non-finite right PCM"
        );
        peak = peak.max(frame.left.abs()).max(frame.right.abs());
    }
    assert!(
        peak > 1.0e-4,
        "{format} decoded silence instead of the fixture tone"
    );
}

#[test]
fn decodes_wav_pcm() {
    assert_fixture_decodes("WAV/PCM", include_bytes!("../examples/assets/tone.wav"));
}

#[test]
fn decodes_aiff_pcm() {
    assert_fixture_decodes(
        "AIFF/PCM",
        include_bytes!("../examples/assets/decoder_fixtures/tone.aiff"),
    );
}

#[test]
fn decodes_flac() {
    assert_fixture_decodes(
        "FLAC",
        include_bytes!("../examples/assets/decoder_fixtures/tone.flac"),
    );
}

#[test]
fn decodes_mp3() {
    assert_fixture_decodes(
        "MP3",
        include_bytes!("../examples/assets/decoder_fixtures/tone.mp3"),
    );
}

#[test]
fn decodes_ogg_vorbis() {
    assert_fixture_decodes(
        "Ogg Vorbis",
        include_bytes!("../examples/assets/decoder_fixtures/tone.ogg"),
    );
}
