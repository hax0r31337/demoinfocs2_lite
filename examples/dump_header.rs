use std::{
    io::{BufReader, Cursor},
    sync::Mutex,
};

use bitstream_io::BitReader;
use demoinfocs2_lite::{
    bit::BitReaderExt,
    entity::{decoder::BASIC_ENCODINGS, field::FieldType},
    event::PacketEvent,
    protobuf,
};
use foldhash::{HashMap, HashMapExt, HashSet, HashSetExt};
use log::warn;
use prost::Message;

#[cfg(not(feature = "handle_packet"))]
fn main() {
    panic!(
        "This example requires the 'handle_packet' feature to be enabled. Please run with `--features handle_packet`."
    );
}

static EXTRACT_STATE: Mutex<(bool, bool)> = Mutex::new((false, false));

#[cfg(feature = "handle_packet")]
fn main() -> Result<(), std::io::Error> {
    use demoinfocs2_lite::{
        event::DemoStartEvent,
        protobuf::{EBaseGameEvents, EDemoCommands},
    };

    env_logger::init();

    let file = std::env::args()
        .nth(1)
        .expect("Please provide a demo file path as the first argument");
    let file = std::fs::File::open(file).expect("Failed to open demo file");

    let mut parser =
        demoinfocs2_lite::CsDemoParser::new(BufReader::with_capacity(1024 * 128, file))?;

    parser.register_packet_handler::<protobuf::CMsgSource1LegacyGameEventList>(
        EBaseGameEvents::GeSource1LegacyGameEventList as u32,
    );

    parser.register_demo_command_handler::<protobuf::CDemoSendTables>(
        EDemoCommands::DemSendTables as u32,
    );

    parser.event_manager.register_listener(
        |event: &DemoStartEvent, _state: &demoinfocs2_lite::CsDemoParserState| {
            println!("// GENERATED CODE");
            println!("// Network Protocol: {}", event.network_protocol);
            println!(
                "\nuse demoinfocs2_lite::{{game_event::derive::GameEvent, entity::EntityClass}};"
            );
            println!("use demoinfocs2_lite::entity::serializer::vector::*;\n\n");

            Ok(())
        },
    );

    parser
        .event_manager
        .register_listener(handle_game_event_list);

    parser.event_manager.register_listener(handle_send_tables);

    loop {
        if !parser.read_frame()? {
            break;
        }

        if let Ok(guard) = EXTRACT_STATE.lock() {
            let (extract_game_events, extract_send_tables) = *guard;
            if extract_game_events && extract_send_tables {
                break;
            }
        }
    }

    Ok(())
}

fn handle_game_event_list(
    msg: &PacketEvent<protobuf::CMsgSource1LegacyGameEventList>,
    _state: &demoinfocs2_lite::CsDemoParserState,
) -> Result<(), std::io::Error> {
    for descriptor in &msg.packet.descriptors {
        let Some(event_id) = descriptor.eventid else {
            warn!("Received game event with no event ID, skipping");
            continue;
        };

        let Some(event_name) = descriptor.name.as_ref() else {
            warn!("Received game event with no name, skipping");
            continue;
        };

        println!("/// event_id: {event_id}, event_name: {event_name}");
        println!("#[derive(GameEvent, Default, Debug)]");
        println!("pub struct {}Event {{", to_camel_case(event_name));

        for key in &descriptor.keys {
            let Some(key_name) = key.name.as_ref() else {
                warn!("Received game event key with no name, skipping");
                continue;
            };

            let escaped_key_name = if RUST_KEYWORDS.contains(&key_name.as_str()) {
                &format!("r#{}", key_name)
            } else {
                key_name
            };

            let key_type = match key.r#type.unwrap_or_default() {
                1 => "String",
                2 => "f32",
                3 => "i32",
                4 => "i16",
                5 => "u8",
                6 => "bool",
                7 => "u64",
                8 => "u32",
                9 => "u16",
                _ => "UNKNOWN_TYPE",
            };
            println!("\tpub {escaped_key_name}: {key_type},");
        }

        println!("}}\n");
    }

    let mut guard = EXTRACT_STATE.lock().unwrap();
    guard.0 = true;

    Ok(())
}

fn handle_send_tables(
    msg: &PacketEvent<protobuf::CDemoSendTables>,
    _state: &demoinfocs2_lite::CsDemoParserState,
) -> Result<(), std::io::Error> {
    let Some(data) = msg.packet.data.clone() else {
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
    let mut field_type_cache = HashMap::with_capacity(256);

    let hungarian_notation_regex = regex::Regex::new(r"^[a-z]{0,2}[A-Z]").unwrap();

    for serializer_pb in msg.serializers {
        let Some(serializer_name) = serializer_pb
            .serializer_name_sym
            .and_then(|sym| msg.symbols.get(sym as usize).cloned())
        else {
            return Err(std::io::Error::other(
                "Missing serializer name in serializer",
            ));
        };

        let serializer_class = if let Some(existing) = serializers.get_mut(&serializer_name) {
            existing
        } else {
            let new = SerializerClass {
                fields: Vec::with_capacity(64),
            };
            serializers.insert(serializer_name.clone(), new);

            serializers.get_mut(&serializer_name).unwrap()
        };

        let mut last_idx = 0;

        for field_idx in serializer_pb.fields_index {
            let Some(field_pb) = msg.fields.get(field_idx as usize) else {
                return Err(std::io::Error::other("Missing field in serializer"));
            };

            let Some(var_name_str) = field_pb
                .var_name_sym
                .and_then(|sym| msg.symbols.get(sym as usize))
                .map(|s| s.as_str())
            else {
                return Err(std::io::Error::other("Missing variable name in field"));
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

            let var_name = var_name_str.to_string();
            let (var_type, comment) = if !field_pb.polymorphic_types.is_empty() {
                let polymorphic_serializers = field_pb
                    .polymorphic_types
                    .iter()
                    .filter_map(|pb| pb.polymorphic_field_serializer_name_sym)
                    .map(|sym| {
                        msg.symbols
                            .get(sym as usize)
                            .map(|s| s.as_str())
                            .ok_or_else(|| std::io::Error::other("Missing polymorphic serializer"))
                    })
                    .collect::<Result<Vec<_>, _>>()?
                    .join(", ");

                (
                    "usize".to_string(),
                    Some(format!("polymorphic field: {polymorphic_serializers}")),
                )
            } else if let Some(serializer_name) = field_pb
                .field_serializer_name_sym
                .and_then(|sym| msg.symbols.get(sym as usize))
            {
                (serializer_derivation(serializer_name, field_type), None)
            } else {
                let type_str = get_type_str(field_type)?;

                (serializer_derivation(type_str, field_type), None)
            };

            let var_type = SerializerField {
                var_name,
                type_name: var_type,
                comment,
            };

            if let Some((idx, pos)) = serializer_class
                .fields
                .iter_mut()
                .enumerate()
                .find(|(_, f)| f.var_name == var_name_str)
            {
                *pos = var_type;

                last_idx = idx;
            } else {
                serializer_class.fields.insert(last_idx, var_type);
                last_idx += 1;
            }
        }
    }

    let mut sorted_serializers: Vec<_> = serializers.into_iter().collect();
    sorted_serializers.sort_by(|(a, _), (b, _)| a.cmp(b));

    for (serializer_name, serializer_class) in sorted_serializers {
        println!("#[derive(EntityClass, Clone, Default)]");
        println!("pub struct {serializer_name} {{");

        let mut deduplicate_escaped_name = HashSet::with_capacity(serializer_class.fields.len());

        for field in serializer_class.fields {
            if let Some(comment) = field.comment {
                println!("\t// {comment}");
            }

            println!("\t#[entity(name = \"{}\")]", field.var_name);

            let escaped_var_name = field.var_name;

            // the variable naming scheme of value is quite inconsistent
            // sometimes hungarian notation is used
            // sometimes only m_ prefixes are used
            // sometimes hungarian notations are used without m_ prefixes
            // also for camel casing
            // sometimes `ID` and sometimes `Id`
            // we have to take all of this into account when converting to snake_case

            let escaped_var_name = if let Some(override_var_name) =
                MANUAL_FIELD_OVERRIDES.get(escaped_var_name.as_str())
            {
                override_var_name.to_string()
            } else {
                let mut offset = if escaped_var_name.starts_with("m_") {
                    2
                } else {
                    0
                };

                if let Some(m) = hungarian_notation_regex.find(&escaped_var_name[offset..]) {
                    offset += m.len() - 1;
                }

                let mut escaped_var_name = to_snake_case(&escaped_var_name[offset..]);
                let mut seq = 0;
                loop {
                    let sequenced = if seq == 0 {
                        escaped_var_name.clone()
                    } else {
                        format!("{}_dup{}", escaped_var_name, seq)
                    };

                    if !deduplicate_escaped_name.contains(&sequenced) {
                        deduplicate_escaped_name.insert(sequenced.clone());
                        escaped_var_name = sequenced;
                        break;
                    }

                    seq += 1;
                }

                escaped_var_name
            };

            let escaped_var_name = if RUST_KEYWORDS.contains(&escaped_var_name.as_str()) {
                format!("r#{}", escaped_var_name)
            } else {
                escaped_var_name
            };

            println!("\tpub {}: {},", escaped_var_name, field.type_name);
        }

        println!("}}\n");
    }

    let mut guard = EXTRACT_STATE.lock().unwrap();
    guard.1 = true;

    Ok(())
}

const RUST_KEYWORDS: [&str; 51] = [
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for",
    "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return",
    "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await", "dyn", "abstract", "become", "box", "do", "final", "macro",
    "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
];

static MANUAL_FIELD_OVERRIDES: std::sync::LazyLock<HashMap<&'static str, &'static str>> =
    std::sync::LazyLock::new(|| {
        let mut map = HashMap::new();

        map.insert("m_nCollisionGroup", "n_collision_group");
        map.insert("m_CollisionGroup", "collision_group");
        map.insert("m_iRecoilIndex", "recoil_index_int");
        map.insert("m_flRecoilIndex", "recoil_index_float");

        map
    });

struct SerializerClass {
    fields: Vec<SerializerField>,
}

struct SerializerField {
    var_name: String,
    type_name: String,
    comment: Option<String>,
}

fn serializer_derivation(type_str: &str, field_type: &FieldType) -> String {
    if field_type.is_optional {
        format!("Option<{type_str}>")
    } else if field_type.array_size > 0 && field_type.base_type != "char"
        || field_type.base_type == "CUtlVector"
        || field_type.base_type == "CNetworkUtlVectorBase"
        || field_type.base_type == "CUtlVectorEmbeddedNetworkVar"
    {
        format!("Vec<{type_str}>")
    } else {
        type_str.to_string()
    }
}

fn get_type_str(field_type: &FieldType) -> Result<&'static str, std::io::Error> {
    let var_type = field_type.get_var_type();
    let (net_type, components) = match BASIC_ENCODINGS.get(var_type) {
        Some((net_type, components)) => (*net_type, *components),
        None => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("No serializer found for type: {var_type}"),
            ));
        }
    };

    Ok(match net_type {
        "NET_DATA_TYPE_UINT64" => {
            if components != 1 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Multiple components for UINT64 are not supported",
                ));
            }

            "u64"
        }
        "NET_DATA_TYPE_INT64" => {
            if components != 1 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Multiple components for INT64 are not supported",
                ));
            }

            "i64"
        }
        "NET_DATA_TYPE_FLOAT32" => {
            if var_type == "QAngle" {
                "QAngle"
            } else {
                match components {
                    1 => "f32",
                    2 => "Vector2",
                    3 => "Vector3",
                    4 => "Vector4",
                    6 => "Transform6",
                    _ => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("Unsupported number of components for FLOAT32: {components}"),
                        ));
                    }
                }
            }
        }
        "NET_DATA_TYPE_STRING" => {
            if components != 1 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Multiple components for STRING are not supported",
                ));
            }

            "String"
        }
        "NET_DATA_TYPE_BOOL" => {
            if components != 1 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Multiple components for BOOL are not supported",
                ));
            }

            "bool"
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unsupported net type: {net_type} ({var_type})"),
            ));
        }
    })
}

fn to_camel_case(snake: &str) -> String {
    let mut result = String::with_capacity(snake.len());
    let mut capitalize_next = true;

    for c in snake.chars() {
        if c == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(c.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(c.to_ascii_lowercase());
        }
    }

    result
}

fn to_snake_case(camel: &str) -> String {
    let mut result = String::with_capacity(camel.len() + 8);
    let mut prev_was_upper = true;

    for c in camel.chars() {
        if c == '_' {
            result.push('_');
            prev_was_upper = true;
        } else if c.is_uppercase() {
            if !prev_was_upper {
                result.push('_');
            }
            result.push(c.to_ascii_lowercase());
            prev_was_upper = true;
        } else {
            result.push(c);
            prev_was_upper = false;
        }
    }

    result
}
