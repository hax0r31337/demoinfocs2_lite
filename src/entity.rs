pub mod decoder;
pub mod derive;
pub mod field;
pub mod fieldpath;
pub mod list;
pub mod serializer;

use std::{any::Any, io::Cursor, sync::Arc};

use bitstream_io::{BitRead, BitReader};
use foldhash::{HashMap, HashMapExt};
use log::{error, warn};
use prost::Message;

use crate::{
    CsDemoParser,
    bit::BitReaderExt,
    entity::{
        decoder::get_serializer,
        field::FieldType,
        fieldpath::read_field_paths,
        list::EntityItem,
        serializer::{
            EntityClassSerializer, EntitySerializer, PolymorphicSerializer, UnknownEntity,
            UnknownEntitySerializer,
        },
    },
    protobuf::{self},
};

pub type EntitySerializerCreator =
    fn(serializers: Vec<(&str, Arc<dyn EntitySerializer>)>) -> Arc<dyn EntityClassSerializer>;

type Reader<'a> = BitReader<Cursor<&'a [u8]>, bitstream_io::LittleEndian>;

impl<T: std::io::BufRead + Send + Sync> CsDemoParser<T> {
    pub fn register_entity_serializer(
        &mut self,
        name: &'static str,
        creator: EntitySerializerCreator,
    ) {
        if !self.is_fresh() {
            warn!("Cannot register entity serializer after parsing has started");
            return;
        }

        self.entity_serializer_creators.insert(name, creator);
    }

    #[cold]
    pub(super) fn handle_demo_class_info(
        &mut self,
        msg: protobuf::CDemoClassInfo,
    ) -> Result<(), std::io::Error> {
        self.class_info = msg
            .classes
            .into_iter()
            .filter_map(|class| {
                if let (Some(class_id), Some(class_name)) = (class.class_id, class.network_name) {
                    Some((class_id as u32, class_name))
                } else {
                    error!("Missing class ID or name in class info");
                    None
                }
            })
            .collect::<HashMap<_, _>>();

        Ok(())
    }

    #[cold]
    pub(super) fn handle_demo_send_tables(
        &mut self,
        msg: protobuf::CDemoSendTables,
    ) -> Result<(), std::io::Error> {
        if !self.entity_serializers.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Demo send tables already processed",
            ));
        }

        if let Some(instance_baseline) = self.instance_baseline.as_mut() {
            instance_baseline.purge_cache();
        }

        let Some(data) = msg.data else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Missing data in demo send tables",
            ));
        };

        let offset = {
            let mut r = BitReader::endian(Cursor::new(data.as_ref()), bitstream_io::LittleEndian);
            r.read_varint_u32()?;

            r.position_in_bits()? as usize >> 3
        };

        let msg =
            protobuf::CsvcMsgFlattenedSerializer::decode(data.slice(offset..)).map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Failed to decode flattened serializer: {err:?}"),
                )
            })?;

        let mut serializers = HashMap::with_capacity(msg.serializers.len());
        let mut fields_cache: Vec<Option<(&str, Arc<dyn EntitySerializer>)>> =
            vec![None; msg.fields.len()];
        let mut field_type_cache = HashMap::with_capacity(256);

        for serializer_pb in msg.serializers {
            let Some(serializer_name) = serializer_pb
                .serializer_name_sym
                .and_then(|sym| msg.symbols.get(sym as usize).cloned())
            else {
                return Err(std::io::Error::other(
                    "Missing serializer name in serializer",
                ));
            };

            let mut serializer_fields = Vec::with_capacity(serializer_pb.fields_index.len());

            for field_idx in serializer_pb.fields_index {
                if let Some(field_serializer) = fields_cache
                    .get(field_idx as usize)
                    .and_then(|s| s.as_ref())
                {
                    serializer_fields.push(field_serializer.clone());
                    continue;
                }

                let Some(field_pb) = msg.fields.get(field_idx as usize) else {
                    return Err(std::io::Error::other("Missing field in serializer"));
                };

                let Some(var_type) = field_pb
                    .var_type_sym
                    .and_then(|sym| msg.symbols.get(sym as usize))
                else {
                    return Err(std::io::Error::other("Missing variable type in field"));
                };

                let var_type = var_type.as_str();
                let field_type = if let Some(field_type) = field_type_cache.get(var_type) {
                    field_type
                } else {
                    let field_type = FieldType::new(var_type)?;
                    field_type_cache.insert(var_type, field_type);

                    field_type_cache
                        .get(var_type)
                        .expect("Field type should be cached")
                };

                let Some(var_name) = field_pb
                    .var_name_sym
                    .and_then(|sym| msg.symbols.get(sym as usize))
                    .map(|s| s.as_str())
                else {
                    return Err(std::io::Error::other("Missing variable name in field"));
                };

                let encoder = field_pb
                    .var_encoder_sym
                    .and_then(|sym| msg.symbols.get(sym as usize))
                    .map(|s| s.as_str());

                let serializer = if !field_pb.polymorphic_types.is_empty() {
                    let polymorphic_serializers = field_pb
                        .polymorphic_types
                        .iter()
                        .filter_map(|pb| pb.polymorphic_field_serializer_name_sym)
                        .map(|sym| {
                            msg.symbols
                                .get(sym as usize)
                                .cloned()
                                .ok_or_else(|| {
                                    std::io::Error::other("Missing polymorphic serializer")
                                })
                                .and_then(|s| {
                                    serializers
                                        .get(s.as_str())
                                        .cloned()
                                        .map(|(s, _)| s)
                                        .ok_or_else(|| {
                                            std::io::Error::other("Missing polymorphic serializer")
                                        })
                                })
                                .map(|s: Arc<dyn EntityClassSerializer>| {
                                    s.serializer_derivation(field_type)
                                })
                        })
                        .collect::<Result<Box<[_]>, _>>()?;

                    Arc::new(PolymorphicSerializer::new(polymorphic_serializers))
                } else if let Some(serializer_name) = field_pb
                    .field_serializer_name_sym
                    .and_then(|sym| msg.symbols.get(sym as usize))
                {
                    let Some((super_serializer, _)): Option<(
                        Arc<dyn EntityClassSerializer>,
                        bool,
                    )> = serializers.get(serializer_name).cloned() else {
                        return Err(std::io::Error::other("Missing serializer for field"));
                    };

                    super_serializer.serializer_derivation(field_type)
                } else {
                    get_serializer(field_type, var_name, encoder, field_pb)?
                };

                fields_cache[field_idx as usize] = Some((var_name, serializer.clone()));

                serializer_fields.push((var_name, serializer))
            }

            let (serializer_creator, serialize_baseline) = if let Some(&serializer_creator) = self
                .entity_serializer_creators
                .get(serializer_name.as_str())
            {
                (serializer_creator, true)
            } else {
                (
                    UnknownEntitySerializer::new_serializer as EntitySerializerCreator,
                    false,
                )
            };

            serializers.insert(
                serializer_name.clone(),
                (serializer_creator(serializer_fields), serialize_baseline),
            );
        }

        self.entity_serializers = serializers;

        Ok(())
    }

    pub(super) fn handle_packet_entities(
        &mut self,
        msg: protobuf::CsvcMsgPacketEntities,
    ) -> Result<(), std::io::Error> {
        let (Some(data), Some(entries)) = (msg.entity_data, msg.updated_entries) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Missing data or number of entries in packet entities",
            ));
        };

        let has_pvs_vis_bits = msg.has_pvs_vis_bits_deprecated.unwrap_or(0) > 0;

        // let len = (data.len() << 3) as u64;
        let mut r = BitReader::endian(Cursor::new(data.as_ref()), bitstream_io::LittleEndian);
        let mut idx: i32 = -1;

        for entry in 0..entries {
            idx += r.read_ubit_int()? as i32 + 1;
            let cmd = r.read_unsigned::<2, u8>()?;

            if cmd & 1 == 0 {
                if cmd & 2 != 0 {
                    // create entity
                    let class_id: u32 = r.read_var(self.class_id_size)?;
                    let serial = r.read_unsigned::<17, u32>()?;
                    let _unk_0 = r.read_varint_u64()?;

                    let Some(class_name) = self.class_info.get(&class_id).map(|s| s.as_str())
                    else {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Unknown class id: {class_id}"),
                        ));
                    };

                    let Some((serializer, serialize_baseline)) =
                        self.entity_serializers.get(class_name)
                    else {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Unknown serializer: {class_name}"),
                        ));
                    };

                    let serializer = serializer.clone();

                    let entity = EntityItem {
                        index: idx as u32,
                        serial,
                        item: if *serialize_baseline {
                            self.parse_entity_from_baseline(class_id, serializer.as_ref())?
                        } else {
                            serializer.new_entity()
                        },
                        serializer,
                    };

                    self.state.entities.insert(idx as usize, entity);
                } else if has_pvs_vis_bits && r.read_unsigned::<2, u8>()? & 1 != 0 {
                    continue;
                }

                let Some(entity) = self.state.entities.get_mut(idx as usize) else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Entity at index {idx} not found for update"),
                    ));
                };

                if entry == entries - 1 && entity.item.is::<UnknownEntity>() {
                    // if the last entity is an unknown entity, we can skip reading the fields
                    continue;
                }

                read_field_paths(&mut r, &mut self.field_path_cache)?;

                for field_path in &self.field_path_cache {
                    entity.serializer.decode(
                        Some(entity.item.as_mut()),
                        field_path.to_slice(),
                        &mut r,
                    )?;
                }

                self.field_path_cache.clear();
            } else if self.state.entities.delete(idx as usize).is_none() {
                error!("Entity at index {idx} not found for deletion");
            }
        }

        // let bits = len - r.position_in_bits()?;
        // if bits >= 8 {
        //     error!("packet entities did not consume all data: {bits}");
        // }

        Ok(())
    }

    fn parse_entity_from_baseline(
        &mut self,
        class_id: u32,
        serializer: &dyn EntityClassSerializer,
    ) -> Result<Box<dyn Any + Send + Sync>, std::io::Error> {
        // try to get entity from cache
        let Some(instance_baseline) = self.instance_baseline.as_mut() else {
            return Ok(serializer.new_entity());
        };

        let baseline_key = class_id.to_string();

        if let Some(entity) = instance_baseline.get_cached(&baseline_key) {
            return serializer.clone_entity(entity.as_ref());
        }

        // if not found, create a new entity
        let mut entity = serializer.new_entity();
        let baseline = instance_baseline.get_raw(&baseline_key);

        if let Some(baseline) = baseline {
            let len = (baseline.len() << 3) as u64;
            let mut baseline_reader =
                BitReader::endian(Cursor::new(baseline), bitstream_io::LittleEndian);

            read_field_paths(&mut baseline_reader, &mut self.field_path_cache)?;

            for field_path in &self.field_path_cache {
                serializer.decode(
                    Some(entity.as_mut()),
                    field_path.to_slice(),
                    &mut baseline_reader,
                )?;
            }

            self.field_path_cache.clear();

            // print bytes left
            let bits = len - baseline_reader.position_in_bits()?;
            if bits >= 8 {
                error!("Baseline did not consume all data: {bits} {len}");
            }
        } else {
            return Ok(entity);
        }

        // cache the entity for future use
        let entity_clone = serializer.clone_entity(entity.as_ref())?;
        instance_baseline.put_cache(baseline_key, entity_clone);

        Ok(entity)
    }
}
