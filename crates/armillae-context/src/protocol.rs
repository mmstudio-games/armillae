//! Cross-paradigm protocol types and the public data protocol version.
//!
//! These types are shared by every compression paradigm and appear in the
//! `Context` trait signatures (spec §4, §5). Paradigm-owned types (config,
//! state, store entries, paradigm-specific operations) never belong here.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Public data protocol version of `armillae-context` (spec §4.1).
pub const PROTOCOL_VERSION: &str = "armillae.context/v1alpha1";

/// The evaluation and preparation stages of the compression flow share a
/// compression target. "How to compress" parameters are paradigm-internal and
/// never cross the public protocol (spec §5.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum CompressionTarget {
    /// Compress a single section of the section paradigm (current sole
    /// implementation).
    Section { id: u64 },
    // Future paradigms add their target shapes as new variants.
}

/// Compression pipeline state, used for error expression and the pipeline
/// semantics contract. The `Context` trait exposes no state query; the state
/// machine lives inside each paradigm (spec §5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompressionState {
    /// Pipeline idle; dialogue writes and manual operations are allowed.
    Idle,
    /// Evaluation produced a target; the context is frozen until prepare or
    /// abandon.
    Evaluated,
    /// Prepared; waiting for apply or abandon.
    Prepared,
}
