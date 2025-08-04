use std::sync::Arc;

use bitstream_io::BitRead;

use crate::{
    bit::BitReaderExt,
    entity::{
        Reader,
        field::FieldType,
        serializer::{
            ArraySerializer, EntityField, EntitySerializer, EntitySerializerMultiComponents,
            EntitySerializerTypeWarpAdapter, EntitySerializerTyped, OptionalSerializer,
            TypedEntitySerializerAdapter, VectorSerializer,
            vector::{QAngle, Transform6, Vector2, Vector3, Vector4},
        },
    },
    protobuf,
};

include!(concat!(env!("OUT_DIR"), "/demoinfo2.rs"));

const LIST_GENERIC_TYPE: [&str; 3] = [
    "CNetworkUtlVectorBase",
    "CUtlVectorEmbeddedNetworkVar",
    "CUtlVector",
];

pub fn get_serializer(
    field_type: &FieldType,
    var_name: &str,
    encoder: Option<&str>,
    field_pb: &protobuf::ProtoFlattenedSerializerFieldT,
) -> Result<Arc<dyn EntitySerializer>, std::io::Error> {
    let var_type = if let Some(generic_type) = &field_type.generic_type {
        if LIST_GENERIC_TYPE.contains(&field_type.base_type.as_str()) {
            &generic_type.base_type
        } else {
            &field_type.base_type
        }
    } else {
        &field_type.base_type
    };
    let (mut net_type, components) = match BASIC_ENCODINGS.get(var_type) {
        Some((net_type, components)) => (*net_type, *components),
        None => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("No serializer found for type: {var_type}"),
            ));
        }
    };

    let mut field_override = None;
    if let Some(ty) = FIELD_ENCODER_OVERRIDES.get(var_name) {
        field_override = Some(net_type);
        net_type = ty;
    }

    let serializer: Arc<dyn EntitySerializer> = match net_type {
        "NET_DATA_TYPE_UINT64" => {
            if components != 1 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Multiple components for UINT64 are not supported",
                ));
            }

            macro_rules! type_warp {
                ($serializer:expr) => {{
                    let serializer: Arc<dyn EntitySerializer> = match field_override {
                        Some("NET_DATA_TYPE_FLOAT32") => serializer_derivation(
                            EntitySerializerTypeWarpAdapter::<_, u64, f32>::new($serializer),
                            field_type,
                        ),
                        None => serializer_derivation($serializer, field_type),
                        Some(ty) => {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!("Unsupported field type warp for UINT64: {ty}"),
                            ));
                        }
                    };

                    serializer
                }};
            }

            match encoder {
                Some("fixed64") => type_warp!(U64SerializerFixed),
                None => type_warp!(U64SerializerVarInt),
                Some(t) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Unsupported encoder for UINT64: {t}"),
                    ));
                }
            }
        }
        "NET_DATA_TYPE_INT64" => {
            if components != 1 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Multiple components for INT64 are not supported",
                ));
            } else if let Some(ty) = field_override {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Field type warp is not supported for INT64: {ty}"),
                ));
            }

            match encoder {
                None => serializer_derivation(I64SerializerVarInt, field_type),
                Some(t) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Unsupported encoder for INT64: {t}"),
                    ));
                }
            }
        }
        "NET_DATA_TYPE_FLOAT32" => {
            if encoder == Some("normal") && var_type == "Vector" && components == 3 {
                return Ok(serializer_derivation(
                    Vector3SerializerNormalized,
                    field_type,
                ));
            } else if let Some(ty) = field_override {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Field type warp is not supported for INT64: {ty}"),
                ));
            }

            let bit_count = field_pb.bit_count.unwrap_or_default();

            if var_type == "QAngle" {
                if components != 3 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "QAngle must have 3 components",
                    ));
                }

                match encoder {
                    Some("qangle_precise") => {
                        return Ok(serializer_derivation(QAngleSerializerPrecise, field_type));
                    }
                    Some("qangle") if bit_count != 0 => {
                        return Ok(serializer_derivation(
                            QAngleSerializerBit::new(bit_count as u32),
                            field_type,
                        ));
                    }
                    Some("qangle") if bit_count == 0 => {
                        return Ok(serializer_derivation(QAngleSerializerCoord, field_type));
                    }
                    _ => {}
                }
            }

            macro_rules! with_components {
                ($serializer:expr) => {{
                    let v: Arc<dyn EntitySerializer> = match components {
                        1 => serializer_derivation($serializer, field_type),
                        2 => serializer_derivation(
                            EntitySerializerMultiComponents::<_, Vector2, _, 2>::new($serializer),
                            field_type,
                        ),
                        3 => serializer_derivation(
                            EntitySerializerMultiComponents::<_, Vector3, _, 3>::new($serializer),
                            field_type,
                        ),
                        4 => serializer_derivation(
                            EntitySerializerMultiComponents::<_, Vector4, _, 4>::new($serializer),
                            field_type,
                        ),
                        6 => serializer_derivation(
                            EntitySerializerMultiComponents::<_, Transform6, _, 6>::new(
                                $serializer,
                            ),
                            field_type,
                        ),
                        _ => {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!(
                                    "Unsupported number of components for FLOAT32: {components}"
                                ),
                            ));
                        }
                    };

                    v
                }};
            }

            match encoder {
                Some("coord") => with_components!(F32SerializerCoord),
                None => {
                    if bit_count <= 0 || bit_count >= 32 {
                        with_components!(F32SerializerNoScale)
                    } else {
                        let bits = bit_count as u32;
                        let flags = field_pb.encode_flags.unwrap_or_default() as u32;
                        let low = field_pb.low_value.unwrap_or_default();
                        let high = field_pb.high_value.unwrap_or_default();

                        let serializer = F32SerializerQuantized::new(bits, flags, low, high)?;
                        with_components!(serializer)
                    }
                }
                Some(t) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Unsupported encoder for FLOAT32: {t}"),
                    ));
                }
            }
        }
        "NET_DATA_TYPE_STRING" => {
            if components != 1 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Multiple components for STRING are not supported",
                ));
            } else if let Some(ty) = field_override {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Field type warp is not supported for INT64: {ty}"),
                ));
            }

            match encoder {
                None => serializer_derivation(StringSerializer, field_type),
                Some(t) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Unsupported encoder for STRING: {t}"),
                    ));
                }
            }
        }
        "NET_DATA_TYPE_BOOL" => {
            if components != 1 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Multiple components for BOOL are not supported",
                ));
            } else if let Some(ty) = field_override {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Field type warp is not supported for INT64: {ty}"),
                ));
            }

            match encoder {
                None => serializer_derivation(BoolSerializer, field_type),
                Some(t) => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Unsupported encoder for BOOL: {t}"),
                    ));
                }
            }
        }
        _ => {
            // NET_DATA_TYPE_FLOAT64 is not supported as it is not used currently
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Unsupported net type: {net_type} ({var_type}), with type warp: {field_override:?}, encoder: {encoder:?}"
                ),
            ));
        }
    };

    Ok(serializer)
}

pub fn serializer_derivation<S, T>(
    serializer: S,
    field_type: &FieldType,
) -> Arc<dyn EntitySerializer>
where
    S: EntitySerializerTyped<T> + 'static,
    T: EntityField,
{
    if field_type.is_optional {
        Arc::new(TypedEntitySerializerAdapter::new(OptionalSerializer::new(
            serializer,
        )))
    } else if field_type.array_size > 0 && field_type.base_type != "char" {
        Arc::new(TypedEntitySerializerAdapter::new(ArraySerializer::new(
            serializer,
            field_type.array_size,
        )))
    } else if field_type.base_type == "CUtlVector"
        || field_type.base_type == "CNetworkUtlVectorBase"
        || field_type.base_type == "CUtlVectorEmbeddedNetworkVar"
    {
        Arc::new(TypedEntitySerializerAdapter::new(VectorSerializer::new(
            serializer,
        )))
    } else {
        Arc::new(TypedEntitySerializerAdapter::new(serializer))
    }
}

macro_rules! primitive_serializer {
    ($name:ident, $type:ty, $read_fn:expr, $skip_fn:expr) => {
        pub struct $name;

        impl EntitySerializerTyped<$type> for $name {
            #[inline(always)]
            fn decode_typed(
                &self,
                entity: Option<&mut $type>,
                path: &[u32],
                reader: &mut Reader<'_>,
            ) -> Result<(), std::io::Error> {
                if !path.is_empty() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Path should be empty when primitive serializer is reached",
                    ));
                }

                if let Some(e) = entity {
                    let v: Result<$type, std::io::Error> = $read_fn(reader);
                    let v = v?;
                    *e = v;
                } else {
                    let v: Result<(), std::io::Error> = $skip_fn(reader);
                    v?;
                }

                Ok(())
            }
        }
    };
}

pub fn skip_varint(reader: &mut Reader<'_>) -> Result<(), std::io::Error> {
    loop {
        let byte = reader.read_u8()?;
        if byte & 0x80 == 0 {
            break;
        }
    }
    Ok(())
}

primitive_serializer!(
    BoolSerializer,
    bool,
    |reader: &mut Reader<'_>| { reader.read_bit() },
    |reader: &mut Reader<'_>| {
        reader.read_bit()?;
        Ok(())
    }
);

primitive_serializer!(
    U64SerializerVarInt,
    u64,
    |reader: &mut Reader<'_>| { reader.read_varint_u64() },
    skip_varint
);

primitive_serializer!(
    U64SerializerFixed,
    u64,
    |reader: &mut Reader<'_>| {
        if reader.byte_aligned() {
            let mut buf = [0u8; 8];
            reader.read_bytes(&mut buf)?;
            Ok(u64::from_le_bytes(buf))
        } else {
            reader.read_unsigned::<64, u64>()
        }
    },
    |reader: &mut Reader<'_>| {
        if reader.byte_aligned() {
            let mut buf = [0u8; 8];
            reader.read_bytes(&mut buf)?;
        } else {
            reader.skip(64)?;
        }
        Ok(())
    }
);

primitive_serializer!(
    I64SerializerVarInt,
    i64,
    |reader: &mut Reader<'_>| { reader.read_varint_i64() },
    skip_varint
);

primitive_serializer!(
    StringSerializer,
    String,
    |reader: &mut Reader<'_>| { reader.read_null_terminated_string() },
    |reader: &mut Reader<'_>| {
        loop {
            let byte = reader.read_u8()?;
            if byte == 0 {
                break;
            }
        }

        Ok(())
    }
);

#[inline(always)]
fn read_coord(reader: &mut Reader<'_>) -> Result<f32, std::io::Error> {
    let mut value = 0.0;

    let mut intval = reader.read_bit()? as u32;
    let mut fractval = reader.read_bit()? as u32;

    if intval != 0 || fractval != 0 {
        let signbit = reader.read_bit()?;

        if intval != 0 {
            intval = reader.read_unsigned::<14, u32>()? + 1;
        }

        if fractval != 0 {
            fractval = reader.read_unsigned::<5, u32>()?;
        }

        value = intval as f32 + fractval as f32 * (1.0 / ((1 << 5) as f32));

        if signbit {
            value = -value;
        }
    }

    Ok(value)
}

#[inline(always)]
fn skip_coord(reader: &mut Reader<'_>) -> Result<(), std::io::Error> {
    let intval = reader.read_bit()?;
    let fractval = reader.read_bit()?;

    if intval && fractval {
        reader.skip(20)?;
    } else if intval {
        reader.skip(15)?;
    } else if fractval {
        reader.skip(6)?;
    }

    Ok(())
}

primitive_serializer!(F32SerializerCoord, f32, read_coord, skip_coord);

primitive_serializer!(
    F32SerializerNoScale,
    f32,
    |reader: &mut Reader<'_>| {
        let v = if reader.byte_aligned() {
            let mut buf = [0u8; 4];
            reader.read_bytes(&mut buf)?;
            u32::from_le_bytes(buf)
        } else {
            reader.read_unsigned::<32, u32>()?
        };
        Ok(f32::from_bits(v))
    },
    |reader: &mut Reader<'_>| {
        if reader.byte_aligned() {
            let mut buf = [0u8; 4];
            reader.read_bytes(&mut buf)?;
        } else {
            reader.skip(32)?;
        }
        Ok(())
    }
);

primitive_serializer!(
    Vector3SerializerNormalized,
    Vector3,
    |reader: &mut Reader<'_>| {
        let read = |reader: &mut Reader<'_>| -> Result<f32, std::io::Error> {
            let sign = reader.read_bit()?;
            let len = reader.read_unsigned::<11, u32>()?;

            let value = (len as f32) * (1.0 / ((1 << 11) as f32 - 1.0));

            Ok(if sign { -value } else { value })
        };

        let mut vector = Vector3::default();

        let has_x = reader.read_bit()?;
        let has_y = reader.read_bit()?;

        if has_x {
            vector.x = read(reader)?;
        }
        if has_y {
            vector.y = read(reader)?;
        }

        let sign_z = reader.read_bit()?;
        let prod_z = vector.x.powi(2) + vector.y.powi(2);
        if prod_z < 1.0 {
            vector.z = (1.0 - prod_z).sqrt();

            if sign_z {
                vector.z = -vector.z;
            }
        }

        Ok(vector)
    },
    |reader: &mut Reader<'_>| {
        let has_x = reader.read_bit()?;
        let has_y = reader.read_bit()?;

        if has_x && has_y {
            reader.skip(25)?;
        } else if has_x || has_y {
            reader.skip(13)?;
        } else {
            reader.read_bit()?;
        }
        Ok(())
    }
);

primitive_serializer!(
    QAngleSerializerCoord,
    QAngle,
    |reader: &mut Reader<'_>| {
        let mut angle = QAngle::default();

        let has_x = reader.read_bit()?;
        let has_y = reader.read_bit()?;
        let has_z = reader.read_bit()?;

        if has_x {
            angle.pitch = read_coord(reader)?;
        }
        if has_y {
            angle.yaw = read_coord(reader)?;
        }
        if has_z {
            angle.roll = read_coord(reader)?;
        }

        Ok(angle)
    },
    |reader: &mut Reader<'_>| {
        let has_x = reader.read_bit()?;
        let has_y = reader.read_bit()?;
        let has_z = reader.read_bit()?;

        if has_x {
            skip_coord(reader)?;
        }
        if has_y {
            skip_coord(reader)?;
        }
        if has_z {
            skip_coord(reader)?;
        }

        Ok(())
    }
);

primitive_serializer!(
    QAngleSerializerPrecise,
    QAngle,
    |reader: &mut Reader<'_>| {
        let read = |reader: &mut Reader<'_>| -> Result<f32, std::io::Error> {
            const BITS: u32 = 20;
            let v = reader.read_unsigned::<BITS, u32>()? as f32;
            Ok((v * 360.0 / ((1 << BITS) as f32)) - 180.0)
        };

        let mut angle = QAngle::default();

        let has_x = reader.read_bit()?;
        let has_y = reader.read_bit()?;
        let has_z = reader.read_bit()?;

        if has_x {
            angle.pitch = read(reader)?;
        }
        if has_y {
            angle.yaw = read(reader)?;
        }
        if has_z {
            angle.roll = read(reader)?;
        }

        Ok(angle)
    },
    |reader: &mut Reader<'_>| {
        let has_x = reader.read_bit()?;
        let has_y = reader.read_bit()?;
        let has_z = reader.read_bit()?;

        if has_x {
            reader.skip(20)?;
        }
        if has_y {
            reader.skip(20)?;
        }
        if has_z {
            reader.skip(20)?;
        }

        Ok(())
    }
);

pub struct QAngleSerializerBit {
    bits: u32,
}

impl QAngleSerializerBit {
    pub fn new(bits: u32) -> Self {
        Self { bits }
    }

    fn read_angle(&self, reader: &mut Reader<'_>) -> Result<f32, std::io::Error> {
        let v = reader.read_var::<u32>(self.bits)? as f32;
        Ok(v * 360.0 / ((1 << self.bits) as f32))
    }
}

impl EntitySerializerTyped<QAngle> for QAngleSerializerBit {
    #[inline(always)]
    fn decode_typed(
        &self,
        entity: Option<&mut QAngle>,
        _path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error> {
        if let Some(e) = entity {
            let x = self.read_angle(reader)?;
            let y = self.read_angle(reader)?;
            let z = self.read_angle(reader)?;

            *e = QAngle {
                pitch: x,
                yaw: y,
                roll: z,
            };
        } else {
            reader.skip(self.bits * 3)?;
        }

        Ok(())
    }
}

pub struct F32SerializerQuantized {
    low: f32,
    high: f32,
    high_low_mul: f32,
    dec_mul: f32,
    bits: u32,
    rounddown: bool,
    roundup: bool,
    encode_zero: bool,
}

const QFF_ROUNDDOWN: u32 = 1 << 0;
const QFF_ROUNDUP: u32 = 1 << 1;
const QFF_ENCODE_ZERO: u32 = 1 << 2;
const QFF_ENCODE_INTEGERS: u32 = 1 << 3;

impl F32SerializerQuantized {
    pub fn new(
        mut bits: u32,
        mut flags: u32,
        mut low: f32,
        mut high: f32,
    ) -> Result<Self, std::io::Error> {
        if flags != 0 {
            // discard zero flag when encoding min / max set to 0
            if (low == 0.0 && (flags & QFF_ROUNDDOWN) != 0)
                || (high == 0.0 && (flags & QFF_ROUNDUP) != 0)
            {
                flags &= !QFF_ENCODE_ZERO;
            }

            // ff min / max is zero when encoding zero, switch to round up / round down instead
            if low == 0.0 && (flags & QFF_ENCODE_ZERO) != 0 {
                flags |= QFF_ROUNDDOWN;
                flags &= !QFF_ENCODE_ZERO;
            }

            if high == 0.0 && (flags & QFF_ENCODE_ZERO) != 0 {
                flags |= QFF_ROUNDUP;
                flags &= !QFF_ENCODE_ZERO;
            }

            // check if the range spans zero
            if low > 0.0 || high < 0.0 {
                flags &= !QFF_ENCODE_ZERO;
            }

            // if we are left with encode zero, only leave integer flag
            if (flags & QFF_ENCODE_INTEGERS) != 0 {
                flags &= !(QFF_ROUNDUP | QFF_ROUNDDOWN | QFF_ENCODE_ZERO);
            }

            // verify that we don't have roundup / rounddown set
            if flags & (QFF_ROUNDDOWN | QFF_ROUNDUP) == (QFF_ROUNDDOWN | QFF_ROUNDUP) {
                return Err(std::io::Error::other(
                    "Roundup / Rounddown are mutually exclusive",
                ));
            }
        }

        let mut steps = 1u32 << bits;
        let mut offset;
        if flags & QFF_ROUNDDOWN != 0 {
            offset = (high - low) / steps as f32;
            high -= offset;
        } else if flags & QFF_ROUNDUP != 0 {
            offset = (high - low) / steps as f32;
            low += offset;
        }

        if flags & QFF_ENCODE_INTEGERS != 0 {
            let mut delta = high - low;
            if delta < 1.0 {
                delta = 1.0;
            }

            let delta_log2 = delta.log2().ceil();
            let range2 = 1u32 << delta_log2 as u32;
            let mut bc = bits;

            while (1u32 << bc) <= range2 {
                bc += 1;
            }

            if bc > bits {
                bits = bc;
                steps = 1 << bits;
            }

            offset = range2 as f32 / steps as f32;
            high = low + range2 as f32 - offset;
        }

        let mut s = Self {
            low,
            high,
            high_low_mul: 0.0,
            dec_mul: 0.0,
            bits,
            rounddown: (flags & QFF_ROUNDDOWN) != 0,
            roundup: (flags & QFF_ROUNDUP) != 0,
            encode_zero: (flags & QFF_ENCODE_ZERO) != 0,
        };

        s.assign_multipliers(steps)?;

        if s.rounddown && s.quantize(s.low)? == s.low {
            s.rounddown = false;
        }

        if s.roundup && s.quantize(s.high)? == s.high {
            s.roundup = false;
        }

        if s.encode_zero && s.quantize(0.0)? == 0.0 {
            s.encode_zero = false;
        }

        Ok(s)
    }

    pub fn assign_multipliers(&mut self, steps: u32) -> Result<(), std::io::Error> {
        self.high_low_mul = 0.0;
        let range = self.high - self.low;

        let high: u32 = if self.bits == 32 {
            0xFFFFFFFE
        } else {
            (1u32 << self.bits) - 1
        };

        let mut high_mul = if range.abs() <= 0.0 {
            high as f32
        } else {
            high as f32 / range
        };

        if high_mul * range > high as f32 || (high_mul * range) as f64 > high as f64 {
            const Q_FLOAT_MULTIPLIERS: [f32; 5] = [0.9999, 0.99, 0.9, 0.8, 0.7];
            for mult in Q_FLOAT_MULTIPLIERS.iter() {
                high_mul = high as f32 / range * *mult;

                if high_mul * range > high as f32 || (high_mul * range) as f64 > high as f64 {
                    continue;
                }

                break;
            }
        }

        self.high_low_mul = high_mul;
        self.dec_mul = 1.0 / (steps as f32 - 1.0);

        if self.high_low_mul == 0.0 {
            Err(std::io::Error::other(
                "Error computing high / low multiplier",
            ))
        } else {
            Ok(())
        }
    }

    pub fn quantize(&self, value: f32) -> Result<f32, std::io::Error> {
        if value < self.low {
            if !self.roundup {
                return Err(std::io::Error::other(
                    "Field tried to quantize an out of range value",
                ));
            }

            return Ok(self.low);
        } else if value > self.high {
            if !self.rounddown {
                return Err(std::io::Error::other(
                    "Field tried to quantize an out of range value",
                ));
            }

            return Ok(self.high);
        }

        let i = ((value - self.low) * self.high_low_mul) as u32;
        Ok(self.low + (self.high - self.low) * (i as f32 * self.dec_mul))
    }
}

impl EntitySerializerTyped<f32> for F32SerializerQuantized {
    #[inline(always)]
    fn decode_typed(
        &self,
        entity: Option<&mut f32>,
        _path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error> {
        if let Some(e) = entity {
            let v = if self.rounddown && reader.read_bit()? {
                self.low
            } else if self.roundup && reader.read_bit()? {
                self.high
            } else if self.encode_zero && reader.read_bit()? {
                0.0
            } else {
                let v = reader.read_var::<u64>(self.bits)?;
                self.low + (self.high - self.low) * v as f32 * self.dec_mul
            };
            *e = v;
        } else if !(self.rounddown && reader.read_bit()?
            || self.roundup && reader.read_bit()?
            || self.encode_zero && reader.read_bit()?)
        {
            reader.skip(self.bits)?;
        }

        Ok(())
    }
}
