use std::{any::Any, sync::Arc};

use bitstream_io::BitRead;

use crate::{
    bit::BitReaderExt,
    entity::{
        Reader,
        decoder::{serializer_derivation, skip_varint},
        field::FieldType,
    },
};

pub mod vector;

pub trait EntitySerializer: Send + Sync {
    fn decode(
        &self,
        entity: Option<&mut dyn Any>,
        path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error>;

    fn new_entity(&self) -> Box<dyn Any + Send + Sync>;
}

pub trait EntityClassSerializer: EntitySerializer {
    fn serializer_derivation(&self, field_type: &FieldType) -> Arc<dyn EntitySerializer>;

    fn clone_entity(&self, entity: &dyn Any) -> Result<Box<dyn Any + Send + Sync>, std::io::Error>;
}

pub trait EntityField: Any + Send + Sync + Sized + Clone + 'static {
    fn new() -> Self;

    #[inline(always)]
    fn as_any(&self) -> &dyn Any {
        self
    }

    #[inline(always)]
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// implement EntityField for primitive types
impl EntityField for bool {
    #[inline(always)]
    fn new() -> Self {
        false
    }
}

impl EntityField for f32 {
    #[inline(always)]
    fn new() -> Self {
        0.0
    }
}

impl EntityField for u64 {
    #[inline(always)]
    fn new() -> Self {
        0
    }
}

impl EntityField for i64 {
    #[inline(always)]
    fn new() -> Self {
        0
    }
}

impl EntityField for String {
    #[inline(always)]
    fn new() -> Self {
        String::new()
    }
}

impl<T: EntityField> EntityField for Vec<T> {
    #[inline(always)]
    fn new() -> Self {
        Vec::new()
    }
}

impl<T: EntityField> EntityField for Option<T> {
    #[inline(always)]
    fn new() -> Self {
        None
    }
}

impl EntityTypeWarp<f32> for u64 {
    #[inline(always)]
    fn as_entity_field(&mut self) -> f32 {
        *self as f32
    }
}

pub trait EntitySerializerTyped<T: EntityField>: Send + Sync {
    fn decode_typed(
        &self,
        entity: Option<&mut T>,
        path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error>;
}

pub struct TypedEntitySerializerAdapter<S, T>
where
    S: EntitySerializerTyped<T>,
    T: EntityField,
{
    inner: S,
    _marker: std::marker::PhantomData<T>,
}

impl<S, T> TypedEntitySerializerAdapter<S, T>
where
    S: EntitySerializerTyped<T>,
    T: EntityField,
{
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<S, T> EntitySerializer for TypedEntitySerializerAdapter<S, T>
where
    S: EntitySerializerTyped<T>,
    T: EntityField,
{
    #[inline(always)]
    fn decode(
        &self,
        entity: Option<&mut dyn Any>,
        path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error> {
        if let Some(e) = entity {
            if let Some(e) = e.downcast_mut::<T>() {
                self.inner.decode_typed(Some(e), path, reader)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "Entity type mismatch: expected {}",
                        std::any::type_name::<T>()
                    ),
                ))
            }
        } else {
            self.inner.decode_typed(None, path, reader)
        }
    }

    #[inline(always)]
    fn new_entity(&self) -> Box<dyn Any + Send + Sync> {
        Box::new(T::new())
    }
}

pub trait EntityMultiComponents<T: EntityField, const N: usize>: EntityField {
    fn get_items(&mut self) -> &mut [T; N];
}

pub struct EntitySerializerMultiComponents<
    S: EntitySerializerTyped<T>,
    E: EntityMultiComponents<T, N>,
    T: EntityField,
    const N: usize,
> {
    inner: S,
    _entity: std::marker::PhantomData<E>,
    _marker: std::marker::PhantomData<T>,
}

impl<S, E, T, const N: usize> EntitySerializerMultiComponents<S, E, T, N>
where
    S: EntitySerializerTyped<T>,
    E: EntityMultiComponents<T, N>,
    T: EntityField,
{
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            _entity: std::marker::PhantomData,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<S, E, T, const N: usize> EntitySerializerTyped<E>
    for EntitySerializerMultiComponents<S, E, T, N>
where
    S: EntitySerializerTyped<T>,
    E: EntityMultiComponents<T, N>,
    T: EntityField,
{
    #[inline(always)]
    fn decode_typed(
        &self,
        entity: Option<&mut E>,
        path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error> {
        if let Some(e) = entity {
            let items = e.get_items();

            for item in items.iter_mut() {
                self.inner.decode_typed(Some(item), path, reader)?;
            }
        } else {
            for _ in 0..N {
                self.inner.decode_typed(None, path, reader)?;
            }
        }

        Ok(())
    }
}

pub trait EntityTypeWarp<D: EntityField>: EntityField {
    fn as_entity_field(&mut self) -> D;
}

pub struct EntitySerializerTypeWarpAdapter<S, F, D>
where
    S: EntitySerializerTyped<F>,
    F: EntityField + EntityTypeWarp<D>,
    D: EntityField,
{
    inner: S,
    _field: std::marker::PhantomData<F>,
    _marker: std::marker::PhantomData<D>,
}

impl<S, F, D> EntitySerializerTypeWarpAdapter<S, F, D>
where
    S: EntitySerializerTyped<F>,
    F: EntityField + EntityTypeWarp<D>,
    D: EntityField,
{
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            _field: std::marker::PhantomData,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<S, F, D> EntitySerializerTyped<D> for EntitySerializerTypeWarpAdapter<S, F, D>
where
    S: EntitySerializerTyped<F>,
    F: EntityField + EntityTypeWarp<D>,
    D: EntityField,
{
    #[inline(always)]
    fn decode_typed(
        &self,
        entity: Option<&mut D>,
        path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error> {
        if let Some(e) = entity {
            let mut v: F = F::new();

            self.inner.decode_typed(Some(&mut v), path, reader)?;

            *e = v.as_entity_field();

            Ok(())
        } else {
            self.inner.decode_typed(None, path, reader)
        }
    }
}

/// deserializes a dynamically sized vector
pub struct VectorSerializer<S: EntitySerializerTyped<T>, T: EntityField> {
    inner: S,
    _marker: std::marker::PhantomData<T>,
}

impl<S, T> VectorSerializer<S, T>
where
    S: EntitySerializerTyped<T>,
    T: EntityField,
{
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<S, T> EntitySerializerTyped<Vec<T>> for VectorSerializer<S, T>
where
    S: EntitySerializerTyped<T>,
    T: EntityField,
{
    #[inline(always)]
    fn decode_typed(
        &self,
        entity: Option<&mut Vec<T>>,
        path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error> {
        if path.is_empty() {
            // resize the vector to the expected size
            if let Some(e) = entity {
                let size = reader.read_varint_u64()? as usize;
                e.resize(size, T::new());
            } else {
                skip_varint(reader)?;
            }
        } else if let Some(e) = entity {
            let idx = path[0] as usize;
            if idx < e.len() {
                self.inner
                    .decode_typed(Some(&mut e[idx]), &path[1..], reader)?;
            } else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid vector index",
                ));
            }
        } else {
            self.inner.decode_typed(None, &path[1..], reader)?;
        }

        Ok(())
    }
}

/// deserializes a fixed-size array
/// but we don't know the size at compile time
pub struct ArraySerializer<S: EntitySerializerTyped<T>, T: EntityField> {
    inner: S,
    size: usize,
    _marker: std::marker::PhantomData<T>,
}

impl<S, T> ArraySerializer<S, T>
where
    S: EntitySerializerTyped<T>,
    T: EntityField,
{
    pub fn new(inner: S, size: usize) -> Self {
        Self {
            inner,
            size,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<S, T> EntitySerializerTyped<Vec<T>> for ArraySerializer<S, T>
where
    S: EntitySerializerTyped<T>,
    T: EntityField,
{
    #[inline(always)]
    fn decode_typed(
        &self,
        entity: Option<&mut Vec<T>>,
        path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error> {
        if let Some(e) = entity {
            if e.len() != self.size {
                e.resize(self.size, T::new());
            }

            let idx = if path.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Empty path is not allowed for ArraySerializer",
                ));
            } else {
                path[0] as usize
            };

            if idx < self.size {
                self.inner
                    .decode_typed(Some(&mut e[idx]), &path[1..], reader)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid array index",
                ))
            }
        } else {
            self.inner.decode_typed(None, &path[1..], reader)
        }
    }
}

/// deserializes a pointer type
pub struct OptionalSerializer<S: EntitySerializerTyped<T>, T: EntityField> {
    inner: S,
    _marker: std::marker::PhantomData<T>,
}

impl<S, T> OptionalSerializer<S, T>
where
    S: EntitySerializerTyped<T>,
    T: EntityField,
{
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<S, T> EntitySerializerTyped<Option<T>> for OptionalSerializer<S, T>
where
    S: EntitySerializerTyped<T>,
    T: EntityField,
{
    #[inline(always)]
    fn decode_typed(
        &self,
        entity: Option<&mut Option<T>>,
        path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error> {
        if path.is_empty() {
            let has_item = reader.read_bit()?;
            if let Some(e) = entity {
                if has_item {
                    e.replace(T::new());
                } else {
                    e.take();
                }
            }
        } else if let Some(e) = entity {
            if e.is_none() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "PointerSerializer expects a non-empty entity",
                ));
            }
            self.inner
                .decode_typed(Some(e.as_mut().unwrap()), path, reader)?;
        } else {
            self.inner.decode_typed(None, path, reader)?;
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct UnknownEntity;

impl EntityField for UnknownEntity {
    fn new() -> Self {
        UnknownEntity
    }
}

#[derive(Clone)]
pub struct UnknownEntitySerializer {
    serializers: Box<[Arc<dyn EntitySerializer>]>,
}

impl UnknownEntitySerializer {
    pub fn new_serializer(
        serializers: Vec<(&str, Arc<dyn EntitySerializer>)>,
    ) -> Arc<dyn EntityClassSerializer> {
        Arc::new(Self {
            serializers: serializers.into_iter().map(|(_, s)| s).collect(),
        })
    }
}

impl EntitySerializerTyped<UnknownEntity> for UnknownEntitySerializer {
    fn decode_typed(
        &self,
        _entity: Option<&mut UnknownEntity>,
        path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error> {
        if path.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Empty path is not allowed for EntitySerializerUnknownEntity",
            ));
        }

        let idx = path[0] as usize;
        if idx < self.serializers.len() {
            self.serializers[idx].decode(None, &path[1..], reader)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Unknown entity index: {} (max: {})",
                    path[0],
                    self.serializers.len() - 1,
                ),
            ))
        }
    }
}

impl EntitySerializer for UnknownEntitySerializer {
    fn decode(
        &self,
        _entity: Option<&mut dyn Any>,
        path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error> {
        self.decode_typed(None, path, reader)
    }

    fn new_entity(&self) -> Box<dyn Any + Send + Sync> {
        Box::new(UnknownEntity)
    }
}

impl EntityClassSerializer for UnknownEntitySerializer {
    fn serializer_derivation(&self, field_type: &FieldType) -> Arc<dyn EntitySerializer> {
        serializer_derivation(self.clone(), field_type)
    }

    fn clone_entity(&self, entity: &dyn Any) -> Result<Box<dyn Any + Send + Sync>, std::io::Error> {
        if let Some(e) = entity.downcast_ref::<UnknownEntity>() {
            Ok(Box::new(e.clone()))
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Entity type mismatch in UnknownEntitySerializer",
            ))
        }
    }
}

pub struct PolymorphicEntity {
    pub index: usize,
    pub item: Box<dyn Any + Send + Sync>,
}

pub struct PolymorphicSerializer {
    serializers: Box<[Arc<dyn EntitySerializer>]>,
}

impl PolymorphicSerializer {
    pub fn new(serializers: Box<[Arc<dyn EntitySerializer>]>) -> Self {
        Self { serializers }
    }
}

impl EntitySerializer for PolymorphicSerializer {
    fn decode(
        &self,
        entity: Option<&mut dyn Any>,
        path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error> {
        let Some(e) = entity else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "PolymorphicSerializer expects a non-empty entity",
            ));
        };

        // TODO: validate the code

        if let Some(e) = e.downcast_mut::<PolymorphicEntity>() {
            if path.is_empty() {
                let _present = reader.read_bit()?;
                let serializer_idx = reader.read_ubit_int()? as usize;
                e.item = if serializer_idx < self.serializers.len() {
                    let serializer = &self.serializers[serializer_idx];
                    e.index = serializer_idx;
                    serializer.new_entity()
                } else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!(
                            "Unknown polymorphic serializer index: {} (max: {})",
                            serializer_idx,
                            self.serializers.len() - 1
                        ),
                    ));
                };
            } else {
                let serializer = &self.serializers[e.index];
                serializer.decode(Some(&mut *e.item), path, reader)?;
            }
        } else if let Some(e) = e.downcast_mut::<usize>() {
            if path.is_empty() {
                let _present = reader.read_bit()?;
                let serializer_idx = reader.read_ubit_int()? as usize;
                *e = serializer_idx;
            } else {
                let serializer = &self.serializers[*e];
                serializer.decode(None, path, reader)?;
            }
        } else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "PolymorphicSerializer expects a PolymorphicEntity or usize",
            ));
        }

        Ok(())
    }

    fn new_entity(&self) -> Box<dyn Any + Send + Sync> {
        self.serializers[0].new_entity()
    }
}

pub type CustomEntitySerializerType<T> = Box<
    [(
        Arc<dyn EntitySerializer>,
        // getter of field reference
        Option<fn(&mut T) -> &mut dyn Any>,
        // on changed callbacks
        Option<fn(&mut T) -> Result<(), std::io::Error>>,
    )],
>;

#[derive(Clone)]
pub struct CustomEntitySerializer<T: EntityField> {
    #[allow(clippy::type_complexity)]
    serializers: CustomEntitySerializerType<T>,
}

impl<T: EntityField> CustomEntitySerializer<T> {
    pub fn new(serializers: CustomEntitySerializerType<T>) -> Self {
        Self { serializers }
    }
}

impl<T: EntityField> EntitySerializer for CustomEntitySerializer<T> {
    fn decode(
        &self,
        entity: Option<&mut dyn Any>,
        path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error> {
        if let Some(e) = entity {
            if let Some(e) = e.downcast_mut::<T>() {
                self.decode_typed(Some(e), path, reader)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Entity type mismatch in CustomEntitySerializer",
                ))
            }
        } else {
            self.decode_typed(None, path, reader)
        }
    }

    fn new_entity(&self) -> Box<dyn Any + Send + Sync> {
        Box::new(T::new())
    }
}

impl<T: EntityField> EntitySerializerTyped<T> for CustomEntitySerializer<T> {
    fn decode_typed(
        &self,
        entity: Option<&mut T>,
        path: &[u32],
        reader: &mut Reader<'_>,
    ) -> Result<(), std::io::Error> {
        if path.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid field path",
            ));
        }

        let idx = path[0] as usize;
        if idx >= self.serializers.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Invalid serializer index: {} (max: {})",
                    idx,
                    self.serializers.len() - 1,
                ),
            ));
        }

        let (serializer, field_getter, callback) = &self.serializers[idx];

        if let Some(e) = entity {
            if let Some(getter) = field_getter {
                let field = getter(e);
                serializer.decode(Some(field), &path[1..], reader)?;

                if let Some(callback) = callback {
                    callback(e)?;
                }
            } else {
                serializer.decode(None, &path[1..], reader)?;
            }
        } else {
            serializer.decode(None, &path[1..], reader)?;
        }

        Ok(())
    }
}

impl<T: EntityField> EntityClassSerializer for CustomEntitySerializer<T> {
    fn serializer_derivation(&self, field_type: &FieldType) -> Arc<dyn EntitySerializer> {
        serializer_derivation(self.clone(), field_type)
    }

    fn clone_entity(&self, entity: &dyn Any) -> Result<Box<dyn Any + Send + Sync>, std::io::Error> {
        if let Some(e) = entity.downcast_ref::<T>() {
            Ok(Box::new(e.clone()))
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Entity type mismatch in CustomEntitySerializer",
            ))
        }
    }
}
