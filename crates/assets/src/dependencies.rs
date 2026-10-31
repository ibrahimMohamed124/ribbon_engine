//! Typed asset dependency metadata and dependency-first scheduling order.
//!
//! The graph is deliberately independent from concrete loaders: an application can register
//! dependencies discovered by its importer/editor and then submit the returned typed requests to
//! `AssetServer` using the appropriate Rust asset type. This avoids pretending the engine can
//! instantiate an arbitrary Rust type from a path string.

use std::any::{type_name, TypeId};
use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::{Asset, LoadError};

/// Strongly typed identity for an asset path in a dependency graph.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct AssetDependencyKey {
    type_id: TypeId,
    /// Fully qualified Rust type name for diagnostics and serialization by callers.
    pub type_name: &'static str,
    /// Source path as registered by the caller.
    pub path: String,
}

impl AssetDependencyKey {
    /// Construct a typed key without requiring the asset to be loaded.
    pub fn of<T: Asset>(path: impl Into<String>) -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            type_name: type_name::<T>(),
            path: path.into(),
        }
    }

    /// Whether this key identifies the requested Rust asset type.
    pub fn is<T: Asset>(&self) -> bool {
        self.type_id == TypeId::of::<T>()
    }

    pub(crate) fn type_id(&self) -> TypeId {
        self.type_id
    }
}

/// Whether a dependency must be available before its parent can be activated.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AssetDependencyKind {
    /// Parent readiness requires this dependency.
    Required,
    /// Useful to preload, but does not block parent readiness.
    Optional,
}

/// One typed dependency declared for a parent asset.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetDependency {
    pub key: AssetDependencyKey,
    pub kind: AssetDependencyKind,
}

impl AssetDependency {
    pub fn required<T: Asset>(path: impl Into<String>) -> Self {
        Self {
            key: AssetDependencyKey::of::<T>(path),
            kind: AssetDependencyKind::Required,
        }
    }

    pub fn optional<T: Asset>(path: impl Into<String>) -> Self {
        Self {
            key: AssetDependencyKey::of::<T>(path),
            kind: AssetDependencyKind::Optional,
        }
    }
}

/// Current AssetServer state for one required dependency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetDependencyLoadState {
    NotRequested,
    Unloaded,
    Loading,
    Loaded,
    Failed(LoadError),
}

/// Required dependency and its current cache state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetDependencyStatus {
    pub key: AssetDependencyKey,
    pub state: AssetDependencyLoadState,
}

/// Invalid dependency declaration or a cycle in the asset dependency graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssetDependencyError {
    EmptyPath {
        type_name: &'static str,
    },
    Duplicate {
        parent: AssetDependencyKey,
        dependency: AssetDependencyKey,
    },
    Cycle(Vec<AssetDependencyKey>),
}

impl fmt::Display for AssetDependencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPath { type_name } => {
                write!(f, "asset dependency path for `{type_name}` cannot be empty")
            }
            Self::Duplicate { parent, dependency } => write!(
                f,
                "asset `{}` ({}) declares dependency `{}` ({}) more than once",
                parent.path, parent.type_name, dependency.path, dependency.type_name
            ),
            Self::Cycle(cycle) => {
                f.write_str("asset dependency cycle detected: ")?;
                for (index, key) in cycle.iter().enumerate() {
                    if index != 0 {
                        f.write_str(" -> ")?;
                    }
                    write!(f, "{} ({})", key.path, key.type_name)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for AssetDependencyError {}

/// Dependency graph that validates cycles and returns deterministic leaf-first load order.
#[derive(Clone, Debug, Default)]
pub struct AssetDependencyGraph {
    edges: HashMap<AssetDependencyKey, Vec<AssetDependency>>,
}

impl AssetDependencyGraph {
    /// Add one edge from `parent_path` to a typed dependency path. Cycles and duplicate edges are
    /// rejected atomically; an invalid edge is never left in the graph.
    pub fn add<P: Asset, D: Asset>(
        &mut self,
        parent_path: impl Into<String>,
        dependency_path: impl Into<String>,
        kind: AssetDependencyKind,
    ) -> Result<(), AssetDependencyError> {
        let parent = AssetDependencyKey::of::<P>(parent_path);
        let dependency =
            AssetDependency::from_key(AssetDependencyKey::of::<D>(dependency_path), kind);
        validate_key(&parent)?;
        validate_key(&dependency.key)?;
        if parent == dependency.key {
            return Err(AssetDependencyError::Cycle(vec![parent.clone(), parent]));
        }
        let edges = self.edges.entry(parent.clone()).or_default();
        if edges.iter().any(|entry| entry.key == dependency.key) {
            return Err(AssetDependencyError::Duplicate {
                parent,
                dependency: dependency.key,
            });
        }
        edges.push(dependency);
        if let Err(error) = self.validate() {
            let remove_parent = if let Some(edges) = self.edges.get_mut(&parent) {
                edges.pop();
                edges.is_empty()
            } else {
                false
            };
            if remove_parent {
                self.edges.remove(&parent);
            }
            return Err(error);
        }
        if let Some(edges) = self.edges.get_mut(&parent) {
            edges.sort_by(|left, right| {
                left.key
                    .type_name
                    .cmp(right.key.type_name)
                    .then_with(|| left.key.path.cmp(&right.key.path))
            });
        }
        Ok(())
    }

    /// Replace all dependency edges for one parent. The operation is transactional: on error the
    /// old edges are restored.
    pub fn replace<P: Asset>(
        &mut self,
        parent_path: impl Into<String>,
        dependencies: impl IntoIterator<Item = AssetDependency>,
    ) -> Result<(), AssetDependencyError> {
        let parent = AssetDependencyKey::of::<P>(parent_path);
        validate_key(&parent)?;
        let mut replacement: Vec<_> = dependencies.into_iter().collect();
        let mut seen = HashSet::new();
        for dependency in &replacement {
            validate_key(&dependency.key)?;
            if !seen.insert(dependency.key.clone()) {
                return Err(AssetDependencyError::Duplicate {
                    parent,
                    dependency: dependency.key.clone(),
                });
            }
        }
        replacement.sort_by(|left, right| {
            left.key
                .type_name
                .cmp(right.key.type_name)
                .then_with(|| left.key.path.cmp(&right.key.path))
        });
        let previous = self.edges.insert(parent.clone(), replacement);
        if let Err(error) = self.validate() {
            match previous {
                Some(edges) => {
                    self.edges.insert(parent, edges);
                }
                None => {
                    self.edges.remove(&parent);
                }
            }
            return Err(error);
        }
        Ok(())
    }

    /// Direct dependencies registered for an asset.
    pub fn dependencies_of<T: Asset>(&self, path: impl AsRef<str>) -> &[AssetDependency] {
        let key = AssetDependencyKey::of::<T>(path.as_ref().to_owned());
        self.edges.get(&key).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Deterministic dependency-first ordering, with the requested parent as the last element.
    /// Optional edges are included in order so callers can preload them, but should not gate the
    /// parent's readiness on those entries.
    pub fn order_for<T: Asset>(
        &self,
        path: impl AsRef<str>,
    ) -> Result<Vec<AssetDependencyKey>, AssetDependencyError> {
        let root = AssetDependencyKey::of::<T>(path.as_ref().to_owned());
        validate_key(&root)?;
        let mut visited = HashSet::new();
        let mut active = Vec::new();
        let mut output = Vec::new();
        self.visit(&root, &mut visited, &mut active, &mut output)?;
        Ok(output)
    }

    /// Return only the transitive required-dependency closure, dependency-first. Optional edges
    /// are deliberately excluded from readiness gating, even when their target has dependencies.
    pub fn required_order_for<T: Asset>(
        &self,
        path: impl AsRef<str>,
    ) -> Result<Vec<AssetDependencyKey>, AssetDependencyError> {
        let root = AssetDependencyKey::of::<T>(path.as_ref().to_owned());
        validate_key(&root)?;
        let mut visited = HashSet::new();
        let mut active = Vec::new();
        let mut output = Vec::new();
        self.visit_required(&root, &mut visited, &mut active, &mut output)?;
        // `visit_required` includes the root for cycle/order consistency; callers request only
        // its dependencies, so remove the final root entry.
        if output.last() == Some(&root) {
            output.pop();
        }
        Ok(output)
    }

    /// Validate every connected component, not only a particular root.
    pub fn validate(&self) -> Result<(), AssetDependencyError> {
        let mut roots: Vec<_> = self.edges.keys().cloned().collect();
        roots.sort_by(|left, right| {
            left.type_name
                .cmp(right.type_name)
                .then_with(|| left.path.cmp(&right.path))
        });
        let mut visited = HashSet::new();
        let mut active = Vec::new();
        let mut output = Vec::new();
        for root in roots {
            self.visit(&root, &mut visited, &mut active, &mut output)?;
        }
        Ok(())
    }

    /// Remove a typed asset node and all incoming/outgoing edges referencing it.
    pub fn remove<T: Asset>(&mut self, path: impl AsRef<str>) -> usize {
        let key = AssetDependencyKey::of::<T>(path.as_ref().to_owned());
        let mut removed: usize = if self.edges.remove(&key).is_some() {
            1
        } else {
            0
        };
        for dependencies in self.edges.values_mut() {
            let old_len = dependencies.len();
            dependencies.retain(|dependency| dependency.key != key);
            removed = removed.saturating_add(old_len.saturating_sub(dependencies.len()));
        }
        removed
    }

    fn visit_required(
        &self,
        key: &AssetDependencyKey,
        visited: &mut HashSet<AssetDependencyKey>,
        active: &mut Vec<AssetDependencyKey>,
        output: &mut Vec<AssetDependencyKey>,
    ) -> Result<(), AssetDependencyError> {
        if visited.contains(key) {
            return Ok(());
        }
        if let Some(position) = active.iter().position(|entry| entry == key) {
            let mut cycle = active[position..].to_vec();
            cycle.push(key.clone());
            return Err(AssetDependencyError::Cycle(cycle));
        }
        active.push(key.clone());
        if let Some(dependencies) = self.edges.get(key) {
            for dependency in dependencies
                .iter()
                .filter(|dependency| dependency.kind == AssetDependencyKind::Required)
            {
                self.visit_required(&dependency.key, visited, active, output)?;
            }
        }
        active.pop();
        visited.insert(key.clone());
        output.push(key.clone());
        Ok(())
    }

    fn visit(
        &self,
        key: &AssetDependencyKey,
        visited: &mut HashSet<AssetDependencyKey>,
        active: &mut Vec<AssetDependencyKey>,
        output: &mut Vec<AssetDependencyKey>,
    ) -> Result<(), AssetDependencyError> {
        if visited.contains(key) {
            return Ok(());
        }
        if let Some(position) = active.iter().position(|entry| entry == key) {
            let mut cycle = active[position..].to_vec();
            cycle.push(key.clone());
            return Err(AssetDependencyError::Cycle(cycle));
        }
        active.push(key.clone());
        if let Some(dependencies) = self.edges.get(key) {
            for dependency in dependencies {
                self.visit(&dependency.key, visited, active, output)?;
            }
        }
        active.pop();
        visited.insert(key.clone());
        output.push(key.clone());
        Ok(())
    }
}

impl AssetDependency {
    fn from_key(key: AssetDependencyKey, kind: AssetDependencyKind) -> Self {
        Self { key, kind }
    }
}

fn validate_key(key: &AssetDependencyKey) -> Result<(), AssetDependencyError> {
    if key.path.trim().is_empty() {
        Err(AssetDependencyError::EmptyPath {
            type_name: key.type_name,
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{AssetDependency, AssetDependencyError, AssetDependencyGraph, AssetDependencyKind};

    struct Model;
    struct Texture;
    struct Material;

    #[test]
    fn dependency_order_is_leaf_first_and_deduplicates_shared_nodes() {
        let mut graph = AssetDependencyGraph::default();
        graph
            .add::<Model, Material>("hero.model", "hero.material", AssetDependencyKind::Required)
            .unwrap();
        graph
            .add::<Material, Texture>(
                "hero.material",
                "hero.basecolor",
                AssetDependencyKind::Required,
            )
            .unwrap();
        graph
            .add::<Model, Texture>(
                "hero.model",
                "hero.basecolor",
                AssetDependencyKind::Optional,
            )
            .unwrap();
        let order = graph.order_for::<Model>("hero.model").unwrap();
        assert_eq!(order.len(), 3);
        assert_eq!(order[0].path, "hero.basecolor");
        assert_eq!(order[1].path, "hero.material");
        assert_eq!(order[2].path, "hero.model");
    }

    #[test]
    fn cycles_are_rejected_without_mutating_the_graph() {
        let mut graph = AssetDependencyGraph::default();
        graph
            .add::<Model, Material>("hero.model", "hero.material", AssetDependencyKind::Required)
            .unwrap();
        let result = graph.add::<Material, Model>(
            "hero.material",
            "hero.model",
            AssetDependencyKind::Required,
        );
        assert!(matches!(result, Err(AssetDependencyError::Cycle(_))));
        assert_eq!(graph.order_for::<Model>("hero.model").unwrap().len(), 2);
    }

    #[test]
    fn duplicate_dependencies_are_rejected() {
        let mut graph = AssetDependencyGraph::default();
        graph
            .add::<Model, Texture>(
                "hero.model",
                "hero.basecolor",
                AssetDependencyKind::Required,
            )
            .unwrap();
        assert!(matches!(
            graph.add::<Model, Texture>(
                "hero.model",
                "hero.basecolor",
                AssetDependencyKind::Optional
            ),
            Err(AssetDependencyError::Duplicate { .. })
        ));
    }

    #[test]
    fn replace_is_transactional_when_replacement_introduces_a_cycle() {
        let mut graph = AssetDependencyGraph::default();
        graph
            .add::<Model, Texture>(
                "hero.model",
                "hero.basecolor",
                AssetDependencyKind::Required,
            )
            .unwrap();
        graph
            .add::<Material, Model>("hero.material", "hero.model", AssetDependencyKind::Required)
            .unwrap();
        let result = graph.replace::<Model>(
            "hero.model",
            [AssetDependency::required::<Material>("hero.material")],
        );
        assert!(matches!(result, Err(AssetDependencyError::Cycle(_))));
        assert_eq!(
            graph.dependencies_of::<Model>("hero.model")[0].key.path,
            "hero.basecolor"
        );
        assert!(graph.validate().is_ok());
    }
    #[test]
    fn required_order_ignores_optional_edges_and_is_dependency_first() {
        let mut graph = AssetDependencyGraph::default();
        graph
            .add::<Model, Material>("hero.model", "hero.material", AssetDependencyKind::Required)
            .unwrap();
        graph
            .add::<Material, Texture>(
                "hero.material",
                "hero.basecolor",
                AssetDependencyKind::Required,
            )
            .unwrap();
        graph
            .add::<Model, Texture>(
                "hero.model",
                "optional.preview",
                AssetDependencyKind::Optional,
            )
            .unwrap();
        let required = graph.required_order_for::<Model>("hero.model").unwrap();
        assert_eq!(required.len(), 2);
        assert_eq!(required[0].path, "hero.basecolor");
        assert_eq!(required[1].path, "hero.material");
    }
}
