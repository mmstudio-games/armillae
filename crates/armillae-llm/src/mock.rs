use std::{
    collections::VecDeque,
    fmt,
    sync::{Mutex, MutexGuard},
};

use armillae_core::{
    AssistantContent, CompletionEvent, CompletionRequest, CompletionResponse, ContentKind,
    FinishReason, TextContent, ToolCall, ToolCallId,
};
use futures_util::stream;
use serde_json::Value;

use crate::{
    BoxFuture, BridgeCapabilities, BridgeError, CompletionStream, ErrorMetadata, LlmBridge,
};

pub mod contract;

/// One deterministic result consumed by a [`MockBridge`] call.
#[derive(Clone)]
#[non_exhaustive]
pub enum MockResponse {
    Completion(CompletionResponse),
    Stream(Vec<Result<CompletionEvent, BridgeError>>),
    Error(BridgeError),
}

impl MockResponse {
    pub fn completion(response: CompletionResponse) -> Self {
        Self::Completion(response)
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self::Completion(text_response(text.into()))
    }

    pub fn tool_call(id: ToolCallId, name: impl Into<String>, arguments: Value) -> Self {
        Self::Completion(tool_call_response(ToolCall {
            id,
            name: name.into(),
            arguments,
        }))
    }

    pub fn error(error: BridgeError) -> Self {
        Self::Error(error)
    }

    pub fn stream(events: impl IntoIterator<Item = Result<CompletionEvent, BridgeError>>) -> Self {
        Self::Stream(events.into_iter().collect())
    }

    /// Builds a valid single-content text stream with one completion event.
    pub fn text_stream<I, S>(chunks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let chunks: Vec<String> = chunks.into_iter().map(Into::into).collect();
        let text = chunks.concat();
        let mut events = Vec::with_capacity(chunks.len() + 4);
        events.push(Ok(CompletionEvent::ResponseStarted {
            id: None,
            model: None,
        }));
        events.push(Ok(CompletionEvent::ContentStarted {
            index: 0,
            kind: ContentKind::Text,
        }));
        events.extend(
            chunks
                .into_iter()
                .map(|text| Ok(CompletionEvent::TextDelta { index: 0, text })),
        );
        events.push(Ok(CompletionEvent::ContentCompleted { index: 0 }));
        events.push(Ok(CompletionEvent::ResponseCompleted {
            response: text_response(text),
        }));
        Self::Stream(events)
    }

    /// Builds a valid ToolCall stream from arbitrary JSON argument fragments.
    pub fn tool_call_stream<I, S>(
        id: ToolCallId,
        name: impl Into<String>,
        argument_fragments: I,
    ) -> Result<Self, BridgeError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let name = name.into();
        let fragments: Vec<String> = argument_fragments.into_iter().map(Into::into).collect();
        let arguments = serde_json::from_str(&fragments.concat()).map_err(|_| {
            BridgeError::InvalidConfiguration {
                message: "Mock ToolCall argument fragments must form valid JSON".to_owned(),
            }
        })?;
        let call = ToolCall {
            id: id.clone(),
            name: name.clone(),
            arguments,
        };
        let mut events = Vec::with_capacity(fragments.len() + 6);
        events.push(Ok(CompletionEvent::ResponseStarted {
            id: None,
            model: None,
        }));
        events.push(Ok(CompletionEvent::ContentStarted {
            index: 0,
            kind: ContentKind::ToolCall,
        }));
        events.push(Ok(CompletionEvent::ToolCallStarted {
            index: 0,
            id,
            name: Some(name),
        }));
        events.extend(
            fragments
                .into_iter()
                .map(|fragment| Ok(CompletionEvent::ToolCallArgumentsDelta { index: 0, fragment })),
        );
        events.push(Ok(CompletionEvent::ToolCallCompleted {
            index: 0,
            call: call.clone(),
        }));
        events.push(Ok(CompletionEvent::ContentCompleted { index: 0 }));
        events.push(Ok(CompletionEvent::ResponseCompleted {
            response: tool_call_response(call),
        }));
        Ok(Self::Stream(events))
    }

    /// Appends a normalized interruption after the supplied semantic events.
    pub fn interrupted_stream(
        events: impl IntoIterator<Item = CompletionEvent>,
        metadata: ErrorMetadata,
    ) -> Self {
        let mut events: Vec<Result<CompletionEvent, BridgeError>> =
            events.into_iter().map(Ok).collect();
        events.push(Err(BridgeError::StreamInterrupted { metadata }));
        Self::Stream(events)
    }
}

impl fmt::Debug for MockResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::Completion(_) => "completion",
            Self::Stream(_) => "stream",
            Self::Error(_) => "error",
        };
        formatter
            .debug_struct("MockResponse")
            .field("kind", &kind)
            .finish_non_exhaustive()
    }
}

/// A deterministic, runtime-independent Bridge for downstream tests.
pub struct MockBridge {
    capabilities: BridgeCapabilities,
    plan: MockPlan,
    requests: Mutex<Vec<CompletionRequest>>,
}

enum MockPlan {
    Fixed(MockResponse),
    Scripted(Mutex<VecDeque<MockResponse>>),
}

impl MockBridge {
    pub fn fixed(response: MockResponse) -> Self {
        Self {
            capabilities: BridgeCapabilities::all(),
            plan: MockPlan::Fixed(response),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn scripted(responses: impl IntoIterator<Item = MockResponse>) -> Self {
        Self {
            capabilities: BridgeCapabilities::all(),
            plan: MockPlan::Scripted(Mutex::new(responses.into_iter().collect())),
            requests: Mutex::new(Vec::new()),
        }
    }

    pub fn with_capabilities(
        mut self,
        capabilities: BridgeCapabilities,
    ) -> Result<Self, BridgeError> {
        capabilities.validate()?;
        self.capabilities = capabilities;
        Ok(self)
    }

    /// Returns a snapshot of every request received by `complete` or `stream`.
    pub fn requests(&self) -> Result<Vec<CompletionRequest>, BridgeError> {
        Ok(lock(&self.requests)?.clone())
    }

    /// Removes and returns every recorded request.
    pub fn take_requests(&self) -> Result<Vec<CompletionRequest>, BridgeError> {
        Ok(std::mem::take(&mut *lock(&self.requests)?))
    }

    pub fn remaining_scripted_responses(&self) -> Result<Option<usize>, BridgeError> {
        match &self.plan {
            MockPlan::Fixed(_) => Ok(None),
            MockPlan::Scripted(responses) => Ok(Some(lock(responses)?.len())),
        }
    }

    fn record_request(&self, request: &CompletionRequest) -> Result<(), BridgeError> {
        lock(&self.requests)?.push(request.clone());
        Ok(())
    }

    fn next_response(&self) -> Result<MockResponse, BridgeError> {
        match &self.plan {
            MockPlan::Fixed(response) => Ok(response.clone()),
            MockPlan::Scripted(responses) => {
                lock(responses)?
                    .pop_front()
                    .ok_or_else(|| BridgeError::InvalidRequest {
                        message: "MockBridge script is exhausted".to_owned(),
                    })
            }
        }
    }
}

impl LlmBridge for MockBridge {
    fn capabilities(&self) -> BridgeCapabilities {
        self.capabilities
    }

    fn complete<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionResponse, BridgeError>> {
        Box::pin(async move {
            self.record_request(&request)?;
            self.capabilities.validate_request(&request)?;
            match self.next_response()? {
                MockResponse::Completion(response) => Ok(response),
                MockResponse::Error(error) => Err(error),
                MockResponse::Stream(_) => Err(BridgeError::InvalidRequest {
                    message: "MockBridge complete call received a Stream script item".to_owned(),
                }),
            }
        })
    }

    fn stream<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionStream, BridgeError>> {
        Box::pin(async move {
            self.record_request(&request)?;
            self.capabilities.validate_streaming_request(&request)?;
            match self.next_response()? {
                MockResponse::Stream(events) => {
                    Ok(Box::pin(stream::iter(events)) as CompletionStream)
                }
                MockResponse::Error(error) => Err(error),
                MockResponse::Completion(_) => Err(BridgeError::InvalidRequest {
                    message: "MockBridge stream call received a Completion script item".to_owned(),
                }),
            }
        })
    }
}

impl fmt::Debug for MockBridge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let plan = match &self.plan {
            MockPlan::Fixed(_) => "fixed",
            MockPlan::Scripted(_) => "scripted",
        };
        formatter
            .debug_struct("MockBridge")
            .field("capabilities", &self.capabilities)
            .field("plan", &plan)
            .field("requests", &"[omitted]")
            .finish()
    }
}

fn text_response(text: String) -> CompletionResponse {
    CompletionResponse {
        id: None,
        model: None,
        content: vec![AssistantContent::Text(TextContent::new(text))],
        finish_reason: Some(FinishReason::Stop),
        usage: None,
        provider_metadata: Value::Null,
    }
}

fn tool_call_response(call: ToolCall) -> CompletionResponse {
    CompletionResponse {
        id: None,
        model: None,
        content: vec![AssistantContent::ToolCall(call)],
        finish_reason: Some(FinishReason::ToolCall),
        usage: None,
        provider_metadata: Value::Null,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, BridgeError> {
    mutex.lock().map_err(|_| BridgeError::InvalidConfiguration {
        message: "MockBridge internal state lock is poisoned".to_owned(),
    })
}
