//! Typed double-buffered events shared by deterministic engine systems.

use engine_ecs::World;

struct Event<T> {
    id: u64,
    value: T,
}

/// Two-buffer event storage. Events remain readable through two calls to [`update`].
pub struct Events<T> {
    buffers: [Vec<Event<T>>; 2],
    active: usize,
    next_id: u64,
}

impl<T> Default for Events<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Events<T> {
    /// Create empty event storage.
    pub fn new() -> Self {
        Self {
            buffers: [Vec::new(), Vec::new()],
            active: 0,
            next_id: 0,
        }
    }

    /// Add an event with a world-local monotonically increasing identifier.
    pub fn send(&mut self, value: T) -> u64 {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("event id space exhausted");
        self.buffers[self.active].push(Event { id, value });
        id
    }

    /// Advance the buffers, clearing the oldest generation.
    pub fn update(&mut self) {
        let next = 1 - self.active;
        self.buffers[next].clear();
        self.active = next;
    }

    /// Count currently retained events across both buffers.
    pub fn len(&self) -> usize {
        self.buffers.iter().map(Vec::len).sum()
    }

    /// Whether both event buffers are empty.
    pub fn is_empty(&self) -> bool {
        self.buffers.iter().all(Vec::is_empty)
    }
}

/// Per-consumer cursor. Different cursors can independently observe the same events.
pub struct EventCursor<T> {
    next_id: u64,
    marker: std::marker::PhantomData<fn() -> T>,
}

impl<T> Default for EventCursor<T> {
    fn default() -> Self {
        Self {
            next_id: 0,
            marker: std::marker::PhantomData,
        }
    }
}

impl<T> EventCursor<T> {
    /// Read unread events in send order and advance this cursor past them.
    pub fn read<'a>(&mut self, events: &'a Events<T>) -> std::vec::IntoIter<&'a T> {
        let mut unread: Vec<_> = events
            .buffers
            .iter()
            .flat_map(|buffer| buffer.iter())
            .filter(|event| event.id >= self.next_id)
            .map(|event| (event.id, &event.value))
            .collect();
        unread.sort_by_key(|(id, _)| *id);
        if let Some((last_id, _)) = unread.last() {
            self.next_id = last_id.checked_add(1).expect("event id space exhausted");
        }
        unread
            .into_iter()
            .map(|(_, event)| event)
            .collect::<Vec<_>>()
            .into_iter()
    }
}

/// Convenience methods for keeping typed event storage in a world resource.
pub trait WorldEventsExt {
    /// Insert event storage. Repeated registration resets its buffers.
    fn add_event<T: Send + Sync + 'static>(&mut self);
    /// Send an event through its registered storage.
    fn send_event<T: Send + Sync + 'static>(&mut self, event: T);
    /// Borrow registered event storage.
    fn events<T: Send + Sync + 'static>(&self) -> &Events<T>;
    /// Mutably borrow registered event storage.
    fn events_mut<T: Send + Sync + 'static>(&mut self) -> &mut Events<T>;
}

impl WorldEventsExt for World {
    fn add_event<T: Send + Sync + 'static>(&mut self) {
        self.insert_resource(Events::<T>::new());
    }

    fn send_event<T: Send + Sync + 'static>(&mut self, event: T) {
        self.resource_mut::<Events<T>>().send(event);
    }

    fn events<T: Send + Sync + 'static>(&self) -> &Events<T> {
        self.resource::<Events<T>>()
    }

    fn events_mut<T: Send + Sync + 'static>(&mut self) -> &mut Events<T> {
        self.resource_mut::<Events<T>>()
    }
}

#[cfg(test)]
mod tests {
    use super::{EventCursor, Events, WorldEventsExt};
    use engine_ecs::World;

    #[test]
    fn independent_cursors_read_in_order_and_only_once_each() {
        let mut events = Events::new();
        events.send("first");
        events.send("second");
        let mut first = EventCursor::default();
        let mut second = EventCursor::default();
        assert_eq!(
            first.read(&events).copied().collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(first.read(&events).count(), 0);
        assert_eq!(
            second.read(&events).copied().collect::<Vec<_>>(),
            ["first", "second"]
        );
    }

    #[test]
    fn an_event_expires_on_the_second_buffer_update() {
        let mut events = Events::new();
        events.send(7);
        let mut cursor = EventCursor::default();
        events.update();
        assert_eq!(cursor.read(&events).copied().collect::<Vec<_>>(), [7]);
        events.update();
        assert_eq!(cursor.read(&events).count(), 0);
        assert!(events.is_empty());
    }

    #[test]
    fn world_extension_routes_typed_events() {
        let mut world = World::new();
        world.add_event::<u32>();
        world.send_event(12_u32);
        let mut cursor = EventCursor::default();
        assert_eq!(
            cursor
                .read(world.events::<u32>())
                .copied()
                .collect::<Vec<_>>(),
            [12]
        );
    }
}
