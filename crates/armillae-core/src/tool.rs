use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A tool made available to a model call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// A structured tool invocation requested by a model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// The result associated with one tool call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolResult {
    pub call_id: String,
    pub content: Vec<ToolResultContent>,
    pub is_error: bool,
}

/// An ordered content item in a tool result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolResultContent {
    Text { text: String },
    Json { value: Value },
}

/// The model's permitted tool-selection behavior.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolChoice {
    Auto,
    None,
    Required,
    Specific { name: String },
}
