//! Deterministic test doubles for the context contracts (spec §12).
//!
//! `MockContext` is a scripted paradigm: it accepts all dialogue writes,
//! exports its raw history, and evaluates compression deterministically from
//! its `auto` flag. It backs the shared contract tests and the paradigm-switch
//! test with `SectionContext`. `InMemorySectionStore` is re-exported from the
//! production `memory` module so the section-paradigm tests run against the
//! same store production users get.

use armillae_core::{Message, TokenUsage};

use crate::context::Context;
use crate::error::ContextError;
use crate::machine::CompressionMachine;
use crate::protocol::{CompressionState, CompressionTarget};

pub use crate::memory::InMemorySectionStore;

/// Scripted `Context` implementation with deterministic evaluation.
pub struct MockContext {
    machine: CompressionMachine,
    messages: Vec<Message>,
    auto: bool,
    last_usage: Option<TokenUsage>,
}

impl MockContext {
    /// Create a mock paradigm. With `auto` set, `evaluate_compression` always
    /// produces a `Section` target; otherwise it never triggers.
    pub fn new(auto: bool) -> Self {
        Self {
            machine: CompressionMachine::new(),
            messages: Vec::new(),
            auto,
            last_usage: None,
        }
    }

    /// Current dialogue history.
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// Most recent turn usage recorded by `apply_model_output`.
    pub fn last_usage(&self) -> Option<&TokenUsage> {
        self.last_usage.as_ref()
    }

    /// Current compression pipeline state (paradigm observation API, spec
    /// §7.1.6).
    pub fn compression_state(&self) -> CompressionState {
        self.machine.state()
    }
}

impl Context for MockContext {
    fn push_user_input(&mut self, message: Message) -> Result<(), ContextError> {
        self.machine.require_idle("push_user_input")?;
        self.messages.push(message);
        Ok(())
    }

    fn apply_model_output(
        &mut self,
        message: Message,
        usage: TokenUsage,
    ) -> Result<(), ContextError> {
        self.machine.require_idle("apply_model_output")?;
        self.last_usage = Some(usage);
        self.messages.push(message);
        Ok(())
    }

    fn export(&self) -> Result<Vec<Message>, ContextError> {
        if self.messages.is_empty() {
            return Err(ContextError::InvalidRequest {
                message: "exported message sequence is empty".to_owned(),
            });
        }
        Ok(self.messages.clone())
    }

    fn evaluate_compression(&mut self) -> Result<Option<CompressionTarget>, ContextError> {
        let target = if self.auto {
            Some(CompressionTarget::Section {
                id: self.messages.len() as u64,
            })
        } else {
            None
        };
        self.machine.on_evaluate(target.clone())?;
        Ok(target)
    }

    fn prepare_compression(
        &mut self,
        target: CompressionTarget,
    ) -> Result<Vec<Message>, ContextError> {
        self.machine.on_prepare(&target)?;
        Ok(self.messages.clone())
    }

    fn apply_compression_result(&mut self, summary: Vec<Message>) -> Result<(), ContextError> {
        self.machine.on_apply()?;
        self.messages = summary;
        Ok(())
    }

    fn abandon_compression(&mut self) -> Result<(), ContextError> {
        self.machine.on_abandon()
    }
}
