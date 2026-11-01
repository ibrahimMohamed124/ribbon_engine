//! Stable asset identities, weak handles, and typed storage.
use std::collections::HashMap;
use std::fmt;
use std::marker::PhantomData;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AssetId(pub(super) u64);

impl AssetId {
    /// Return the stable numeric identity, independent of its current generation.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Copyable weak typed identifier. It does not pin the value against eviction.
pub struct Handle<T> {
    id: AssetId,
    generation: u32,
    marker: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    pub(super) fn new(id: AssetId, generation: u32) -> Self {
        Self {
            id,
            generation,
            marker: PhantomData,
        }
    }

    /// Stable numeric identifier, retained for compatibility with existing renderer caches.
    pub fn id(self) -> u64 {
        self.id.get()
    }

    /// Typed identity, including the stable asset ID but not its generation.
    pub fn asset_id(self) -> AssetId {
        self.id
    }

    /// Generation checked when retrieving the asset.
    pub fn generation(self) -> u32 {
        self.generation
    }
}

impl<T> Default for Handle<T> {
    fn default() -> Self {
        Self {
            id: AssetId(0),
            generation: 0,
            marker: PhantomData,
        }
    }
}

impl<T> Copy for Handle<T> {}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Handle")
            .field("id", &self.id)
            .field("generation", &self.generation)
            .finish()
    }
}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> std::hash::Hash for Handle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.id, state);
        std::hash::Hash::hash(&self.generation, state);
    }
}

/// Typed storage indexed by weak handles.
pub struct Assets<T> {
    values: HashMap<u64, (u32, T)>,
}

impl<T> Default for Assets<T> {
    fn default() -> Self {
        Self {
            values: HashMap::new(),
        }
    }
}

impl<T> Assets<T> {
    /// Get a value when both the identifier and generation match.
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        let (generation, value) = self.values.get(&handle.id.get())?;
        (*generation == handle.generation).then_some(value)
    }

    /// Mutably get a value when the identifier and generation match.
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        let (generation, value) = self.values.get_mut(&handle.id.get())?;
        (*generation == handle.generation).then_some(value)
    }

    /// Number of values currently stored.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the storage is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub(super) fn insert(&mut self, handle: Handle<T>, value: T) {
        self.values
            .insert(handle.id.get(), (handle.generation, value));
    }

    pub(super) fn remove(&mut self, handle: Handle<T>) -> bool {
        if self
            .values
            .get(&handle.id.get())
            .is_some_and(|(generation, _)| *generation == handle.generation)
        {
            self.values.remove(&handle.id.get());
            true
        } else {
            false
        }
    }
}
