use crate::{event::EventManager, protobuf};

pub use macro_derive::GameEvent;

pub type ListKeysT = [(u32, String)];
pub type KeyT = protobuf::c_msg_source1_legacy_game_event::KeyT;

pub type GameEventSerializerFactory =
    fn(keys: &ListKeysT) -> Result<Box<dyn GameEventSerializer>, std::io::Error>;

pub trait GameEventSerializer: Send + Sync {
    fn parse_and_dispatch_event(
        &self,
        keys: Vec<KeyT>,
        event_manager: &mut EventManager,
        state: &crate::CsDemoParserState,
    ) -> Result<(), std::io::Error>;
}
