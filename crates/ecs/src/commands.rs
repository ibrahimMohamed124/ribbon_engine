use crate::{Bundle, Component, Entity, World};

type Deferred = Box<dyn FnOnce(&mut World) + Send + 'static>;

/// Ordered, deferred world mutations applied at a synchronization point.
#[derive(Default)]
pub struct Commands {
    queued: Vec<Deferred>,
}

impl Commands {
    /// Create an empty command buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue spawning a bundle.
    pub fn spawn<B: Bundle + Send + 'static>(&mut self, bundle: B) {
        self.queued.push(Box::new(move |world| {
            world.spawn(bundle);
        }));
    }

    /// Queue despawning an entity. Stale handles are ignored when applied.
    pub fn despawn(&mut self, entity: Entity) {
        self.queued.push(Box::new(move |world| {
            let _ = world.despawn(entity);
        }));
    }

    /// Queue inserting or replacing a component.
    pub fn insert<T: Component>(&mut self, entity: Entity, component: T) {
        self.queued.push(Box::new(move |world| {
            let _ = world.insert(entity, component);
        }));
    }

    /// Queue removing a component. Missing components are ignored when applied.
    pub fn remove<T: Component>(&mut self, entity: Entity) {
        self.queued.push(Box::new(move |world| {
            let _ = world.remove::<T>(entity);
        }));
    }

    /// Apply every queued mutation in submission order, then empty the buffer.
    pub fn apply(&mut self, world: &mut World) {
        for command in std::mem::take(&mut self.queued) {
            command(world);
        }
    }

    /// Number of pending commands.
    pub fn len(&self) -> usize {
        self.queued.len()
    }

    /// Whether the buffer has no pending commands.
    pub fn is_empty(&self) -> bool {
        self.queued.is_empty()
    }
}
