use armillae_core::{ToolCall, ToolDefinition, ToolResult};

use crate::{BoxFuture, ToolContext, ToolExecutionError};

/// Executes exactly one ToolCall without invoking an LLM Bridge.
pub trait ToolExecutor: Send + Sync {
    fn definitions(&self) -> Vec<ToolDefinition>;

    fn execute<'a>(
        &'a self,
        context: ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolResult, ToolExecutionError>>;
}
