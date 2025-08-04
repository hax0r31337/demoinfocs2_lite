use std::{any::Any, sync::Arc};

use crate::entity::serializer::EntityClassSerializer;

const MAX_ENTITIES_IN_LIST: usize = 512;
const MAX_ENTITY_LISTS: usize = 64;

const ENTITY_CHUNK_SHIFT: u32 = MAX_ENTITIES_IN_LIST.trailing_zeros();
const ENTITY_OFFSET_MASK: usize = MAX_ENTITIES_IN_LIST - 1;

pub struct EntityItem {
    pub serial: u32,
    pub item: Box<dyn Any + Send + Sync>,
    pub serializer: Arc<dyn EntityClassSerializer>,
}

/// a simple implementation of CConcreteEntityList
/// i guess it's better than a flat vector
/// as the original CConcreteEntityList does the same thing
pub struct EntityList {
    entity_chunk: [Option<Box<EntityChunk>>; MAX_ENTITY_LISTS],
}

struct EntityChunk {
    counter: usize,
    entities: [Option<EntityItem>; MAX_ENTITIES_IN_LIST],
}

impl EntityList {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            entity_chunk: [const { None }; MAX_ENTITY_LISTS],
        }
    }

    fn chunk(&self, idx: usize) -> Option<&EntityChunk> {
        if idx >= MAX_ENTITY_LISTS {
            return None;
        }

        unsafe {
            self.entity_chunk
                .get_unchecked(idx)
                .as_ref()
                .map(|c| c.as_ref())
        }
    }

    fn chunk_mut(&mut self, idx: usize) -> Option<&mut EntityChunk> {
        if idx >= MAX_ENTITY_LISTS {
            return None;
        }

        unsafe {
            self.entity_chunk
                .get_unchecked_mut(idx)
                .as_mut()
                .map(|c| c.as_mut())
        }
    }

    pub fn get(&self, idx: usize) -> Option<&EntityItem> {
        let chunk = self.chunk(idx >> ENTITY_CHUNK_SHIFT)?;
        let entity_idx = idx & ENTITY_OFFSET_MASK;

        unsafe { chunk.entities.get_unchecked(entity_idx).as_ref() }
    }

    pub fn get_mut(&mut self, idx: usize) -> Option<&mut EntityItem> {
        let chunk = self.chunk_mut(idx >> ENTITY_CHUNK_SHIFT)?;
        let entity_idx = idx & ENTITY_OFFSET_MASK;

        unsafe { chunk.entities.get_unchecked_mut(entity_idx).as_mut() }
    }

    pub fn delete(&mut self, idx: usize) -> Option<EntityItem> {
        let chunk = self.chunk_mut(idx >> ENTITY_CHUNK_SHIFT)?;
        let entity_idx = idx & ENTITY_OFFSET_MASK;

        let entity = unsafe { chunk.entities.get_unchecked_mut(entity_idx) }.take()?;

        chunk.counter -= 1;
        if chunk.counter == 0 {
            self.entity_chunk[idx >> ENTITY_CHUNK_SHIFT] = None;
        }

        Some(entity)
    }

    pub fn insert(&mut self, idx: usize, entity: EntityItem) -> Option<EntityItem> {
        let chunk_idx = idx >> ENTITY_CHUNK_SHIFT;
        if chunk_idx >= MAX_ENTITY_LISTS {
            return None;
        }

        let chunk = unsafe {
            let chunk = self.entity_chunk.get_unchecked_mut(chunk_idx);
            if chunk.is_none() {
                *chunk = Some(Box::new(
                    const {
                        EntityChunk {
                            counter: 0,
                            entities: [const { None }; MAX_ENTITIES_IN_LIST],
                        }
                    },
                ));
            }
            chunk.as_mut().unwrap()
        };

        let entity_idx = idx & ENTITY_OFFSET_MASK;

        let old_entity = unsafe { chunk.entities.get_unchecked_mut(entity_idx) };
        if old_entity.is_none() {
            chunk.counter += 1;
        }
        old_entity.replace(entity)
    }
}
