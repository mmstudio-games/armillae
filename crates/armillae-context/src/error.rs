//! Structured errors for the context subsystem (spec §11).
//!
//! Following the crate conventions of `armillae-llm` / `armillae-tools` /
//! `armillae-simulate`: error enums are `#[non_exhaustive]` with structured
//! named-field variants and derive `PartialEq + Eq`; lower-layer failures are
//! normalized into own variants with the facts callers need, never wrapping
//! another domain's error type.

use crate::protocol::CompressionState;
use crate::store::StoreError;

/// Errors returned by the context contract and its paradigms.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ContextError {
    /// The paradigm configuration is invalid.
    #[error("invalid context configuration: {message}")]
    InvalidConfiguration { message: String },

    /// An operation is illegal in the current compression pipeline state.
    #[error("invalid state for {operation}: expected {expected:?}, actual {actual:?}")]
    InvalidState {
        /// The operation that was rejected.
        operation: &'static str,
        /// The pipeline state the operation requires.
        expected: CompressionState,
        /// The pipeline state the paradigm was actually in.
        actual: CompressionState,
    },

    /// The request itself is invalid (for example an empty exported sequence).
    #[error("invalid request: {message}")]
    InvalidRequest { message: String },

    /// The operation violates paradigm semantics (for example a compression
    /// target that does not match the evaluated target).
    #[error("invalid operation: {message}")]
    InvalidOperation { message: String },

    /// The persistence backend failed. `StoreError` is normalized into this
    /// variant (backend failure is retryable; an invalid entry is not), so the
    /// cross-paradigm error never carries the paradigm store error type.
    #[error("store failure (retryable: {retryable}): {message}")]
    Store { retryable: bool, message: String },
}

impl From<StoreError> for ContextError {
    fn from(error: StoreError) -> Self {
        match error {
            StoreError::Backend { message } => ContextError::Store {
                retryable: true,
                message,
            },
            StoreError::InvalidEntry { message } => ContextError::Store {
                retryable: false,
                message,
            },
        }
    }
}
