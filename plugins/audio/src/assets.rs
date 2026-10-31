//! Integration with Ribbon's typed asset pipeline.
//! The loader preserves encoded bytes; decoding and device-bound work stay backend-owned.
use ribbon_assets::{AssetLoader, LoadContext, LoadError, LoadErrorKind};

#[derive(Clone, Debug)]
pub struct EncodedAudioAsset {
    pub path: String,
    pub bytes: Vec<u8>,
}

impl EncodedAudioAsset {
    pub fn len(&self) -> usize {
        self.bytes.len()
    }
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

#[derive(Default)]
pub struct AudioAssetLoader;

impl AssetLoader for AudioAssetLoader {
    type Asset = EncodedAudioAsset;

    fn extensions(&self) -> &[&str] {
        &["wav", "ogg", "oga", "mp3", "flac", "aiff", "aif"]
    }

    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
        if bytes.is_empty() {
            return Err(LoadError::new(
                LoadErrorKind::Decode,
                &context.path,
                "audio file is empty",
            ));
        }
        let extension = context
            .path
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if !self.extensions().contains(&extension.as_str()) {
            return Err(LoadError::new(
                LoadErrorKind::NoLoader,
                &context.path,
                "unsupported audio extension",
            ));
        }
        Ok(EncodedAudioAsset {
            path: context.path.clone(),
            bytes: bytes.to_vec(),
        })
    }

    fn estimated_size_bytes(&self, asset: &Self::Asset) -> u64 {
        (std::mem::size_of::<Self::Asset>() + asset.path.capacity() + asset.bytes.capacity()) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_audio_bytes_and_rejects_empty_files() {
        let loader = AudioAssetLoader;
        let mut context = LoadContext {
            path: "sfx/door.wav".into(),
        };
        let asset = loader.load(b"RIFF", &mut context).unwrap();
        assert_eq!(asset.bytes, b"RIFF");
        assert!(loader.load(b"", &mut context).is_err());
    }

    #[test]
    fn rejects_unknown_extensions() {
        let mut context = LoadContext {
            path: "audio/thing.txt".into(),
        };
        assert!(AudioAssetLoader.load(b"x", &mut context).is_err());
    }

    #[test]
    fn rejects_opus_until_the_selected_decoder_supports_it() {
        let mut context = LoadContext {
            path: "audio/thing.opus".into(),
        };
        let result = AudioAssetLoader.load(b"not really opus", &mut context);
        assert!(result.is_err());
    }
}
