use super::*;

pub(super) fn remove_asset_from_map<T: Asset>(
    assets: &mut ErasedAssetMap,
    id: AssetId,
    generation: u32,
) -> bool {
    assets
        .get_mut(&TypeId::of::<T>())
        .and_then(|storage| storage.downcast_mut::<Assets<T>>())
        .is_some_and(|storage| storage.remove(Handle::new(id, generation)))
}

pub(crate) fn extension_for(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
}

pub(crate) fn read_from_sources(
    sources: &[SourceSnapshot],
    path: &str,
) -> Result<(Vec<u8>, u64, Option<SystemTime>), LoadError> {
    read_from_sources_cancellable(sources, path, &CancellationToken::default())
}

pub(crate) fn read_from_sources_cancellable(
    sources: &[SourceSnapshot],
    path: &str,
    cancellation: &CancellationToken,
) -> Result<(Vec<u8>, u64, Option<SystemTime>), LoadError> {
    cancellation.check(path)?;
    if sources.is_empty() {
        return Err(LoadError::new(
            LoadErrorKind::MissingSource,
            path,
            "no asset sources are registered",
        ));
    }
    let mut last_io_error = None;
    for entry in sources {
        cancellation.check(path)?;
        match entry.source.read_cancellable(path, cancellation) {
            Ok(bytes) => {
                cancellation.check(path)?;
                return Ok((bytes, entry.id, entry.source.modified(path)));
            }
            Err(error)
                if error.kind() == io::ErrorKind::Interrupted && cancellation.is_cancelled() =>
            {
                return Err(LoadError::new(
                    LoadErrorKind::Cancelled,
                    path,
                    "asset source read was cancelled",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => last_io_error = Some(error),
        }
    }
    cancellation.check(path)?;
    if let Some(error) = last_io_error {
        Err(LoadError::new(LoadErrorKind::Io, path, error.to_string()))
    } else {
        Err(LoadError::new(
            LoadErrorKind::MissingSource,
            path,
            "no registered source contains this path",
        ))
    }
}
