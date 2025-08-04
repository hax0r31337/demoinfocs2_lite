use std::io::Read;

use bitstream_io::{BitRead, BitReader};

pub trait BitReaderExt {
    fn read_u8(&mut self) -> Result<u8, std::io::Error>;

    fn read_varint_u32(&mut self) -> Result<u32, std::io::Error>;
    fn read_varint_u64(&mut self) -> Result<u64, std::io::Error>;

    fn read_varint_i32(&mut self) -> Result<i32, std::io::Error>;
    fn read_varint_i64(&mut self) -> Result<i64, std::io::Error>;

    fn read_ubit_int(&mut self) -> Result<u32, std::io::Error>;
    fn read_ubit_int_fp(&mut self) -> Result<i32, std::io::Error>;

    fn read_null_terminated_string(&mut self) -> Result<String, std::io::Error>;
}

macro_rules! impl_read_varint {
    ($t:ty, $fn_name:ident, $signed_t:ty, $signed_fn_name:ident) => {
        fn $fn_name(&mut self) -> Result<$t, std::io::Error> {
            let mut value: $t = 0;
            let mut shift = 0u32;

            if self.byte_aligned() {
                loop {
                    let v = self.read_u8()?;
                    let flag = v & 0x80 != 0;

                    value |= ((v & 0x7F) as $t) << shift;

                    if !flag {
                        break;
                    }

                    shift += 7;
                }
            } else {
                loop {
                    let v = self.read_unsigned::<7, $t>()?;
                    let flag = self.read_bit()?;

                    value |= v << shift;

                    if !flag {
                        break;
                    }

                    shift += 7;
                }
            }

            Ok(value)
        }

        fn $signed_fn_name(&mut self) -> Result<$signed_t, std::io::Error> {
            let v = self.$fn_name()?;

            Ok(if v & 1 != 0 {
                (!(v >> 1)) as $signed_t
            } else {
                (v >> 1) as $signed_t
            })
        }
    };
}

impl<R: Read> BitReaderExt for BitReader<R, bitstream_io::LittleEndian> {
    #[inline(always)]
    fn read_u8(&mut self) -> Result<u8, std::io::Error> {
        if self.byte_aligned() {
            let mut buf = [0u8; 1];
            self.read_bytes(&mut buf)?;
            Ok(buf[0])
        } else {
            self.read_unsigned::<8, u8>()
        }
    }

    impl_read_varint!(u32, read_varint_u32, i32, read_varint_i32);
    impl_read_varint!(u64, read_varint_u64, i64, read_varint_i64);

    fn read_ubit_int(&mut self) -> Result<u32, std::io::Error> {
        let ret = self.read_unsigned::<6, u32>()?;

        Ok(match ret & (16 | 32) {
            16 => (ret & 15) | (self.read_unsigned::<4, u32>()? << 4),
            32 => (ret & 15) | ((self.read_u8()? as u32) << 4),
            48 => (ret & 15) | (self.read_unsigned::<28, u32>()? << 4),
            _ => ret,
        })
    }

    fn read_ubit_int_fp(&mut self) -> Result<i32, std::io::Error> {
        let v = if self.read_bit()? {
            self.read_unsigned::<2, u32>()
        } else if self.read_bit()? {
            self.read_unsigned::<4, u32>()
        } else if self.read_bit()? {
            self.read_unsigned::<10, u32>()
        } else if self.read_bit()? {
            self.read_unsigned::<17, u32>()
        } else {
            self.read_unsigned::<31, u32>()
        }?;

        Ok(v as i32)
    }

    fn read_null_terminated_string(&mut self) -> Result<String, std::io::Error> {
        let mut s = Vec::new();
        loop {
            let c = self.read_u8()?;
            if c == 0 {
                break;
            }

            s.push(c);
        }
        Ok(String::from_utf8_lossy(&s).to_string())
    }
}
