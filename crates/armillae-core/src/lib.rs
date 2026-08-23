//! Provider-independent protocols shared by Armillae components.

mod completion;
mod message;
mod stream;
mod tool;
mod usage;

pub use completion::{
    AssistantContent, CompletionRequest, CompletionResponse, FinishReason, GenerationOptions,
    OutputFormat, ProviderData, ProviderExtensions,
};
pub use message::{ContentPart, Message, Role, TextContent};
pub use stream::{CompletionEvent, ContentKind};
pub use tool::{
    InvalidToolCallId, ToolCall, ToolCallId, ToolChoice, ToolDefinition, ToolResult,
    ToolResultContent,
};
pub use usage::TokenUsage;
