//! RON-backed authored world-partition descriptions.
//!
//! The asset only describes portable region metadata. Runtime handles, physics objects and ECS
//! entities are created by the streaming coordinator after the description has been decoded.

use glam::{Quat, Vec3};
use ribbon_assets::{AssetLoader, LoadContext, LoadError, LoadErrorKind};
use ribbon_scene::Transform3D;
use serde::{Deserialize, Serialize};

use crate::{RegionBounds, RegionDefinition, RegionId, RegionModel, RegionPolicy};

/// Versioned authored partition asset loaded from `.worldpart` files.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldPartitionAsset {
    pub format_version: u32,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub regions: Vec<WorldRegionAsset>,
}

/// Serializable region record. Bounds are world-space AABBs; transforms use `[x, y, z, w]`
/// quaternion order and are normalized during conversion to runtime descriptors.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldRegionAsset {
    pub id: String,
    pub name: String,
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    #[serde(default)]
    pub scene_doc_path: Option<String>,
    #[serde(default)]
    pub models: Vec<WorldModelPlacementAsset>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub neighbors: Vec<String>,
    #[serde(default)]
    pub policy: WorldRegionPolicyAsset,
}

/// One path-backed model placement in an authored region.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorldModelPlacementAsset {
    pub asset_path: String,
    #[serde(default)]
    pub translation: [f32; 3],
    #[serde(default = "identity_rotation")]
    pub rotation_xyzw: [f32; 4],
    #[serde(default = "one_scale")]
    pub scale: [f32; 3],
}

/// Serializable copy of the region's observer and activation policy.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldRegionPolicyAsset {
    pub preload_distance: f32,
    pub activation_distance: f32,
    pub unload_distance: f32,
    pub base_priority: i32,
    pub auto_activate: bool,
    pub preview_when_ready: bool,
}

impl Default for WorldRegionPolicyAsset {
    fn default() -> Self {
        let policy = RegionPolicy::default();
        Self {
            preload_distance: policy.preload_distance,
            activation_distance: policy.activation_distance,
            unload_distance: policy.unload_distance,
            base_priority: policy.base_priority,
            auto_activate: policy.auto_activate,
            preview_when_ready: policy.preview_when_ready,
        }
    }
}

fn identity_rotation() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

fn one_scale() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

impl WorldPartitionAsset {
    /// Convert and validate the entire authored partition before any region is registered.
    pub fn region_definitions(&self) -> Result<Vec<RegionDefinition>, String> {
        if self.format_version != 1 {
            return Err(format!(
                "unsupported world-partition format version {} (supported: 1)",
                self.format_version
            ));
        }
        if self.id.trim().is_empty() || self.name.trim().is_empty() {
            return Err("world-partition ID and name must not be empty".to_owned());
        }
        if self.regions.is_empty() {
            return Err(format!("world partition `{}` contains no regions", self.id));
        }

        let mut seen = std::collections::HashSet::new();
        let mut definitions = Vec::with_capacity(self.regions.len());
        for region in &self.regions {
            if region.id.trim().is_empty() || region.name.trim().is_empty() {
                return Err("world region ID and name must not be empty".to_owned());
            }
            if !seen.insert(region.id.as_str()) {
                return Err(format!(
                    "world partition contains duplicate region `{}`",
                    region.id
                ));
            }
            let bounds = RegionBounds::new(
                Vec3::from_array(region.bounds_min),
                Vec3::from_array(region.bounds_max),
            )
            .map_err(|error| format!("region `{}` has invalid bounds: {error}", region.id))?;
            let policy = RegionPolicy {
                preload_distance: region.policy.preload_distance,
                activation_distance: region.policy.activation_distance,
                unload_distance: region.policy.unload_distance,
                base_priority: region.policy.base_priority,
                auto_activate: region.policy.auto_activate,
                preview_when_ready: region.policy.preview_when_ready,
            };
            if !policy.preload_distance.is_finite()
                || !policy.activation_distance.is_finite()
                || !policy.unload_distance.is_finite()
                || policy.activation_distance < 0.0
                || policy.preload_distance < policy.activation_distance
                || policy.unload_distance < policy.preload_distance
            {
                return Err(format!(
                    "region `{}` has an invalid streaming distance policy",
                    region.id
                ));
            }

            let mut definition = RegionDefinition::new(
                RegionId::new(region.id.clone()),
                region.name.clone(),
                bounds,
            )
            .with_policy(policy)
            .with_dependencies(region.dependencies.iter().cloned().map(RegionId::new))
            .with_neighbors(region.neighbors.iter().cloned().map(RegionId::new));

            if let Some(path) = &region.scene_doc_path {
                if path.trim().is_empty() {
                    return Err(format!("region `{}` has an empty SceneDoc path", region.id));
                }
                definition = definition.with_scene_doc(path.clone());
            }

            for model in &region.models {
                if model.asset_path.trim().is_empty() {
                    return Err(format!(
                        "region `{}` has an empty model asset path",
                        region.id
                    ));
                }
                let translation = Vec3::from_array(model.translation);
                let rotation = Quat::from_array(model.rotation_xyzw);
                let scale = Vec3::from_array(model.scale);
                if !translation.is_finite()
                    || !rotation.is_finite()
                    || rotation.length_squared() <= 1.0e-12
                    || !scale.is_finite()
                    || scale.min_element() <= 0.0
                {
                    return Err(format!(
                        "region `{}` has an invalid model transform",
                        region.id
                    ));
                }
                definition = definition.with_model(RegionModel::new(
                    model.asset_path.clone(),
                    Transform3D {
                        translation,
                        rotation: rotation.normalize(),
                        scale,
                    },
                ));
            }
            let mut dependency_ids = std::collections::HashSet::new();
            for dependency in &definition.dependencies {
                if dependency == &definition.id {
                    return Err(format!("region `{}` cannot depend on itself", region.id));
                }
                if !dependency_ids.insert(dependency) {
                    return Err(format!(
                        "region `{}` contains a duplicate dependency `{}`",
                        region.id,
                        dependency.as_str()
                    ));
                }
            }
            definitions.push(definition);
        }
        Ok(definitions)
    }
}

/// RON deserializer for reusable authored world partitions.
#[derive(Clone, Copy, Debug, Default)]
pub struct WorldPartitionLoader;

impl AssetLoader for WorldPartitionLoader {
    type Asset = WorldPartitionAsset;

    fn extensions(&self) -> &[&str] {
        &["worldpart"]
    }

    fn load(&self, bytes: &[u8], context: &mut LoadContext) -> Result<Self::Asset, LoadError> {
        let source = std::str::from_utf8(bytes).map_err(|error| {
            LoadError::new(
                LoadErrorKind::Decode,
                &context.path,
                format!("world-partition asset is not valid UTF-8: {error}"),
            )
        })?;
        ron::de::from_str(source).map_err(|error| {
            LoadError::new(
                LoadErrorKind::Decode,
                &context.path,
                format!("could not decode world-partition asset: {error}"),
            )
        })
    }

    fn estimated_size_bytes(&self, partition: &Self::Asset) -> u64 {
        let mut bytes = std::mem::size_of::<WorldPartitionAsset>() as u64;
        bytes = bytes
            .saturating_add(partition.id.capacity() as u64)
            .saturating_add(partition.name.capacity() as u64)
            .saturating_add(
                (partition.regions.capacity() as u64)
                    .saturating_mul(std::mem::size_of::<WorldRegionAsset>() as u64),
            );
        for region in &partition.regions {
            bytes = bytes
                .saturating_add(region.id.capacity() as u64)
                .saturating_add(region.name.capacity() as u64)
                .saturating_add(
                    region
                        .scene_doc_path
                        .as_ref()
                        .map_or(0, |path| path.capacity() as u64),
                )
                .saturating_add(
                    (region.models.capacity() as u64)
                        .saturating_mul(std::mem::size_of::<WorldModelPlacementAsset>() as u64),
                )
                .saturating_add(
                    (region.dependencies.capacity() as u64)
                        .saturating_mul(std::mem::size_of::<String>() as u64),
                )
                .saturating_add(
                    (region.neighbors.capacity() as u64)
                        .saturating_mul(std::mem::size_of::<String>() as u64),
                );
            for model in &region.models {
                bytes = bytes.saturating_add(model.asset_path.capacity() as u64);
            }
            for path in region.dependencies.iter().chain(region.neighbors.iter()) {
                bytes = bytes.saturating_add(path.capacity() as u64);
            }
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::{WorldPartitionAsset, WorldPartitionLoader, WorldRegionAsset};
    use ribbon_assets::{AssetLoader, LoadContext};

    fn partition() -> WorldPartitionAsset {
        WorldPartitionAsset {
            format_version: 1,
            id: "test-world".into(),
            name: "Test World".into(),
            regions: vec![WorldRegionAsset {
                id: "field".into(),
                name: "Open Field".into(),
                bounds_min: [-10.0, -1.0, -10.0],
                bounds_max: [10.0, 5.0, 10.0],
                scene_doc_path: Some("world/field.scenedoc".into()),
                models: Vec::new(),
                dependencies: Vec::new(),
                neighbors: Vec::new(),
                policy: Default::default(),
            }],
        }
    }

    #[test]
    fn world_partition_loader_decodes_and_validates_authored_regions() {
        let asset = partition();
        let encoded = ron::ser::to_string(&asset).unwrap();
        let mut context = LoadContext {
            path: "world/test.worldpart".into(),
        };
        let decoded = WorldPartitionLoader
            .load(encoded.as_bytes(), &mut context)
            .unwrap();
        assert_eq!(decoded, asset);
        let definitions = decoded.region_definitions().unwrap();
        assert_eq!(definitions.len(), 1);
        assert_eq!(
            definitions[0].scene_doc_path.as_deref(),
            Some("world/field.scenedoc")
        );
    }

    #[test]
    fn world_partition_loader_rejects_invalid_format_and_non_finite_bounds() {
        let mut invalid_version = partition();
        invalid_version.format_version = 99;
        assert!(invalid_version.region_definitions().is_err());

        let mut invalid_bounds = partition();
        invalid_bounds.regions[0].bounds_min[0] = f32::NAN;
        assert!(invalid_bounds.region_definitions().is_err());
    }

    #[test]
    fn world_partition_loader_reports_bad_ron_and_utf8() {
        let mut context = LoadContext {
            path: "broken.worldpart".into(),
        };
        assert!(WorldPartitionLoader.load(&[0xff], &mut context).is_err());
        assert!(WorldPartitionLoader.load(b"not ron", &mut context).is_err());
    }
}
