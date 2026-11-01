//! Asynchronous AssetServer decoding for serialized Ribbon scene documents.
//!
//! Scene documents remain renderer-agnostic data. Component codecs declare their external
//! dependencies separately through `PrefabCodecRegistry`, so decoding a document never performs
//! synchronous asset IO or creates GPU resources.

use ribbon_assets::{AssetLoader, AssetMemoryBreakdown, LoadContext, LoadError, LoadErrorKind};
use ribbon_reflect::Value;
use ribbon_scene::SceneDoc;
use std::mem::size_of;

/// Loads RON-encoded [`SceneDoc`] values from files with the `.scenedoc` extension.
#[derive(Clone, Copy, Debug, Default)]
pub struct SceneDocLoader;

impl AssetLoader for SceneDocLoader {
    type Asset = SceneDoc;

    fn extensions(&self) -> &[&str] {
        &["scenedoc"]
    }

    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
        let source = std::str::from_utf8(bytes).map_err(|error| {
            LoadError::new(
                LoadErrorKind::Decode,
                &context.path,
                format!("scene document is not valid UTF-8: {error}"),
            )
        })?;
        SceneDoc::from_ron(source).map_err(|error| {
            LoadError::new(
                LoadErrorKind::Decode,
                &context.path,
                format!("could not decode scene document: {error}"),
            )
        })
    }

    fn estimated_size_bytes(&self, document: &Self::Asset) -> u64 {
        estimate_scene_doc_bytes(document)
    }

    fn memory_breakdown(&self, document: &Self::Asset) -> AssetMemoryBreakdown {
        AssetMemoryBreakdown {
            scene_metadata_bytes: estimate_scene_doc_bytes(document),
            ..AssetMemoryBreakdown::default()
        }
    }
}

fn estimate_value_bytes(value: &Value) -> u64 {
    let base = size_of::<Value>() as u64;
    match value {
        Value::Str(value) => base.saturating_add(value.capacity() as u64),
        Value::List(values) => values.iter().fold(
            base.saturating_add(
                (values.capacity() as u64).saturating_mul(size_of::<Value>() as u64),
            ),
            |bytes, value| bytes.saturating_add(estimate_value_bytes(value)),
        ),
        Value::Struct(fields) => fields.iter().fold(
            base.saturating_add(
                (fields.capacity() as u64).saturating_mul(size_of::<(String, Value)>() as u64),
            ),
            |bytes, (name, value)| {
                bytes
                    .saturating_add(name.capacity() as u64)
                    .saturating_add(estimate_value_bytes(value))
            },
        ),
        Value::Bool(_)
        | Value::Int(_)
        | Value::Float(_)
        | Value::Vec2(_)
        | Value::Vec3(_)
        | Value::Vec4(_) => base,
    }
}

fn estimate_scene_doc_bytes(document: &SceneDoc) -> u64 {
    let mut bytes = size_of::<SceneDoc>() as u64;
    bytes = bytes.saturating_add(
        (document.prefab.entities.capacity() as u64)
            .saturating_mul(size_of::<ribbon_scene::PrefabEntity>() as u64),
    );
    bytes = bytes.saturating_add(document.prefab.name.capacity() as u64);
    for entity in &document.prefab.entities {
        bytes = bytes.saturating_add(
            (entity.components.capacity() as u64)
                .saturating_mul(size_of::<(String, Value)>() as u64),
        );
        for (name, value) in &entity.components {
            bytes = bytes
                .saturating_add(name.capacity() as u64)
                .saturating_add(estimate_value_bytes(value));
        }
    }
    for (key, value) in &document.metadata {
        bytes = bytes
            .saturating_add(key.capacity() as u64)
            .saturating_add(value.capacity() as u64);
    }
    bytes.saturating_add(
        (document.metadata.capacity() as u64).saturating_mul(size_of::<(String, String)>() as u64),
    )
}

#[cfg(test)]
mod tests {
    use super::SceneDocLoader;
    use ribbon_assets::{AssetLoader, LoadContext};
    use ribbon_scene::{Prefab, SceneDoc};
    use std::collections::HashMap;

    #[test]
    fn scene_doc_loader_round_trips_ron_documents() {
        let document = SceneDoc {
            prefab: Prefab {
                format_version: 1,
                name: "streamed scene".into(),
                entities: Vec::new(),
            },
            metadata: HashMap::from([("author".into(), "test".into())]),
        };
        let encoded = document.to_ron().unwrap();
        let mut context = LoadContext {
            path: "levels/test.scenedoc".into(),
        };
        let decoded = SceneDocLoader
            .load(encoded.as_bytes(), &mut context)
            .unwrap();
        assert_eq!(decoded, document);
        let estimated = SceneDocLoader.estimated_size_bytes(&decoded);
        assert!(estimated > std::mem::size_of::<SceneDoc>() as u64);
        assert_eq!(
            SceneDocLoader
                .memory_breakdown(&decoded)
                .scene_metadata_bytes,
            estimated
        );
    }

    #[test]
    fn scene_doc_loader_rejects_invalid_utf8_and_invalid_ron() {
        let mut context = LoadContext {
            path: "levels/broken.scenedoc".into(),
        };
        assert!(SceneDocLoader.load(&[0xff, 0xfe], &mut context).is_err());
        assert!(SceneDocLoader.load(b"not a scene", &mut context).is_err());
    }
}
