use thiserror::Error;

/// A structured failure while executing one ToolCall.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolExecutionError {
    #[error("unknown tool: {name}")]
    UnknownTool { name: String },

    #[error("invalid arguments for tool {name}: {message}")]
    InvalidArguments { name: String, message: String },

    #[error("tool {name} failed: {message}")]
    ExecutionFailed { name: String, message: String },

    #[error("tool output serialization failed: {message}")]
    OutputSerialization { message: String },
}

/// A structured failure while building or changing a ToolRegistry.
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolRegistryError {
    #[error("duplicate tool: {name}")]
    DuplicateTool { name: String },
}
