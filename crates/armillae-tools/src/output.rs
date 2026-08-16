use std::fmt;

use armillae_core::ToolResultContent;
use serde::Serialize;

use crate::ToolExecutionError;

/// A normalized, ordered collection of model-visible Tool result content.
#[derive(Clone, PartialEq)]
pub struct ToolOutput {
    pub content: Vec<ToolResultContent>,
}

impl ToolOutput {
    pub fn new(content: Vec<ToolResultContent>) -> Self {
        Self { content }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::new(vec![ToolResultContent::Text { text: text.into() }])
    }

    pub fn json(value: serde_json::Value) -> Self {
        Self::new(vec![ToolResultContent::Json { value }])
    }
}

impl fmt::Debug for ToolOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let content_kinds = self
            .content
            .iter()
            .map(|content| match content {
                ToolResultContent::Text { .. } => "text",
                ToolResultContent::Json { .. } => "json",
                _ => "unknown",
            })
            .collect::<Vec<_>>();

        formatter
            .debug_struct("ToolOutput")
            .field("content_count", &self.content.len())
            .field("content_kinds", &content_kinds)
            .finish()
    }
}

/// Conversion from an author-facing return value into canonical Tool output.
pub trait IntoToolOutput {
    fn into_tool_output(self) -> Result<ToolOutput, ToolExecutionError>;
}

impl<T> IntoToolOutput for T
where
    T: Serialize,
{
    fn into_tool_output(self) -> Result<ToolOutput, ToolExecutionError> {
        serde_json::to_value(self)
            .map(ToolOutput::json)
            .map_err(|error| ToolExecutionError::OutputSerialization {
                message: error.to_string(),
            })
    }
}

impl IntoToolOutput for ToolOutput {
    fn into_tool_output(self) -> Result<ToolOutput, ToolExecutionError> {
        Ok(self)
    }
}
