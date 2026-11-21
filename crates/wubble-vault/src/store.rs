//! Storage behind a small trait. Production uses `store_adapter::StoreLogins` (wubble-store's
//! `logins` table); `MemoryStore` serves tests.

use crate::origin::Origin;
use crate::record::{RecordId, StoredRecord};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("record not found")]
    NotFound,
    #[error("record id already exists")]
    Duplicate,
    #[error("store failure: {0}")]
    Backend(String),
}

pub trait LoginStore {
    fn insert(&mut self, record: StoredRecord) -> Result<(), StoreError>;
    /// Replace the encrypted secrets of an existing row. The vault only ever re-encrypts: the
    /// other fields are never changed through `update`.
    fn update(&mut self, record: StoredRecord) -> Result<(), StoreError>;
    fn delete(&mut self, id: &RecordId) -> Result<bool, StoreError>;
    /// `new` in place of row `old`, together or not at all (`NotFound`: no row `old`).
    fn replace(&mut self, old: &RecordId, new: StoredRecord) -> Result<(), StoreError>;
    fn get(&self, id: &RecordId) -> Result<Option<StoredRecord>, StoreError>;
    /// Rows whose origin equals `origin` exactly.
    fn by_origin(&self, origin: &Origin) -> Result<Vec<StoredRecord>, StoreError>;
    fn all(&self) -> Result<Vec<StoredRecord>, StoreError>;
    /// Bump times_used and set last_used_ms.
    fn touch(&mut self, id: &RecordId, now_ms: i64) -> Result<(), StoreError>;
}

/// In-memory store, ordered by id.
#[derive(Debug, Default)]
pub struct MemoryStore {
    rows: std::collections::BTreeMap<RecordId, StoredRecord>,
}

#[cfg(test)]
impl MemoryStore {
    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Mutable access to a row, for tamper tests.
    pub(crate) fn row_mut(&mut self, id: &RecordId) -> Option<&mut StoredRecord> {
        self.rows.get_mut(id)
    }
}

impl LoginStore for MemoryStore {
    fn insert(&mut self, record: StoredRecord) -> Result<(), StoreError> {
        if self.rows.contains_key(&record.id) {
            return Err(StoreError::Duplicate);
        }
        self.rows.insert(record.id.clone(), record);
        Ok(())
    }

    fn update(&mut self, record: StoredRecord) -> Result<(), StoreError> {
        match self.rows.get_mut(&record.id) {
            Some(row) => {
                *row = record;
                Ok(())
            }
            None => Err(StoreError::NotFound),
        }
    }

    fn delete(&mut self, id: &RecordId) -> Result<bool, StoreError> {
        Ok(self.rows.remove(id).is_some())
    }

    fn replace(&mut self, old: &RecordId, new: StoredRecord) -> Result<(), StoreError> {
        if !self.rows.contains_key(old) {
            return Err(StoreError::NotFound);
        }
        if self.rows.contains_key(&new.id) {
            return Err(StoreError::Duplicate);
        }
        self.rows.remove(old);
        self.rows.insert(new.id.clone(), new);
        Ok(())
    }

    fn get(&self, id: &RecordId) -> Result<Option<StoredRecord>, StoreError> {
        Ok(self.rows.get(id).cloned())
    }

    fn by_origin(&self, origin: &Origin) -> Result<Vec<StoredRecord>, StoreError> {
        Ok(self
            .rows
            .values()
            .filter(|r| &r.origin == origin)
            .cloned()
            .collect())
    }

    fn all(&self) -> Result<Vec<StoredRecord>, StoreError> {
        Ok(self.rows.values().cloned().collect())
    }

    fn touch(&mut self, id: &RecordId, now_ms: i64) -> Result<(), StoreError> {
        let row = self.rows.get_mut(id).ok_or(StoreError::NotFound)?;
        row.times_used += 1;
        row.last_used_ms = Some(now_ms);
        Ok(())
    }
}
