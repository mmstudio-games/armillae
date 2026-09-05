//! Section-paradigm persistence contract (spec §7.1.7).
//!
//! Persistence is paradigm-owned: the section paradigm defines its own store
//! contract and entry types, and its downstream implements the store. Storage
//! medium, serialization, lazy loading, cache replacement, and cross-entry
//! atomicity are free choices of the paradigm and its downstream; a missing
//! compression snapshot is tolerated (it degrades to the original view).

use std::time::SystemTime;

use armillae_core::Message;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::protocol::{CompressionState, CompressionTarget};
use crate::section::{ActiveWindow, Section};

/// Schema version of `SectionState` (spec §7.1.7).
pub const SECTION_STATE_SCHEMA_VERSION: u32 = 1;

/// Window state of the three-zone partition, persisted with the store entry
/// (spec §7.1.7).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WindowState {
    /// Active window mode.
    pub mode: ActiveWindow,
    /// Cache-zone section count (determined at creation, never changes).
    pub cache_prefix_sections: usize,
    /// Number of sealed (compression-candidate) sections.
    pub sealed_count: usize,
    /// Number of active (open) sections.
    pub active_count: usize,
}

/// Token-count facts: the most recent turn's `usage.input_tokens` is the
/// official context size (spec §9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TokenFacts {
    /// Most recent turn input tokens.
    pub input_tokens: u64,
}

/// Section-paradigm state persisted via `SectionStore` (spec §7.1.7).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SectionState {
    pub schema_version: u32,
    pub session_id: String,
    pub sections: Vec<Section>,
    pub window: WindowState,
    pub machine: CompressionState,
    pub token_facts: TokenFacts,
}

/// Compressed entry of the section paradigm (spec §7.1.7). The summary view
/// is carried natively by `compressed_text`; persistent stores may serialize
/// entries themselves, so the in-memory path never touches JSON.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SectionCompressedEntry {
    pub session_id: String,
    pub record_id: String,
    pub compressed_text: Vec<Message>,
    pub original_ref: OriginalRef,
    pub version: u64,
    #[schemars(with = "String")]
    pub archived_at: SystemTime,
}

/// Original entry of the section paradigm (spec §7.1.7).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SectionOriginalEntry {
    pub session_id: String,
    pub target: CompressionTarget,
    pub messages: Vec<Message>,
    pub version: u64,
    #[schemars(with = "String")]
    pub archived_at: SystemTime,
}

/// Opaque non-empty reference to a compressed entry (section paradigm).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct CompressedRef(String);

impl CompressedRef {
    /// Construct a reference; `None` when the value is empty.
    pub fn new(value: String) -> Option<Self> {
        if value.is_empty() {
            return None;
        }
        Some(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for CompressedRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value)
            .ok_or_else(|| serde::de::Error::custom("compressed reference must not be empty"))
    }
}

/// Opaque non-empty reference to an original entry (section paradigm).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct OriginalRef(String);

impl OriginalRef {
    /// Construct a reference; `None` when the value is empty.
    pub fn new(value: String) -> Option<Self> {
        if value.is_empty() {
            return None;
        }
        Some(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for OriginalRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value)
            .ok_or_else(|| serde::de::Error::custom("original reference must not be empty"))
    }
}

/// Original snapshot produced inside `prepare` and persisted before any
/// compression messages are generated (spec §7.1.7).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OriginalSnapshot {
    pub section_id: u64,
    pub messages: Vec<Message>,
    pub version: u64,
}

/// Errors returned by `SectionStore` implementations (section paradigm).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    /// The persistence backend failed.
    #[error("persistence backend error: {message}")]
    Backend { message: String },
    /// The store produced or received an invalid entry.
    #[error("invalid store entry: {message}")]
    InvalidEntry { message: String },
}

/// Store contract of the section paradigm; implemented by its downstream and
/// injected at construction (spec §7.1.7).
pub trait SectionStore: Send + Sync {
    fn save_state(&self, state: &SectionState) -> Result<(), StoreError>;
    fn load_state(&self, session_id: &str) -> Result<Option<SectionState>, StoreError>;
    fn delete_state(&self, session_id: &str) -> Result<(), StoreError>;

    fn save_compressed(&self, entry: &SectionCompressedEntry) -> Result<CompressedRef, StoreError>;
    fn load_compressed(
        &self,
        session_id: &str,
        reference: &CompressedRef,
    ) -> Result<Option<SectionCompressedEntry>, StoreError>;
    fn delete_compressed(
        &self,
        session_id: &str,
        reference: &CompressedRef,
    ) -> Result<(), StoreError>;

    fn save_original(&self, entry: &SectionOriginalEntry) -> Result<OriginalRef, StoreError>;
    fn load_original(
        &self,
        session_id: &str,
        reference: &OriginalRef,
    ) -> Result<Option<SectionOriginalEntry>, StoreError>;
    fn delete_original(&self, session_id: &str, reference: &OriginalRef) -> Result<(), StoreError>;
}
