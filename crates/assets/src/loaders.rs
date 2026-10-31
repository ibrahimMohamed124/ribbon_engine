use super::*;

/// Byte asset used with [`BytesLoader`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BytesAsset(pub Vec<u8>);

/// UTF-8 text asset used with [`TextLoader`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextAsset(pub String);

/// Loader that keeps the source bytes unchanged.
pub struct BytesLoader;

impl AssetLoader for BytesLoader {
    type Asset = BytesAsset;

    fn extensions(&self) -> &[&str] {
        &["bin", "bytes"]
    }

    fn load(&self, bytes: &[u8], _context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
        Ok(BytesAsset(bytes.to_vec()))
    }

    fn estimated_size_bytes(&self, asset: &Self::Asset) -> u64 {
        std::mem::size_of::<Self::Asset>() as u64 + asset.0.capacity() as u64
    }
}

/// Loader for UTF-8 text files.
pub struct TextLoader;

impl AssetLoader for TextLoader {
    type Asset = TextAsset;

    fn extensions(&self) -> &[&str] {
        &["txt", "text"]
    }

    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
        String::from_utf8(bytes.to_vec())
            .map(TextAsset)
            .map_err(|error| {
                LoadError::new(LoadErrorKind::Decode, &context.path, error.to_string())
            })
    }

    fn estimated_size_bytes(&self, asset: &Self::Asset) -> u64 {
        std::mem::size_of::<Self::Asset>() as u64 + asset.0.capacity() as u64
    }
}
