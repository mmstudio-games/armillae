//! Production-grade in-memory `SectionStore` (default build).
//!
//! The store keeps the section paradigm's native data model directly:
//! `SectionState` / `Section` / `Turn` / `Message` are cloned as-is and
//! keyed by the opaque reference types. No JSON serialization happens on the
//! storage path, so the in-memory round-trip is a plain clone/hash lookup.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::store::{
    CompressedRef, OriginalRef, SectionCompressedEntry, SectionOriginalEntry, SectionState,
    SectionStore, StoreError,
};

#[derive(Default)]
struct SessionData {
    state: Option<SectionState>,
    compressed: HashMap<CompressedRef, SectionCompressedEntry>,
    original: HashMap<OriginalRef, SectionOriginalEntry>,
}

/// Thread-safe in-memory `SectionStore` backed by a mutex-protected map of
/// sessions. Entry references are generated internally; storage is native
/// (no serialization) and per-session isolation is guaranteed by the map.
pub struct InMemorySectionStore {
    inner: Mutex<HashMap<String, SessionData>>,
    next: AtomicU64,
}

impl Default for InMemorySectionStore {
    fn default() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            next: AtomicU64::new(0),
        }
    }
}

impl InMemorySectionStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, HashMap<String, SessionData>>, StoreError> {
        self.inner.lock().map_err(|_| StoreError::Backend {
            message: "in-memory store lock poisoned".to_owned(),
        })
    }
}

impl SectionStore for InMemorySectionStore {
    fn save_state(&self, state: &SectionState) -> Result<(), StoreError> {
        self.lock()?
            .entry(state.session_id.clone())
            .or_default()
            .state = Some(state.clone());
        Ok(())
    }

    fn load_state(&self, session_id: &str) -> Result<Option<SectionState>, StoreError> {
        Ok(self
            .lock()?
            .get(session_id)
            .and_then(|data| data.state.clone()))
    }

    fn delete_state(&self, session_id: &str) -> Result<(), StoreError> {
        if let Some(data) = self.lock()?.get_mut(session_id) {
            data.state = None;
        }
        Ok(())
    }

    fn save_compressed(&self, entry: &SectionCompressedEntry) -> Result<CompressedRef, StoreError> {
        let reference = CompressedRef::new(format!("c-{}", entry.record_id)).ok_or_else(|| {
            StoreError::InvalidEntry {
                message: "compressed record id is empty".to_owned(),
            }
        })?;
        self.lock()?
            .entry(entry.session_id.clone())
            .or_default()
            .compressed
            .insert(reference.clone(), entry.clone());
        Ok(reference)
    }

    fn load_compressed(
        &self,
        session_id: &str,
        reference: &CompressedRef,
    ) -> Result<Option<SectionCompressedEntry>, StoreError> {
        Ok(self
            .lock()?
            .get(session_id)
            .and_then(|data| data.compressed.get(reference).cloned()))
    }

    fn delete_compressed(
        &self,
        session_id: &str,
        reference: &CompressedRef,
    ) -> Result<(), StoreError> {
        if let Some(data) = self.lock()?.get_mut(session_id) {
            data.compressed.remove(reference);
        }
        Ok(())
    }

    fn save_original(&self, entry: &SectionOriginalEntry) -> Result<OriginalRef, StoreError> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let reference =
            OriginalRef::new(format!("o-{id}")).ok_or_else(|| StoreError::InvalidEntry {
                message: "generated original reference is empty".to_owned(),
            })?;
        self.lock()?
            .entry(entry.session_id.clone())
            .or_default()
            .original
            .insert(reference.clone(), entry.clone());
        Ok(reference)
    }

    fn load_original(
        &self,
        session_id: &str,
        reference: &OriginalRef,
    ) -> Result<Option<SectionOriginalEntry>, StoreError> {
        Ok(self
            .lock()?
            .get(session_id)
            .and_then(|data| data.original.get(reference).cloned()))
    }

    fn delete_original(&self, session_id: &str, reference: &OriginalRef) -> Result<(), StoreError> {
        if let Some(data) = self.lock()?.get_mut(session_id) {
            data.original.remove(reference);
        }
        Ok(())
    }
}

// —— 测试观测辅助（仅 testing feature） ——

#[cfg(feature = "testing")]
impl InMemorySectionStore {
    /// Whether a session state is currently stored (test observation).
    pub fn has_state(&self, session_id: &str) -> bool {
        self.lock()
            .expect("store lock must not be poisoned")
            .get(session_id)
            .and_then(|data| data.state.as_ref())
            .is_some()
    }

    /// Number of stored original entries for a session (test observation).
    pub fn original_count(&self, session_id: &str) -> usize {
        self.lock()
            .expect("store lock must not be poisoned")
            .get(session_id)
            .map_or(0, |data| data.original.len())
    }

    /// Number of stored compressed entries for a session (test observation).
    pub fn compressed_count(&self, session_id: &str) -> usize {
        self.lock()
            .expect("store lock must not be poisoned")
            .get(session_id)
            .map_or(0, |data| data.compressed.len())
    }

    /// Drop all compressed entries for a session (simulates snapshot loss).
    pub fn clear_compressed(&self, session_id: &str) {
        if let Some(data) = self
            .lock()
            .expect("store lock must not be poisoned")
            .get_mut(session_id)
        {
            data.compressed.clear();
        }
    }

    /// Read back the persisted state for a session (test observation).
    pub fn session_state(&self, session_id: &str) -> Option<SectionState> {
        self.lock()
            .expect("store lock must not be poisoned")
            .get(session_id)
            .and_then(|data| data.state.clone())
    }
}
