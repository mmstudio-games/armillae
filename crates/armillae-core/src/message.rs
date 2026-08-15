use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ProviderData, ToolCall, ToolResult};

/// A role-bearing message in model-call history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentPart>,
}

impl Message {
    pub fn new(role: Role, content: Vec<ContentPart>) -> Self {
        Self { role, content }
    }

    pub fn user(text: impl Into<String>) -> Self {
        Self::new(Role::User, vec![ContentPart::text(text)])
    }

    pub fn assistant(content: Vec<ContentPart>) -> Self {
        Self::new(Role::Assistant, content)
    }

    pub fn tool_result(result: ToolResult) -> Self {
        Self::new(Role::Tool, vec![ContentPart::ToolResult(result)])
    }
}

/// The author or protocol role of a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Role {
    System,
    Developer,
    User,
    Assistant,
    Tool,
}

/// An ordered content item within a message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContentPart {
    Text(TextContent),
    ToolCall(ToolCall),
    ToolResult(ToolResult),
    ProviderData(ProviderData),
}

impl ContentPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(TextContent::new(text))
    }
}

/// Plain text content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TextContent {
    pub text: String,
}

impl TextContent {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}
