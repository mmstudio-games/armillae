//! Thin context contract and compression paradigms for Armillae.
//!
//! This crate owns a paradigm-neutral `Context` contract that produces
//! inferable context, the compression pipeline three-state semantics, and the
//! built-in `SectionContext` section paradigm. It does not own LLM inference,
//! agent/tool scheduling, cross-paradigm persistence, or cache breakpoints.

mod context;
mod error;
mod machine;
mod memory;
mod protocol;
mod section;
mod store;

#[cfg(feature = "testing")]
pub mod testing;

pub use context::Context;
pub use error::ContextError;
pub use memory::InMemorySectionStore;
pub use protocol::{CompressionState, CompressionTarget, PROTOCOL_VERSION};
pub use section::{
    ActiveWindow, AutoCompression, CompressionMethod, LabelPolicy, MappingRecord, PARADIGM_ID,
    Section, SectionConfig, SectionContext, SectionContextBuilder, SectionLabel, StandardLabel,
    ToolTurnPolicy, Turn, View,
};
pub use store::{
    CompressedRef, OriginalRef, OriginalSnapshot, SECTION_STATE_SCHEMA_VERSION,
    SectionCompressedEntry, SectionOriginalEntry, SectionState, SectionStore, StoreError,
    TokenFacts, WindowState,
};
