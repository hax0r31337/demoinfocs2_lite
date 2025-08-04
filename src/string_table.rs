use std::io::Cursor;

use bitstream_io::{BitRead, BitReader};
use foldhash::{HashMap, HashMapExt};
use log::error;

use crate::{CsDemoParser, bit::BitReaderExt, protobuf};

pub const STRING_TABLE_INSTANCE_BASELINE: &str = "instancebaseline";

type StringTableMap = HashMap<String, (i32, Option<Box<[u8]>>)>;

pub struct StringTable<P: StringTableParser + Send + Sync, T: Send + Sync> {
    pub parser: P,

    pub map: StringTableMap,
    cache: HashMap<String, T>,
}

impl<P, T> StringTable<P, T>
where
    P: StringTableParser + Send + Sync,
    T: Send + Sync,
{
    pub fn new(parser: P) -> Self {
        Self {
            parser,
            map: HashMap::new(),
            cache: HashMap::new(),
        }
    }

    pub fn get_raw(&self, key: &str) -> Option<&[u8]> {
        self.map.get(key).and_then(|b| b.1.as_deref())
    }

    pub fn get_cached(&self, key: &str) -> Option<&T> {
        self.cache.get(key)
    }

    pub fn put_cache(&mut self, key: String, value: T) {
        self.cache.insert(key, value);
    }

    pub fn purge_cache(&mut self) {
        self.cache.clear();
    }
}

pub trait StringTableUpdatable {
    fn update(&mut self, entries: i32, data: &[u8]) -> Result<(), std::io::Error>;
    fn insert(&mut self, key: String, index: i32, value: Option<Box<[u8]>>);
}

impl<P, T> StringTableUpdatable for StringTable<P, T>
where
    P: StringTableParser + Send + Sync,
    T: Send + Sync,
{
    fn update(&mut self, entries: i32, data: &[u8]) -> Result<(), std::io::Error> {
        self.parser
            .update(&mut self.map, &mut self.cache, entries, data)
    }

    fn insert(&mut self, key: String, index: i32, value: Option<Box<[u8]>>) {
        // invalidate cache for this key
        self.cache.remove(&key);

        self.map.insert(key, (index, value));
    }
}

pub trait CacheInvalidator {
    fn remove(&mut self, key: &str);
}

impl<T> CacheInvalidator for HashMap<String, T>
where
    T: Send + Sync,
{
    fn remove(&mut self, key: &str) {
        self.remove(key);
    }
}

pub trait StringTableParser {
    fn update<CI>(
        &self,
        map: &mut StringTableMap,
        cache_invalidator: &mut CI,
        entries: i32,
        data: &[u8],
    ) -> Result<(), std::io::Error>
    where
        CI: CacheInvalidator;
}

const STRING_TABLE_PARSE_MAX_CACHE_SIZE: usize = 1 << 5;

pub struct BaselineStringTableParser {
    pub user_data_fixed_size: bool,
    pub user_data_size: i32,
    pub flags: i32,
    pub using_varint_bitcounts: bool,
}

impl StringTableParser for BaselineStringTableParser {
    fn update<CI>(
        &self,
        map: &mut StringTableMap,
        cache_invalidator: &mut CI,
        entries: i32,
        data: &[u8],
    ) -> Result<(), std::io::Error>
    where
        CI: CacheInvalidator,
    {
        let mut r = BitReader::endian(Cursor::new(data), bitstream_io::LittleEndian);

        let mut idx: i32 = 0;
        let mut keys = Vec::with_capacity(STRING_TABLE_PARSE_MAX_CACHE_SIZE);

        for _ in 0..entries {
            let incr = r.read_bit()?;
            if incr {
                idx += 1;
            } else {
                idx = r.read_varint_u32()? as i32 + 1;
            }

            let key = if r.read_bit()? {
                let key;
                if r.read_bit()? {
                    let pos = r.read_unsigned::<5, u8>()? as usize;
                    let size = r.read_unsigned::<5, u8>()? as usize;

                    if pos >= keys.len() {
                        key = r.read_null_terminated_string()?;
                    } else {
                        let s: &String = &keys[pos];
                        if size > s.len() {
                            key = s.to_owned() + &r.read_null_terminated_string()?;
                        } else {
                            key = s[..size].to_string() + &r.read_null_terminated_string()?;
                        }
                    }
                } else {
                    key = r.read_null_terminated_string()?;
                }

                keys.push(key.clone());

                if keys.len() > STRING_TABLE_PARSE_MAX_CACHE_SIZE {
                    keys.remove(0);
                }

                Some(key)
            } else {
                None
            };

            let value = if r.read_bit()? {
                let mut compressed = false;
                let bit_size = if self.user_data_fixed_size {
                    self.user_data_size as usize
                } else {
                    if self.flags & 1 != 0 {
                        compressed = r.read_bit()?;
                    }

                    if self.using_varint_bitcounts {
                        (r.read_ubit_int()? * 8) as usize
                    } else {
                        (r.read_unsigned::<17, u64>()? * 8) as usize
                    }
                };

                let mut buf = vec![0u8; bit_size.div_ceil(8)];
                let bytes = bit_size / 8;

                if bytes > 0 {
                    r.read_bytes(&mut buf[..bytes])?;
                }
                let bits = bit_size % 8;
                if bits > 0 {
                    buf[bytes] = r.read_var(bits as u32)?;
                }

                Some(if compressed {
                    snap::raw::Decoder::new()
                        .decompress_vec(&buf)
                        .map_err(|err| {
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                format!("Failed to decompress string table: {err:?}"),
                            )
                        })?
                } else {
                    buf
                })
            } else {
                None
            };

            let key = if let Some(k) = key {
                k
            } else if let Some(k) = map
                .iter()
                .find_map(|(k, (e, _))| if *e == idx { Some(k.clone()) } else { None })
            {
                k
            } else {
                error!("string table entry with index {idx} has no key");
                continue;
            };

            cache_invalidator.remove(&key);
            map.insert(key, (idx, value.map(Box::from)));
        }

        let bits = ((data.len() as u64) << 3) - r.position_in_bits()?;
        if bits >= 8 {
            error!("string table update did not consume all data: {bits}");
        }

        Ok(())
    }
}

impl<T: std::io::BufRead + Send + Sync> CsDemoParser<T> {
    pub(super) fn get_string_table(&mut self, name: &str) -> Option<&mut dyn StringTableUpdatable> {
        if name == STRING_TABLE_INSTANCE_BASELINE {
            self.instance_baseline
                .as_mut()
                .map(|t| t as &mut dyn StringTableUpdatable)
        } else {
            None
        }
    }

    #[cold]
    pub(super) fn handle_create_string_table(
        &mut self,
        msg: protobuf::CsvcMsgCreateStringTable,
    ) -> Result<(), std::io::Error> {
        let (Some(name), Some(entries), Some(string_data), Some(data_compressed)) = (
            msg.name,
            msg.num_entries,
            msg.string_data,
            msg.data_compressed,
        ) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Missing values in create string table message",
            ));
        };

        self.string_tables.push(name.clone());

        let table: &mut dyn StringTableUpdatable = if name == STRING_TABLE_INSTANCE_BASELINE {
            let (
                Some(user_data_fixed_size),
                Some(user_data_size),
                Some(flags),
                Some(using_varint_bitcounts),
            ) = (
                msg.user_data_fixed_size,
                msg.user_data_size,
                msg.flags,
                msg.using_varint_bitcounts,
            )
            else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Missing values in baseline string table",
                ));
            };

            let table = StringTable::new(BaselineStringTableParser {
                user_data_fixed_size,
                user_data_size,
                flags,
                using_varint_bitcounts,
            });

            self.instance_baseline = Some(table);

            self.instance_baseline.as_mut().unwrap()
        } else {
            return Ok(());
        };

        let string_data = if data_compressed {
            &snap::raw::Decoder::new()
                .decompress_vec(&string_data)
                .map_err(|err| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("Failed to decompress string table: {err:?}"),
                    )
                })?
        } else {
            string_data.as_ref()
        };

        table.update(entries, string_data)?;

        Ok(())
    }

    #[cold]
    pub(super) fn handle_update_string_table(
        &mut self,
        _msg: protobuf::CsvcMsgUpdateStringTable,
    ) -> Result<(), std::io::Error> {
        let (Some(table_id), Some(entries), Some(data)) =
            (_msg.table_id, _msg.num_changed_entries, _msg.string_data)
        else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Missing values in update string table message",
            ));
        };

        let table_id = table_id as usize;

        if table_id >= self.string_tables.len() {
            error!("Invalid string table ID: {table_id}");
            return Ok(());
        }

        let table_name = self.string_tables[table_id].clone();
        let Some(table) = self.get_string_table(&table_name) else {
            return Ok(());
        };

        table.update(entries, data.as_ref())?;

        Ok(())
    }

    pub(super) fn handle_demo_string_tables(
        &mut self,
        msg: protobuf::CDemoStringTables,
    ) -> Result<(), std::io::Error> {
        for table in msg.tables.into_iter() {
            let Some(name) = table.table_name else {
                error!("Missing name in demo string table");
                continue;
            };

            let Some(table_obj) = self.get_string_table(&name) else {
                continue;
            };

            for (index, item) in table.items.into_iter().enumerate() {
                let Some(name) = item.str else {
                    error!("Missing entry name in demo string table item");
                    continue;
                };

                table_obj.insert(
                    name,
                    index as i32,
                    item.data.map(|data| data.to_vec().into_boxed_slice()),
                );
            }
        }

        Ok(())
    }
}
