use std::{collections::BTreeMap, fmt};

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Value;

use crate::{ContentPart, Message, TextContent, TokenUsage, ToolCall};

/// A complete, single model-call request.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<crate::ToolDefinition>,
    pub tool_choice: Option<crate::ToolChoice>,
    pub output_format: Option<OutputFormat>,
    pub generation: GenerationOptions,
    pub extensions: ProviderExtensions,
}

/// A requested output representation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum OutputFormat {
    Text,
    JsonObject,
    JsonSchema {
        name: String,
        schema: Value,
        strict: bool,
    },
}

/// Generation controls with cross-provider meaning.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GenerationOptions {
    pub temperature: Option<f64>,
    pub max_output_tokens: Option<u64>,
    pub stop: Vec<String>,
    pub seed: Option<u64>,
}

/// Namespaced request values understood by a specific Adapter.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderExtensions {
    pub values: BTreeMap<String, Value>,
}

impl ProviderExtensions {
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// The normalized result of one model call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompletionResponse {
    pub id: Option<String>,
    pub model: Option<String>,
    pub content: Vec<AssistantContent>,
    /// The Provider-reported reason, or `None` when the Provider did not report one.
    pub finish_reason: Option<FinishReason>,
    pub usage: Option<TokenUsage>,
    pub provider_metadata: Value,
}

impl CompletionResponse {
    pub fn as_assistant_message(&self) -> Message {
        Message::assistant(
            self.content
                .iter()
                .cloned()
                .map(ContentPart::from)
                .collect(),
        )
    }

    pub fn tool_calls(&self) -> impl Iterator<Item = &ToolCall> {
        self.content.iter().filter_map(|item| match item {
            AssistantContent::ToolCall(call) => Some(call),
            AssistantContent::Text(_) | AssistantContent::ProviderData(_) => None,
        })
    }
}

/// An ordered item emitted by the assistant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AssistantContent {
    Text(TextContent),
    ToolCall(ToolCall),
    ProviderData(ProviderData),
}

impl From<AssistantContent> for ContentPart {
    fn from(value: AssistantContent) -> Self {
        match value {
            AssistantContent::Text(text) => Self::Text(text),
            AssistantContent::ToolCall(call) => Self::ToolCall(call),
            AssistantContent::ProviderData(data) => Self::ProviderData(data),
        }
    }
}

/// Why the Provider ended generation.
#[derive(Clone, Debug, PartialEq, Eq, JsonSchema)]
#[schemars(with = "String")]
#[non_exhaustive]
pub enum FinishReason {
    Stop,
    Length,
    ToolCall,
    ContentFilter,
    Cancelled,
    Unknown(String),
}

impl FinishReason {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Stop => "stop",
            Self::Length => "length",
            Self::ToolCall => "tool_call",
            Self::ContentFilter => "content_filter",
            Self::Cancelled => "cancelled",
            Self::Unknown(value) => value,
        }
    }
}

impl Serialize for FinishReason {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for FinishReason {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_string(FinishReasonVisitor)
    }
}

struct FinishReasonVisitor;

impl de::Visitor<'_> for FinishReasonVisitor {
    type Value = FinishReason;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a finish reason string")
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(match value.as_str() {
            "stop" => FinishReason::Stop,
            "length" => FinishReason::Length,
            "tool_call" => FinishReason::ToolCall,
            "content_filter" => FinishReason::ContentFilter,
            "cancelled" => FinishReason::Cancelled,
            _ => FinishReason::Unknown(value),
        })
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_string(value.to_owned())
    }
}

/// Provider-specific data that has no normalized representation yet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderData {
    pub provider: String,
    pub kind: String,
    pub value: Value,
}
