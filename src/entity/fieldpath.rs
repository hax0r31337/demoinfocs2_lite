// https://github.com/dotabuff/manta/blob/master/field_path.go

use std::{cmp::Ordering, collections::BinaryHeap};

use bitstream_io::BitRead;

use crate::{bit::BitReaderExt, entity::Reader};

#[repr(C)]
pub struct FieldPath {
    pub path: [i32; 7],
    pub last: u8,
    pub is_done: bool,
    _padding: [u8; 2],
}

pub const DEFAULT_FIELD_PATH: FieldPath = FieldPath {
    path: [-1, 0, 0, 0, 0, 0, 0],
    last: 0,
    is_done: false,
    _padding: [0; 2],
};

impl PartialEq for FieldPath {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.last == other.last
    }
}

impl FieldPath {
    #[inline(always)]
    pub fn last(&self) -> usize {
        self.last as usize
    }

    #[inline(always)]
    pub fn pop(&mut self, qty: usize) {
        let new_last = self.last.saturating_sub(qty as u8);

        for i in (new_last + 1)..=self.last {
            self.path[i as usize] = 0;
        }

        self.last = new_last;
    }

    #[inline(always)]
    pub fn finish(&self, paths: &mut Vec<FieldPathFixed>) {
        let len = paths.len();
        if paths.capacity() == len {
            paths.reserve(64);
        }

        unsafe {
            let ptr = paths.as_mut_ptr().add(len);
            std::ptr::copy_nonoverlapping(
                self as *const FieldPath as *const FieldPathFixed,
                ptr,
                1,
            );
            paths.set_len(len + 1);
        }
    }
}

pub struct FieldPathFixed([u32; 8]);

impl FieldPathFixed {
    pub fn to_slice(&self) -> &[u32] {
        let len = self.0[7] as usize + 1;

        unsafe { std::slice::from_raw_parts(self.0.as_ptr(), len) }
    }
}

type FieldPathOpFn = fn(&mut Reader<'_>, &mut FieldPath) -> std::result::Result<(), std::io::Error>;

struct FieldPathOp {
    // pub name: &'static str,
    pub weight: u32,
    pub op: FieldPathOpFn,
}

macro_rules! field_path_op {
    ($name:ident, $weight:expr, $op:expr) => {
        FieldPathOp {
            // name: stringify!($name),
            weight: $weight,
            op: $op,
        }
    };
}

const FIELD_PATH_OPS: [FieldPathOp; 40] = [
    field_path_op!(PlusOne, 36271, |_, path| {
        path.path[path.last()] += 1;
        Ok(())
    }),
    field_path_op!(PlusTwo, 10334, |_, path| {
        path.path[path.last()] += 2;
        Ok(())
    }),
    field_path_op!(PlusThree, 1375, |_, path| {
        path.path[path.last()] += 3;
        Ok(())
    }),
    field_path_op!(PlusFour, 646, |_, path| {
        path.path[path.last()] += 4;
        Ok(())
    }),
    field_path_op!(PlusN, 4128, |reader, path| {
        path.path[path.last()] += reader.read_ubit_int_fp()? + 5;
        Ok(())
    }),
    field_path_op!(PushOneLeftDeltaZeroRightZero, 35, |_, path| {
        path.last += 1;
        path.path[path.last()] = 0;
        Ok(())
    }),
    field_path_op!(PushOneLeftDeltaZeroRightNonZero, 3, |reader, path| {
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        Ok(())
    }),
    field_path_op!(PushOneLeftDeltaOneRightZero, 521, |_, path| {
        path.path[path.last()] += 1;
        path.last += 1;
        path.path[path.last()] = 0;
        Ok(())
    }),
    field_path_op!(PushOneLeftDeltaOneRightNonZero, 2942, |reader, path| {
        path.path[path.last()] += 1;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        Ok(())
    }),
    field_path_op!(PushOneLeftDeltaNRightZero, 560, |reader, path| {
        path.path[path.last()] += reader.read_ubit_int_fp()?;
        path.last += 1;
        path.path[path.last()] = 0;
        Ok(())
    }),
    field_path_op!(PushOneLeftDeltaNRightNonZero, 471, |reader, path| {
        path.path[path.last()] += reader.read_ubit_int_fp()? + 2;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()? + 1;
        Ok(())
    }),
    field_path_op!(
        PushOneLeftDeltaNRightNonZeroPack6Bits,
        10530,
        |reader, path| {
            path.path[path.last()] += reader.read_unsigned::<3, u32>()? as i32 + 2;
            path.last += 1;
            path.path[path.last()] = reader.read_unsigned::<3, u32>()? as i32 + 1;
            Ok(())
        }
    ),
    field_path_op!(
        PushOneLeftDeltaNRightNonZeroPack8Bits,
        251,
        |reader, path| {
            path.path[path.last()] += reader.read_unsigned::<4, u32>()? as i32 + 2;
            path.last += 1;
            path.path[path.last()] = reader.read_unsigned::<4, u32>()? as i32 + 1;
            Ok(())
        }
    ),
    field_path_op!(PushTwoLeftDeltaZero, 0, |reader, path| {
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        Ok(())
    }),
    field_path_op!(PushTwoPack5LeftDeltaZero, 0, |reader, path| {
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        Ok(())
    }),
    field_path_op!(PushThreeLeftDeltaZero, 0, |reader, path| {
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        Ok(())
    }),
    field_path_op!(PushThreePack5LeftDeltaZero, 0, |reader, path| {
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        Ok(())
    }),
    field_path_op!(PushTwoLeftDeltaOne, 0, |reader, path| {
        path.path[path.last()] += 1;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        Ok(())
    }),
    field_path_op!(PushTwoPack5LeftDeltaOne, 0, |reader, path| {
        path.path[path.last()] += 1;
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        Ok(())
    }),
    field_path_op!(PushThreeLeftDeltaOne, 0, |reader, path| {
        path.path[path.last()] += 1;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        Ok(())
    }),
    field_path_op!(PushThreePack5LeftDeltaOne, 0, |reader, path| {
        path.path[path.last()] += 1;
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        Ok(())
    }),
    field_path_op!(PushTwoLeftDeltaN, 0, |reader, path| {
        path.path[path.last()] += (reader.read_ubit_int()? as i32) + 2;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        Ok(())
    }),
    field_path_op!(PushTwoPack5LeftDeltaN, 0, |reader, path| {
        path.path[path.last()] += (reader.read_ubit_int()? as i32) + 2;
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        Ok(())
    }),
    field_path_op!(PushThreeLeftDeltaN, 0, |reader, path| {
        path.path[path.last()] += (reader.read_ubit_int()? as i32) + 2;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        path.last += 1;
        path.path[path.last()] = reader.read_ubit_int_fp()?;
        Ok(())
    }),
    field_path_op!(PushThreePack5LeftDeltaN, 0, |reader, path| {
        path.path[path.last()] += (reader.read_ubit_int()? as i32) + 2;
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        path.last += 1;
        path.path[path.last()] = reader.read_unsigned::<5, u32>()? as i32;
        Ok(())
    }),
    field_path_op!(PushN, 0, |reader, path| {
        let n = reader.read_ubit_int()? as usize;
        path.path[path.last()] += reader.read_ubit_int()? as i32;
        for _ in 0..n {
            path.last += 1;
            path.path[path.last()] = reader.read_ubit_int_fp()?;
        }
        Ok(())
    }),
    field_path_op!(PushNAndNonTopological, 310, |reader, path| {
        for i in 0..=path.last() {
            if reader.read_bit()? {
                path.path[i] += (reader.read_varint_i32()?) + 1;
            }
        }
        let count = reader.read_ubit_int()? as usize;
        for _ in 0..count {
            path.last += 1;
            path.path[path.last()] = reader.read_ubit_int_fp()?;
        }
        Ok(())
    }),
    field_path_op!(PopOnePlusOne, 2, |_, path| {
        path.pop(1);
        path.path[path.last()] += 1;
        Ok(())
    }),
    field_path_op!(PopOnePlusN, 0, |reader, path| {
        path.pop(1);
        path.path[path.last()] += reader.read_ubit_int_fp()? + 1;
        Ok(())
    }),
    field_path_op!(PopAllButOnePlusOne, 1837, |_, path| {
        path.pop(path.last());
        path.path[0] += 1;
        Ok(())
    }),
    field_path_op!(PopAllButOnePlusN, 149, |reader, path| {
        path.pop(path.last());
        path.path[0] += reader.read_ubit_int_fp()? + 1;
        Ok(())
    }),
    field_path_op!(PopAllButOnePlusNPack3Bits, 300, |reader, path| {
        path.pop(path.last());
        path.path[0] += reader.read_unsigned::<3, u32>()? as i32 + 1;
        Ok(())
    }),
    field_path_op!(PopAllButOnePlusNPack6Bits, 634, |reader, path| {
        path.pop(path.last());
        path.path[0] += reader.read_unsigned::<6, u32>()? as i32 + 1;
        Ok(())
    }),
    field_path_op!(PopNPlusOne, 0, |reader, path| {
        path.pop(reader.read_ubit_int_fp()? as usize);
        path.path[path.last()] += 1;
        Ok(())
    }),
    field_path_op!(PopNPlusN, 0, |reader, path| {
        path.pop(reader.read_ubit_int_fp()? as usize);
        path.path[path.last()] += reader.read_varint_i32()?;
        Ok(())
    }),
    field_path_op!(PopNAndNonTopographical, 1, |reader, path| {
        path.pop(reader.read_ubit_int_fp()? as usize);
        for i in 0..=path.last() {
            if reader.read_bit()? {
                path.path[i] += reader.read_varint_i32()?;
            }
        }
        Ok(())
    }),
    field_path_op!(NonTopoComplex, 76, |reader, path| {
        for i in 0..=path.last() {
            if reader.read_bit()? {
                path.path[i] += reader.read_varint_i32()?;
            }
        }
        Ok(())
    }),
    field_path_op!(NonTopoPenultimatePlusOne, 271, |_, path| {
        if path.last > 0 {
            path.path[path.last() - 1] += 1;
        }
        Ok(())
    }),
    field_path_op!(NonTopoComplexPack4Bits, 99, |reader, path| {
        for i in 0..=path.last() {
            if reader.read_bit()? {
                path.path[i] += reader.read_unsigned::<4, u32>()? as i32 - 7;
            }
        }
        Ok(())
    }),
    field_path_op!(FieldPathEncodeFinish, 25474, |_, path| {
        path.is_done = true;
        Ok(())
    }),
];

pub struct HuffmanNode {
    pub weight: u32,
    pub value: i32,
    pub op: Option<FieldPathOpFn>,
    pub left: Option<Box<HuffmanNode>>,
    pub right: Option<Box<HuffmanNode>>,
}

impl PartialEq for HuffmanNode {
    fn eq(&self, other: &Self) -> bool {
        self.weight == other.weight && self.value == other.value
    }
}

impl Eq for HuffmanNode {}

impl PartialOrd for HuffmanNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HuffmanNode {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.weight == other.weight {
            self.value.cmp(&other.value)
        } else {
            other.weight.cmp(&self.weight)
        }
    }
}

pub static FIELD_PATH_HUFFMAN: std::sync::LazyLock<HuffmanNode> = std::sync::LazyLock::new(|| {
    let mut trees = BinaryHeap::new();

    for (i, v) in FIELD_PATH_OPS.iter().enumerate() {
        trees.push(HuffmanNode {
            weight: v.weight.max(1),
            value: i as i32,
            op: Some(v.op),
            left: None,
            right: None,
        });
    }

    let mut n = 40;

    while trees.len() > 1 {
        let a = trees.pop().unwrap();
        let b = trees.pop().unwrap();

        let combined_weight = a.weight + b.weight;

        trees.push(HuffmanNode {
            weight: combined_weight,
            value: n,
            op: None,
            left: Some(Box::new(a)),
            right: Some(Box::new(b)),
        });
        n += 1;
    }

    trees.pop().unwrap()
});

pub fn read_field_paths(
    reader: &mut Reader<'_>,
    paths: &mut Vec<FieldPathFixed>,
) -> Result<(), std::io::Error> {
    let start_node: &HuffmanNode = &FIELD_PATH_HUFFMAN;
    let mut node = start_node;
    let mut path = DEFAULT_FIELD_PATH;

    while !path.is_done {
        let next = if reader.read_bit()? {
            node.right.as_deref()
        } else {
            node.left.as_deref()
        }
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid field path huffman tree",
            )
        })?;

        if let Some(op) = next.op {
            node = start_node;

            (op)(reader, &mut path)?;

            if !path.is_done {
                path.finish(paths);
            }
        } else {
            node = next;
        }
    }

    Ok(())
}
