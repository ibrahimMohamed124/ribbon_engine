use hecs::{
    Bundle, Component, ComponentError, Entity, NoSuchEntity, Query, QueryBorrow, QueryMut, Ref,
    RefMut,
};
use std::any::{type_name, Any, TypeId};
use std::collections::HashMap;
use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};

/// Heterogeneous resources keyed by their concrete Rust type.
#[derive(Default)]
pub struct Resources {
    values: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Resources {
    /// Store a resource, returning the previous value of that type if present.
    pub fn insert<T: Any + Send + Sync>(&mut self, value: T) -> Option<T> {
        self.values
            .insert(TypeId::of::<T>(), Box::new(value))
            .and_then(|previous| previous.downcast::<T>().ok().map(|boxed| *boxed))
    }

    /// Borrow a resource by type.
    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref())
    }

    /// Mutably borrow a resource by type.
    pub fn get_mut<T: Any + Send + Sync>(&mut self) -> Option<&mut T> {
        self.values
            .get_mut(&TypeId::of::<T>())
            .and_then(|value| value.downcast_mut())
    }

    /// Remove a resource by type.
    pub fn remove<T: Any + Send + Sync>(&mut self) -> Option<T> {
        self.values
            .remove(&TypeId::of::<T>())
            .and_then(|value| value.downcast::<T>().ok().map(|boxed| *boxed))
    }
}

/// Engine ECS world. hecs remains encapsulated behind this type.
pub struct World {
    inner: hecs::World,
    resources: Resources,
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

impl World {
    /// Create an empty world.
    pub fn new() -> Self {
        Self {
            inner: hecs::World::new(),
            resources: Resources::default(),
        }
    }

    /// Spawn an entity with a component bundle.
    pub fn spawn<B: Bundle + 'static>(&mut self, bundle: B) -> Entity {
        self.inner.spawn(bundle)
    }

    /// Despawn an entity, returning an error for a stale or unknown handle.
    pub fn despawn(&mut self, entity: Entity) -> Result<(), NoSuchEntity> {
        self.inner.despawn(entity)
    }

    /// Check whether an entity handle still refers to a live entity.
    pub fn contains(&self, entity: Entity) -> bool {
        self.inner.contains(entity)
    }

    /// Add or replace one component on an entity.
    pub fn insert<T: Component>(
        &mut self,
        entity: Entity,
        component: T,
    ) -> Result<(), NoSuchEntity> {
        self.inner.insert_one(entity, component)
    }

    /// Remove one component from an entity.
    pub fn remove<T: Component>(&mut self, entity: Entity) -> Result<T, ComponentError> {
        self.inner.remove_one(entity)
    }

    /// Borrow one entity component, if both the entity and component exist.
    pub fn get<T: Component>(&self, entity: Entity) -> Option<Ref<'_, T>> {
        self.inner.get::<&T>(entity).ok()
    }

    /// Mutably borrow one entity component, if both the entity and component exist.
    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<RefMut<'_, T>> {
        self.inner.get::<&mut T>(entity).ok()
    }

    /// Iterate over components matching a hecs query.
    pub fn query<Q: Query>(&self) -> QueryBorrow<'_, Q> {
        self.inner.query::<Q>()
    }

    /// Mutably iterate over components matching a hecs query.
    pub fn query_mut<Q: Query>(&mut self) -> QueryMut<'_, Q> {
        self.inner.query_mut::<Q>()
    }

    /// Number of live entities.
    pub fn len(&self) -> usize {
        self.inner.len() as usize
    }

    /// Whether this world contains no entities.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Store a world resource, returning the previous resource of that type.
    pub fn insert_resource<T: Any + Send + Sync>(&mut self, value: T) -> Option<T> {
        self.resources.insert(value)
    }

    /// Borrow a required resource; the panic names its missing type.
    pub fn resource<T: Any + Send + Sync>(&self) -> &T {
        self.try_resource::<T>()
            .unwrap_or_else(|| panic!("required world resource `{}` is missing", type_name::<T>()))
    }

    /// Borrow a resource if it exists.
    pub fn try_resource<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.resources.get::<T>()
    }

    /// Mutably borrow a required resource; the panic names its missing type.
    pub fn resource_mut<T: Any + Send + Sync>(&mut self) -> &mut T {
        if !self.resources.values.contains_key(&TypeId::of::<T>()) {
            panic!("required world resource `{}` is missing", type_name::<T>());
        }
        self.resources
            .get_mut::<T>()
            .expect("resource type id was checked")
    }

    /// Remove a resource if present.
    pub fn remove_resource<T: Any + Send + Sync>(&mut self) -> Option<T> {
        self.resources.remove::<T>()
    }

    /// Temporarily remove a resource so a closure can also mutably access the world.
    /// The resource is restored even if the closure unwinds.
    pub fn resource_scope<T, R>(&mut self, function: impl FnOnce(&mut Self, &mut T) -> R) -> R
    where
        T: Any + Send + Sync,
    {
        let mut resource = self
            .resources
            .remove::<T>()
            .unwrap_or_else(|| panic!("required world resource `{}` is missing", type_name::<T>()));
        let result = catch_unwind(AssertUnwindSafe(|| function(self, &mut resource)));
        self.resources.insert(resource);
        match result {
            Ok(value) => value,
            Err(payload) => resume_unwind(payload),
        }
    }
}
