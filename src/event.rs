use std::any::{Any, TypeId};

use foldhash::{HashMap, HashMapExt};
use log::error;

use crate::CsDemoParserState;

/// notifies listeners before changing the tick
/// last tick is not notified
pub struct TickEvent {
    pub tick: u32,
    pub tick_interval: f32,
}

impl Event for TickEvent {}

/// notifies whenever demo parsed the first frame
pub struct DemoStartEvent {
    pub network_protocol: i32,
    pub map_name: String,
}

impl Event for DemoStartEvent {}

/// notifies after the parser reaches the last frame
pub struct DemoEndEvent;

impl Event for DemoEndEvent {}

#[cfg(feature = "handle_packet")]
pub struct PacketEvent<T: prost::Message + 'static> {
    pub packet: T,
}

#[cfg(feature = "handle_packet")]
impl<T: prost::Message + 'static> Event for PacketEvent<T> {}

pub trait Event: Any + Send + Sync {}

pub trait EventListener<T: Event>: Send + Sync {
    fn on_event(&mut self, event: &T, state: &CsDemoParserState) -> Result<(), std::io::Error>;
}

impl<F: Fn(&T, &CsDemoParserState) -> Result<(), std::io::Error> + Send + Sync, T: Event>
    EventListener<T> for F
{
    fn on_event(&mut self, event: &T, state: &CsDemoParserState) -> Result<(), std::io::Error> {
        self(event, state)
    }
}

pub struct EventDispatcher<T: Event> {
    listeners: Vec<(u32, Box<dyn EventListener<T>>)>,
    next_listener_id: u32,
}

impl<T: Event> EventDispatcher<T> {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            listeners: Vec::new(),
            next_listener_id: 0,
        }
    }

    pub fn add_listener<L: EventListener<T> + 'static>(&mut self, listener: L) -> u32 {
        self.listeners
            .push((self.next_listener_id, Box::new(listener)));
        let listener_id = self.next_listener_id;
        self.next_listener_id += 1;
        listener_id
    }

    pub fn dispatch(&mut self, event: T, state: &CsDemoParserState) -> Result<(), std::io::Error> {
        for (_, listener) in &mut self.listeners {
            listener.on_event(&event, state)?;
        }

        Ok(())
    }

    pub fn remove_listener(&mut self, id: u32) {
        if let Some(pos) = self
            .listeners
            .iter()
            .position(|(listener_id, _)| *listener_id == id)
        {
            self.listeners.remove(pos);
        } else {
            error!("Listener with ID {id} not found");
        }
    }
}

#[derive(Default)]
pub struct EventManager {
    event_listeners: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl EventManager {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            event_listeners: HashMap::new(),
        }
    }

    pub fn register_listener<L: EventListener<E> + 'static, E: Event>(
        &mut self,
        listener: L,
    ) -> u32 {
        let type_id = TypeId::of::<E>();
        let listeners = self
            .event_listeners
            .entry(type_id)
            .or_insert_with(|| Box::new(EventDispatcher::<E>::new()));

        let dispatcher = listeners.downcast_mut::<EventDispatcher<E>>().unwrap();
        dispatcher.add_listener(listener)
    }

    pub fn remove_listener<E: Event>(&mut self, listener_id: u32) -> bool {
        let type_id = TypeId::of::<E>();

        if let Some(listeners) = self.event_listeners.get_mut(&type_id) {
            let dispatcher = listeners.downcast_mut::<EventDispatcher<E>>().unwrap();
            dispatcher.remove_listener(listener_id);
            return true;
        }

        false
    }

    pub fn notify_listeners<E: Event>(
        &mut self,
        event: E,
        state: &CsDemoParserState,
    ) -> Result<(), std::io::Error> {
        let type_id = TypeId::of::<E>();
        if let Some(listeners) = self.event_listeners.get_mut(&type_id) {
            let dispatcher = listeners.downcast_mut::<EventDispatcher<E>>().unwrap();
            dispatcher.dispatch(event, state)
        } else {
            Ok(())
        }
    }
}
