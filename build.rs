use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

use kv3::Value;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();

    prost_build::Config::new()
        .bytes(["."])
        .default_package_filename("game_messages")
        .compile_protos(
            &[
                "GameTracking-CS2/Protobufs/gameevents.proto",
                // "GameTracking-CS2/Protobufs/usermessages.proto",
                "GameTracking-CS2/Protobufs/netmessages.proto",
                "GameTracking-CS2/Protobufs/demo.proto",
            ],
            &["GameTracking-CS2/Protobufs/"],
        )?;

    // generate demo info table
    let demo_info_path = Path::new(&manifest_dir).join("demoinfo2.txt");
    if !demo_info_path.exists() {
        panic!("Demoinfo file not found: {}", demo_info_path.display());
    }

    println!("cargo:rerun-if-changed={}", demo_info_path.display());
    let path = Path::new(&std::env::var("OUT_DIR")?).join("demoinfo2.rs");
    let mut file = BufWriter::new(File::create(&path)?);

    let demo_info_content = std::fs::read_to_string(demo_info_path)?;
    let demo_info = kv3::from_str(&demo_info_content)?;
    let Value::File(_, demo_info) = demo_info else {
        panic!("Missing demoinfo2 in demoinfo2.txt");
    };

    let Some(Value::Array(basic_encodings)) = demo_info.get("m_BasicEncodings") else {
        panic!("Missing m_BasicEncodings in demoinfo2.txt");
    };

    let Some(Value::Array(field_encoder_overrides)) = demo_info.get("m_FieldEncoderOverrides")
    else {
        panic!("Missing m_FieldEncoderOverrides in demoinfo2.txt");
    };

    let Some(Value::Array(aliases)) = demo_info.get("m_Aliases") else {
        panic!("Missing m_Aliases in demoinfo2.txt");
    };

    let mut encodings_map = &mut phf_codegen::Map::new();
    for encoding in basic_encodings {
        let Some(Value::String(name)) = encoding.get("m_Name") else {
            panic!("Missing m_Name in basic encoding");
        };
        let Some(Value::String(var_type)) = encoding.get("m_VarType") else {
            panic!("Missing m_VarType in basic encoding");
        };
        let components = match encoding.get("m_nComponents") {
            Some(Value::String(s)) => s,
            _ => "1",
        };

        encodings_map = encodings_map.entry(name, format!("(\"{var_type}\", {components})",));
    }

    writeln!(
        &mut file,
        "pub static BASIC_ENCODINGS: phf::Map<&'static str, (&'static str, usize)> = {};",
        encodings_map.build()
    )?;

    let mut field_overrides_map = &mut phf_codegen::Map::new();
    for override_ in field_encoder_overrides {
        let Some(Value::String(name)) = override_.get("m_Name") else {
            panic!("Missing m_Name in field encoder override");
        };
        let Some(Value::String(var_type)) = override_.get("m_VarType") else {
            panic!("Missing m_VarType in field encoder override");
        };

        field_overrides_map = field_overrides_map.entry(name, format!("\"{var_type}\""));
    }

    writeln!(
        &mut file,
        "pub static FIELD_ENCODER_OVERRIDES: phf::Map<&'static str, &'static str> = {};",
        field_overrides_map.build()
    )?;

    let mut aliases_map = &mut phf_codegen::Map::new();
    for alias in aliases {
        let Some(Value::String(type_alias)) = alias.get("m_TypeAlias") else {
            panic!("Missing m_TypeAlias in alias");
        };
        let Some(Value::String(underlying_type)) = alias.get("m_UnderlyingType") else {
            panic!("Missing m_UnderlyingType in alias");
        };

        aliases_map = aliases_map.entry(type_alias, format!("\"{underlying_type}\""));
    }

    writeln!(
        &mut file,
        "pub static ALIASES: phf::Map<&'static str, &'static str> = {};",
        aliases_map.build()
    )?;

    file.flush()?;

    Ok(())
}

trait ObjectKeyExt {
    fn as_str(&self) -> &str;
}

impl ObjectKeyExt for kv3::ObjectKey {
    fn as_str(&self) -> &str {
        match self {
            kv3::ObjectKey::String(s) => s.as_str(),
            kv3::ObjectKey::Identifier(s) => s.as_str(),
        }
    }
}

trait ValueExt {
    fn get(&self, key: &str) -> Option<&Value>;
}

impl ValueExt for Value {
    fn get(&self, key: &str) -> Option<&Value> {
        if let Value::Object(obj) = self {
            for (k, v) in obj {
                if k.as_str() == key {
                    return Some(v);
                }
            }
        }

        None
    }
}
