//! The thin, object-safe `Context` contract (spec §6).

use armillae_core::{Message, TokenUsage};

use crate::error::ContextError;
use crate::protocol::CompressionTarget;

/// Paradigm-neutral contract for producing inferable context.
///
/// A `Context` owns dialogue writes, export, and the compression pipeline
/// (evaluate → prepare → downstream inference → apply). Persistence, recovery,
/// query, and state observation are paradigm-owned and never part of this
/// trait; `CompressionState` appears only in error expression. The trait is
/// object-safe and `Send + Sync`, held as `Arc<dyn Context>` and driven
/// serially by its downstream.
pub trait Context: Send + Sync {
    /// Append a user message. Only allowed while the pipeline is `Idle`.
    fn push_user_input(&mut self, message: Message) -> Result<(), ContextError>;

    /// Record the assistant output of a complete turn. `usage` is required by
    /// the signature; the token facts become the context size. Only allowed
    /// while the pipeline is `Idle`.
    fn apply_model_output(
        &mut self,
        message: Message,
        usage: TokenUsage,
    ) -> Result<(), ContextError>;

    /// Produce the inferable context (pure function, no side effects).
    fn export(&self) -> Result<Vec<Message>, ContextError>;

    /// Evaluate the paradigm-internal trigger and produce a target, or `None`
    /// when this round should not compress. `Some` freezes the pipeline at
    /// `Evaluated`.
    fn evaluate_compression(&mut self) -> Result<Option<CompressionTarget>, ContextError>;

    /// Prepare inferable messages for the target; must follow evaluation
    /// (calling from `Idle` is an `InvalidState`). The paradigm persists the
    /// original content first and generates read-only messages without
    /// modifying the context structure.
    fn prepare_compression(
        &mut self,
        target: CompressionTarget,
    ) -> Result<Vec<Message>, ContextError>;

    /// Replace the view with the downstream-produced summary and return to
    /// `Idle`; the paradigm persists snapshot and state.
    fn apply_compression_result(&mut self, summary: Vec<Message>) -> Result<(), ContextError>;

    /// Abandon an evaluated or prepared compression and return to `Idle`;
    /// calling from `Idle` is a no-op success.
    fn abandon_compression(&mut self) -> Result<(), ContextError>;
}
