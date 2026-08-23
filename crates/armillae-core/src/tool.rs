use std::{borrow::Borrow, error::Error, fmt};

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::Value;

/// A tool made available to a model call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// A non-empty identifier correlating a tool call with its result.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, JsonSchema)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct ToolCallId(#[schemars(length(min = 1))] String);

impl ToolCallId {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidToolCallId> {
        let value = value.into();
        if value.is_empty() {
            return Err(InvalidToolCallId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl<'de> Deserialize<'de> for ToolCallId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(de::Error::custom)
    }
}

impl TryFrom<String> for ToolCallId {
    type Error = InvalidToolCallId;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&str> for ToolCallId {
    type Error = InvalidToolCallId;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ToolCallId> for String {
    fn from(value: ToolCallId) -> Self {
        value.into_inner()
    }
}

impl AsRef<str> for ToolCallId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for ToolCallId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for ToolCallId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Returned when a tool call identifier is empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidToolCallId;

impl fmt::Display for InvalidToolCallId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("tool call ID must not be empty")
    }
}

impl Error for InvalidToolCallId {}

/// A structured tool invocation requested by a model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: String,
    pub arguments: Value,
}

/// The result associated with one tool call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolResult {
    pub call_id: ToolCallId,
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
