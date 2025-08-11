pub mod bit;
pub mod entity;
pub mod event;
pub mod game_event;
pub mod string_table;

pub mod protobuf {
    include!(concat!(env!("OUT_DIR"), "/game_messages.rs"));
}

use foldhash::{HashMap, HashMapExt};
use log::warn;
use std::any::Any;
use std::io::Cursor;
use std::sync::Arc;

use bitstream_io::{BitRead, BitReader};
use bytes::{Bytes, BytesMut};

use crate::bit::BitReaderExt;
use crate::entity::EntitySerializerCreator;
use crate::entity::fieldpath::FieldPathFixed;
use crate::entity::list::EntityList;
use crate::entity::serializer::EntityClassSerializer;
use crate::event::{DemoEndEvent, DemoStartEvent, Event, EventManager, TickEvent};
use crate::game_event::derive::{GameEventSerializer, GameEventSerializerFactory};
use crate::protobuf::{EBaseGameEvents, EDemoCommands, SvcMessages};
use crate::string_table::{BaselineStringTableParser, StringTable};

// 256 KiB
const BUFFER_SIZE: usize = 256 * 1024;

pub struct CsDemoParser<T: std::io::BufRead + Send + Sync> {
    reader: T,

    pub event_manager: EventManager,
    #[allow(clippy::type_complexity)]
    #[cfg(feature = "handle_packet")]
    packet_handler: HashMap<
        u64,
        Box<dyn Fn(Bytes, &mut EventManager, &CsDemoParserState) -> Result<(), std::io::Error>>,
    >,
    pub state: CsDemoParserState,

    class_info: HashMap<u32, String>,
    class_id_size: u32,
    entity_serializer_creators: HashMap<&'static str, EntitySerializerCreator>,
    entity_serializers: HashMap<String, (Arc<dyn EntityClassSerializer>, bool)>,

    game_event_serializers: HashMap<&'static str, GameEventSerializerFactory>,
    game_event_list: HashMap<i32, Box<dyn GameEventSerializer>>,

    string_tables: Vec<String>,
    instance_baseline: Option<StringTable<BaselineStringTableParser, Box<dyn Any + Send + Sync>>>,

    // for caching
    field_path_cache: Vec<FieldPathFixed>,
    buffer: BytesMut,
}

pub struct CsDemoParserState {
    pub tick: u32,
    pub tick_interval: f32,
    pub map_name: String,
    pub network_protocol: i32,

    pub entities: EntityList,
    user_info: Option<StringTable<BaselineStringTableParser, protobuf::CMsgPlayerInfo>>,
}

impl CsDemoParserState {
    pub fn get_player_info(&self, index: u16) -> Option<&protobuf::CMsgPlayerInfo> {
        let Some(user_info) = &self.user_info else {
            return None;
        };

        let key = index.to_string();
        user_info.get_cached(&key)
    }
}

impl<T: std::io::BufRead + Send + Sync> CsDemoParser<T> {
    pub fn new(reader: T) -> Result<CsDemoParser<T>, std::io::Error> {
        Self::new_pre_registered(reader, HashMap::default(), HashMap::default())
    }

    pub fn new_pre_registered(
        mut reader: T,
        game_event_serializers: HashMap<&'static str, GameEventSerializerFactory>,
        entity_serializer_creators: HashMap<&'static str, EntitySerializerCreator>,
    ) -> Result<CsDemoParser<T>, std::io::Error> {
        /* struct protodemoheader_t
        {
            char demofilestamp[ 8 ]; // PROTODEMO_HEADER_ID
            int32 fileinfo_offset;
        }; */
        // header + offset + 4 bytes padding
        let mut magic = [0u8; 16];
        reader.read_exact(&mut magic)?;
        if &magic[0..8] != b"PBDEMS2\0" {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid CS2 demo magic",
            ));
        };

        Ok(CsDemoParser {
            reader,
            event_manager: EventManager::new(),
            #[cfg(feature = "handle_packet")]
            packet_handler: HashMap::default(),
            state: CsDemoParserState {
                tick: 0,
                tick_interval: 1.0 / 64.0, // default tick interval
                map_name: String::new(),
                network_protocol: 0,
                // most demos seems doesn't exceed 0x400 entities
                entities: EntityList::new(),
                user_info: None,
            },
            class_info: HashMap::new(),
            class_id_size: 0,
            entity_serializer_creators,
            entity_serializers: HashMap::new(),
            game_event_serializers,
            game_event_list: HashMap::new(),
            string_tables: Vec::with_capacity(16),
            instance_baseline: None,
            field_path_cache: Vec::with_capacity(256),
            buffer: BytesMut::with_capacity(BUFFER_SIZE),
        })
    }

    #[cfg(feature = "handle_packet")]
    fn register_packet_handler_internal<M: prost::Message + Default + 'static>(
        &mut self,
        message_type: u64,
    ) {
        if self.packet_handler.contains_key(&message_type) {
            warn!("Packet handler for message type {message_type} already registered, overwriting");
        }

        self.packet_handler.insert(
            message_type,
            Box::new(
                |buf: Bytes, event_manager: &mut EventManager, state: &CsDemoParserState| {
                    let msg = M::decode(buf).map_err(|err| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Failed to decode message: {err:?}"),
                        )
                    })?;

                    event_manager.notify_listeners(crate::event::PacketEvent { packet: msg }, state)
                },
            ),
        );
    }

    #[cfg(feature = "handle_packet")]
    pub fn register_packet_handler<M: prost::Message + Default + 'static>(
        &mut self,
        message_type: u32,
    ) {
        self.register_packet_handler_internal::<M>(message_type as u64);
    }

    #[cfg(feature = "handle_packet")]
    pub fn register_demo_command_handler<M: prost::Message + Default + 'static>(
        &mut self,
        message_type: u32,
    ) {
        self.register_packet_handler_internal::<M>((message_type as u64) | (1 << 32));
    }

    pub fn notify_listeners<E: Event>(&mut self, event: E) -> Result<(), std::io::Error> {
        self.event_manager.notify_listeners(event, &self.state)
    }

    /// checks if the parser is fresh, i.e. has not parsed any frames yet
    /// only fresh parsers can register listeners
    pub fn is_fresh(&self) -> bool {
        // map_name is read from the demo file header frame
        // which is the first frame
        self.state.map_name.is_empty()
    }

    /// claim a buffer with the given size from the pool
    /// or create a new one if capacity is not enough
    #[inline]
    fn alloc_bytes(&mut self, size: usize) -> BytesMut {
        if self.buffer.capacity() < size {
            let mut buf = BytesMut::with_capacity(size);
            unsafe {
                buf.set_len(size);
            }
            buf
        } else {
            unsafe {
                self.buffer.set_len(size);
            }
            self.buffer.split_to(size)
        }
    }

    fn snap_decompress_bytes(&mut self, input: &[u8]) -> Result<Bytes, std::io::Error> {
        let mut buf = self.alloc_bytes(snap::raw::decompress_len(input)?);
        let n = snap::raw::Decoder::new()
            .decompress(input, &mut buf)
            .map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Failed to decompress: {err:?}"),
                )
            })?;
        buf.truncate(n);
        Ok(buf.freeze())
    }

    #[inline(always)]
    fn parse_demo_message<M: prost::Message + Default>(
        &mut self,
        buf: Bytes,
        is_compressed: bool,
    ) -> Result<M, std::io::Error> {
        let buf = if is_compressed {
            self.snap_decompress_bytes(&buf)?
        } else {
            buf
        };

        let msg = M::decode(buf).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to decode message: {err:?}"),
            )
        })?;

        Ok(msg)
    }

    #[cold]
    fn handle_server_info(
        &mut self,
        msg: protobuf::CsvcMsgServerInfo,
    ) -> Result<(), std::io::Error> {
        if let Some(tick_interval) = msg.tick_interval {
            self.state.tick_interval = tick_interval;
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Missing tick interval in server info",
            ));
        }

        if let Some(max_classes) = msg.max_classes {
            self.class_id_size = ((max_classes as f64).log2() as u32) + 1;
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Missing max classes in server info",
            ));
        }

        Ok(())
    }

    #[inline(always)]
    fn read_slice_from_demo_packet(
        &mut self,
        data: &Bytes,
        r: &mut BitReader<Cursor<&Bytes>, bitstream_io::LittleEndian>,
        size: usize,
    ) -> Result<Bytes, std::io::Error> {
        Ok(if r.byte_aligned() {
            // perform zero-copy if possible
            let pos = r.position_in_bits()? as usize >> 3;
            let buf = data.slice(pos..pos + size);
            r.seek_bits(std::io::SeekFrom::Current((size as i64) << 3))?;
            buf
        } else {
            let mut buf = self.alloc_bytes(size);
            r.read_bytes(&mut buf)?;
            buf.freeze()
        })
    }

    fn handle_demo_packet(&mut self, msg: protobuf::CDemoPacket) -> Result<(), std::io::Error> {
        let Some(data) = msg.data else {
            return Ok(());
        };

        let total_bits = (data.len() << 3) as u64;
        let mut r = BitReader::endian(Cursor::new(&data), bitstream_io::LittleEndian);

        while total_bits - r.position_in_bits()? >= 8 {
            let message_type = r.read_ubit_int()?;
            let size = r.read_varint_u64()? as usize;

            #[cfg(feature = "handle_packet")]
            let buf = {
                let buf = self.read_slice_from_demo_packet(&data, &mut r, size)?;

                if let Some(handler) = self.packet_handler.get(&(message_type as u64)) {
                    handler(buf.clone(), &mut self.event_manager, &self.state)?;
                }

                buf
            };

            macro_rules! handle_message {
                ($(($mt:expr, $handler:ident)),*) => {
                    $(
                        if message_type == $mt as u32 {
                            #[cfg(not(feature = "handle_packet"))]
                            let buf = self.read_slice_from_demo_packet(&data, &mut r, size)?;

                            let msg = self.parse_demo_message(buf, false)?;

                            self.$handler(msg)?;

                            continue;
                        }
                    )*
                };
            }

            handle_message!(
                (SvcMessages::SvcPacketEntities, handle_packet_entities),
                (
                    EBaseGameEvents::GeSource1LegacyGameEvent,
                    handle_legacy_game_event
                ),
                (
                    EBaseGameEvents::GeSource1LegacyGameEventList,
                    handle_legacy_game_event_list
                ),
                (
                    SvcMessages::SvcUpdateStringTable,
                    handle_update_string_table
                ),
                (
                    SvcMessages::SvcCreateStringTable,
                    handle_create_string_table
                ),
                (SvcMessages::SvcServerInfo, handle_server_info)
            );

            #[cfg(not(feature = "handle_packet"))]
            r.seek_bits(std::io::SeekFrom::Current((size as i64) << 3))?;
        }

        Ok(())
    }

    // fn handle_demo_full_packet(
    //     &mut self,
    //     msg: protobuf::CDemoFullPacket,
    // ) -> Result<(), std::io::Error> {
    //     if let Some(string_tables) = msg.string_table {
    //         self.handle_demo_string_tables(string_tables)?;
    //     }

    //     if let Some(packet) = msg.packet {
    //         self.handle_demo_packet(packet)?;
    //     }

    //     Ok(())
    // }

    #[cold]
    fn handle_demo_file_header(
        &mut self,
        msg: protobuf::CDemoFileHeader,
    ) -> Result<(), std::io::Error> {
        if let Some(map_name) = msg.map_name {
            self.state.map_name = map_name;
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Missing map name in demo file header",
            ));
        }

        let Some(network_protocol) = msg.network_protocol else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Missing network protocol in demo file header",
            ));
        };

        self.state.network_protocol = network_protocol;

        let event = DemoStartEvent {
            network_protocol,
            map_name: self.state.map_name.clone(),
        };
        self.notify_listeners(event)?;

        Ok(())
    }

    fn read_varint(&mut self) -> Result<u64, std::io::Error> {
        let mut value = 0u64;
        let mut shift = 0u32;

        loop {
            let buf = self.reader.fill_buf()?;
            if buf.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "EOF while reading varint",
                ));
            }

            for (i, &b) in buf.iter().enumerate() {
                value |= ((b & 0x7f) as u64) << shift;

                if (b & 0x80) == 0 {
                    self.reader.consume(i + 1);
                    return Ok(value);
                }

                shift += 7;
            }

            let l = buf.len();
            self.reader.consume(l);
        }
    }

    pub fn read_frame(&mut self) -> Result<bool, std::io::Error> {
        let cmd = self.read_varint()? as i32;
        let tick = self.read_varint()? as u32;
        let size = self.read_varint()? as usize;

        // reclaim the buffer
        if !self.buffer.try_reclaim(BUFFER_SIZE) {
            warn!("Failed to reclaim buffer, performance may degrade");
        }

        let mut buf = self.alloc_bytes(size);
        self.reader.read_exact(&mut buf)?;
        let buf = buf.freeze();

        if tick != self.state.tick {
            self.notify_listeners(TickEvent {
                tick,
                tick_interval: self.state.tick_interval,
            })?;
        }
        self.state.tick = tick;

        let is_compressed = cmd & EDemoCommands::DemIsCompressed as i32 != 0;
        let cmd = cmd & !(EDemoCommands::DemIsCompressed as i32);
        if cmd == EDemoCommands::DemStop as i32 {
            self.notify_listeners(DemoEndEvent)?;
            return Ok(false);
        }

        #[cfg(feature = "handle_packet")]
        let buf = {
            let buf = if is_compressed {
                self.snap_decompress_bytes(&buf)?
            } else {
                buf
            };

            let message_type = (cmd as u64) | (1 << 32);
            if let Some(handler) = self.packet_handler.get(&message_type) {
                handler(buf.clone(), &mut self.event_manager, &self.state)?;
            }

            buf
        };

        macro_rules! handle_command {
            ($(($cmd:expr, $handler:ident)),*) => {
                $(
                    if cmd == $cmd as i32 {
                        #[cfg(not(feature = "handle_packet"))]
                        let msg = self.parse_demo_message(buf, is_compressed)?;

                        // the buf is already decompressed
                        #[cfg(feature = "handle_packet")]
                        let msg = self.parse_demo_message(buf, false)?;

                        self.$handler(msg)?;

                        return Ok(true);
                    }
                )*
            };
        }

        handle_command!(
            (EDemoCommands::DemPacket, handle_demo_packet),
            (EDemoCommands::DemSignonPacket, handle_demo_packet),
            // (EDemoCommands::DemFullPacket, handle_demo_full_packet),
            (EDemoCommands::DemFileHeader, handle_demo_file_header),
            (EDemoCommands::DemSendTables, handle_demo_send_tables),
            (EDemoCommands::DemClassInfo, handle_demo_class_info),
            (EDemoCommands::DemStringTables, handle_demo_string_tables)
        );

        Ok(true)
    }
}
