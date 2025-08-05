pub mod derive;

use log::error;

use crate::{CsDemoParser, game_event::derive::GameEventSerializerFactory, protobuf};

impl<T: std::io::BufRead + Send + Sync> CsDemoParser<T> {
    pub fn register_game_event_serializer_factory(
        &mut self,
        event_name: &'static str,
        factory: GameEventSerializerFactory,
    ) -> Result<(), std::io::Error> {
        if !self.is_fresh() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Cannot register game event serializer after parsing has started",
            ));
        }

        if self.game_event_serializers.contains_key(&event_name) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("Game event serializer for '{event_name}' already exists"),
            ));
        }

        self.game_event_serializers.insert(event_name, factory);

        Ok(())
    }

    pub(super) fn handle_legacy_game_event(
        &mut self,
        msg: protobuf::CMsgSource1LegacyGameEvent,
    ) -> Result<(), std::io::Error> {
        let Some(event_id) = msg.eventid else {
            error!("Missing event ID in legacy game event");
            return Ok(());
        };

        let Some(serializer) = self.game_event_list.get(&event_id) else {
            return Ok(());
        };

        serializer.parse_and_dispatch_event(msg.keys, &mut self.event_manager, &self.state)?;

        Ok(())
    }

    #[cold]
    pub(super) fn handle_legacy_game_event_list(
        &mut self,
        msg: protobuf::CMsgSource1LegacyGameEventList,
    ) -> Result<(), std::io::Error> {
        self.game_event_list.clear();

        for descriptor in msg.descriptors.into_iter() {
            let (Some(event_id), Some(event_name)) = (descriptor.eventid, descriptor.name) else {
                error!("Missing event ID or name in game event list");
                continue;
            };

            let Some(factory) = self.game_event_serializers.get(event_name.as_str()) else {
                continue;
            };

            let keys = descriptor
                .keys
                .into_iter()
                .enumerate()
                .map(|(index, key)| (index as u32, key.name.unwrap_or_default()))
                .collect::<Box<[_]>>();

            let serializer = factory(&keys)?;
            self.game_event_list.insert(event_id, serializer);
        }

        if self.game_event_list.len() != self.game_event_serializers.len() {
            error!(
                "Some game event serializers were not registered: {} registered, {} found",
                self.game_event_serializers.len(),
                self.game_event_list.len()
            );
        }

        Ok(())
    }
}
