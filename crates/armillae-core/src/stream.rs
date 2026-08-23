use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{CompletionResponse, ProviderData, TokenUsage, ToolCall, ToolCallId};

/// A semantic event in one streaming model response.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum CompletionEvent {
    ResponseStarted {
        id: Option<String>,
        model: Option<String>,
    },
    ContentStarted {
        index: usize,
        kind: ContentKind,
    },
    TextDelta {
        index: usize,
        text: String,
    },
    ToolCallStarted {
        index: usize,
        id: ToolCallId,
        name: Option<String>,
    },
    ToolCallArgumentsDelta {
        index: usize,
        fragment: String,
    },
    ToolCallCompleted {
        index: usize,
        call: ToolCall,
    },
    ContentCompleted {
        index: usize,
    },
    Usage {
        usage: TokenUsage,
    },
    ProviderEvent {
        data: ProviderData,
    },
    ResponseCompleted {
        response: CompletionResponse,
    },
}

/// The normalized kind of a streaming content block.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContentKind {
    Text,
    ToolCall,
    ProviderData,
}
