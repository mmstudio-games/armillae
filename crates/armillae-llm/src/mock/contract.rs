use std::collections::BTreeMap;

use armillae_core::{
    AssistantContent, CompletionEvent, CompletionRequest, CompletionResponse, ContentKind, ToolCall,
};
use futures_util::StreamExt;
use thiserror::Error;

use crate::{BridgeError, LlmBridge};

/// A safe failure from a reusable Bridge contract check.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum BridgeContractError {
    #[error("Bridge operation failed during contract verification")]
    BridgeFailure {
        operation: &'static str,
        error: BridgeError,
    },

    #[error("Bridge contract event sequence is invalid")]
    EventSequence {
        rule: &'static str,
        index: Option<usize>,
    },

    #[error("Bridge response does not match the contract fixture")]
    ResponseMismatch,
}

/// Verifies one deterministic non-streaming contract fixture.
pub async fn verify_completion(
    bridge: &dyn LlmBridge,
    request: CompletionRequest,
    expected: &CompletionResponse,
) -> Result<(), BridgeContractError> {
    let response =
        bridge
            .complete(request)
            .await
            .map_err(|error| BridgeContractError::BridgeFailure {
                operation: "complete",
                error,
            })?;
    if response != *expected {
        return Err(BridgeContractError::ResponseMismatch);
    }
    Ok(())
}

/// Collects and verifies one deterministic streaming contract fixture.
pub async fn verify_stream(
    bridge: &dyn LlmBridge,
    request: CompletionRequest,
    expected: &CompletionResponse,
) -> Result<Vec<CompletionEvent>, BridgeContractError> {
    let mut stream =
        bridge
            .stream(request)
            .await
            .map_err(|error| BridgeContractError::BridgeFailure {
                operation: "stream",
                error,
            })?;
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event.map_err(|error| BridgeContractError::BridgeFailure {
            operation: "stream item",
            error,
        })?);
    }

    let response = validate_stream_events(&events)?;
    if response != expected {
        return Err(BridgeContractError::ResponseMismatch);
    }
    Ok(events)
}

/// Validates semantic ordering and returns the unique terminal response.
pub fn validate_stream_events(
    events: &[CompletionEvent],
) -> Result<&CompletionResponse, BridgeContractError> {
    let mut response_started = false;
    let mut terminal_response = None;
    let mut contents = BTreeMap::new();

    for (position, event) in events.iter().enumerate() {
        if terminal_response.is_some() {
            return violation("events must not follow ResponseCompleted", None);
        }

        match event {
            CompletionEvent::ResponseStarted { .. } => {
                if position != 0 || response_started {
                    return violation("ResponseStarted must occur exactly once and first", None);
                }
                response_started = true;
            }
            _ if !response_started => {
                return violation("ResponseStarted must occur before all other events", None);
            }
            CompletionEvent::ContentStarted { index, kind } => {
                if contents.contains_key(index) {
                    return violation("a content index must start exactly once", Some(*index));
                }
                let content = match kind {
                    ContentKind::Text => ObservedContent::Text {
                        text: String::new(),
                    },
                    ContentKind::ToolCall => ObservedContent::ToolCall {
                        id: None,
                        name: None,
                        fragments: String::new(),
                        completed: None,
                    },
                    ContentKind::ProviderData => ObservedContent::ProviderData,
                    _ => return violation("unknown content kinds must be rejected", Some(*index)),
                };
                contents.insert(
                    *index,
                    ObservedBlock {
                        content,
                        completed: false,
                    },
                );
            }
            CompletionEvent::TextDelta { index, text } => {
                let block = active_block(&mut contents, *index)?;
                match &mut block.content {
                    ObservedContent::Text { text: observed } => observed.push_str(text),
                    _ => return violation("TextDelta requires active text content", Some(*index)),
                }
            }
            CompletionEvent::ToolCallStarted { index, id, name } => {
                let block = active_block(&mut contents, *index)?;
                match &mut block.content {
                    ObservedContent::ToolCall {
                        id: observed_id,
                        name: observed_name,
                        ..
                    } if observed_id.is_none() => {
                        *observed_id = Some(id.clone());
                        *observed_name = name.clone();
                    }
                    ObservedContent::ToolCall { .. } => {
                        return violation("a ToolCall index must start exactly once", Some(*index));
                    }
                    _ => {
                        return violation(
                            "ToolCallStarted requires active ToolCall content",
                            Some(*index),
                        );
                    }
                }
            }
            CompletionEvent::ToolCallArgumentsDelta { index, fragment } => {
                let block = active_block(&mut contents, *index)?;
                match &mut block.content {
                    ObservedContent::ToolCall { id, fragments, .. } if id.is_some() => {
                        fragments.push_str(fragment);
                    }
                    _ => {
                        return violation(
                            "ToolCall arguments require a started ToolCall",
                            Some(*index),
                        );
                    }
                }
            }
            CompletionEvent::ToolCallCompleted { index, call } => {
                let block = active_block(&mut contents, *index)?;
                match &mut block.content {
                    ObservedContent::ToolCall {
                        id,
                        name,
                        fragments,
                        completed,
                    } => {
                        if completed.is_some() || id.as_deref() != Some(call.id.as_str()) {
                            return violation(
                                "ToolCallCompleted must match one started ToolCall",
                                Some(*index),
                            );
                        }
                        if name
                            .as_deref()
                            .is_some_and(|name| name != call.name.as_str())
                        {
                            return violation("ToolCall name must remain stable", Some(*index));
                        }
                        if !fragments.is_empty() {
                            let arguments: serde_json::Value = serde_json::from_str(fragments)
                                .map_err(|_| BridgeContractError::EventSequence {
                                    rule: "ToolCall argument fragments must form valid JSON",
                                    index: Some(*index),
                                })?;
                            if arguments != call.arguments {
                                return violation(
                                    "ToolCall argument fragments must match the completed call",
                                    Some(*index),
                                );
                            }
                        }
                        *completed = Some(call.clone());
                    }
                    _ => {
                        return violation(
                            "ToolCallCompleted requires active ToolCall content",
                            Some(*index),
                        );
                    }
                }
            }
            CompletionEvent::ContentCompleted { index } => {
                let block = active_block(&mut contents, *index)?;
                if matches!(
                    block.content,
                    ObservedContent::ToolCall {
                        completed: None,
                        ..
                    }
                ) {
                    return violation(
                        "ToolCall content must complete its call before its content",
                        Some(*index),
                    );
                }
                block.completed = true;
            }
            CompletionEvent::Usage { .. } | CompletionEvent::ProviderEvent { .. } => {}
            CompletionEvent::ResponseCompleted { response } => {
                if position + 1 != events.len() {
                    return violation("ResponseCompleted must be the final event", None);
                }
                validate_final_content(&contents, response)?;
                terminal_response = Some(response);
            }
            _ => return violation("unknown completion events must be rejected", None),
        }
    }

    if !response_started {
        return violation("the stream must contain ResponseStarted", None);
    }
    terminal_response.ok_or(BridgeContractError::EventSequence {
        rule: "the stream must contain one ResponseCompleted",
        index: None,
    })
}

struct ObservedBlock {
    content: ObservedContent,
    completed: bool,
}

enum ObservedContent {
    Text {
        text: String,
    },
    ToolCall {
        id: Option<String>,
        name: Option<String>,
        fragments: String,
        completed: Option<ToolCall>,
    },
    ProviderData,
}

fn active_block(
    contents: &mut BTreeMap<usize, ObservedBlock>,
    index: usize,
) -> Result<&mut ObservedBlock, BridgeContractError> {
    let block = contents
        .get_mut(&index)
        .ok_or(BridgeContractError::EventSequence {
            rule: "content events require a started content index",
            index: Some(index),
        })?;
    if block.completed {
        return violation(
            "completed content must not receive more events",
            Some(index),
        );
    }
    Ok(block)
}

fn validate_final_content(
    contents: &BTreeMap<usize, ObservedBlock>,
    response: &CompletionResponse,
) -> Result<(), BridgeContractError> {
    if contents.len() != response.content.len() {
        return violation(
            "stream content indexes must match terminal response content",
            None,
        );
    }

    for (index, expected) in response.content.iter().enumerate() {
        let observed = contents
            .get(&index)
            .ok_or(BridgeContractError::EventSequence {
                rule: "stream content indexes must be contiguous",
                index: Some(index),
            })?;
        if !observed.completed {
            return violation(
                "all content must complete before ResponseCompleted",
                Some(index),
            );
        }
        let matches = match (&observed.content, expected) {
            (ObservedContent::Text { text }, AssistantContent::Text(expected)) => {
                text == &expected.text
            }
            (
                ObservedContent::ToolCall {
                    completed: Some(call),
                    ..
                },
                AssistantContent::ToolCall(expected),
            ) => call == expected,
            (ObservedContent::ProviderData, AssistantContent::ProviderData(_)) => true,
            _ => false,
        };
        if !matches {
            return violation(
                "stream deltas must match terminal response content",
                Some(index),
            );
        }
    }
    Ok(())
}

fn violation<T>(rule: &'static str, index: Option<usize>) -> Result<T, BridgeContractError> {
    Err(BridgeContractError::EventSequence { rule, index })
}
