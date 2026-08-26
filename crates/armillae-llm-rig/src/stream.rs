use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};

use armillae_core::{
    AssistantContent, CompletionEvent, CompletionResponse, ContentKind, ProviderData, TextContent,
    TokenUsage, ToolCall, ToolCallId,
};
use armillae_llm::{BridgeError, CompletionStream, ErrorMetadata};
use futures_util::{StreamExt, stream};
use rig_core::{
    completion::{CompletionError, GetTokenUsage},
    message::{Reasoning, ToolCall as RigToolCall},
    streaming::{StreamedAssistantContent, StreamingCompletionResponse, ToolCallDeltaContent},
};
use serde_json::{Map, Value, json};

use crate::{convert, response::RigStreamingResponseNormalizer};

#[cfg(test)]
use crate::response::NoopStreamingResponseNormalizer;

#[cfg(test)]
pub(crate) fn completion_stream<R>(
    stream: StreamingCompletionResponse<R>,
    provider: impl Into<String>,
) -> CompletionStream
where
    R: Clone + Unpin + GetTokenUsage + Send + 'static,
{
    completion_stream_with_normalizer(stream, provider, Arc::new(NoopStreamingResponseNormalizer))
}

pub(crate) fn completion_stream_with_normalizer<R>(
    stream: StreamingCompletionResponse<R>,
    provider: impl Into<String>,
    normalizer: Arc<dyn RigStreamingResponseNormalizer<R>>,
) -> CompletionStream
where
    R: Clone + Unpin + GetTokenUsage + Send + 'static,
{
    let state = StreamState::new(stream, provider.into(), normalizer);
    Box::pin(stream::unfold(state, |mut state| async move {
        state.next_output().await.map(|output| (output, state))
    }))
}

struct StreamState<R>
where
    R: Clone + Unpin + GetTokenUsage,
{
    stream: StreamingCompletionResponse<R>,
    normalizer: Arc<dyn RigStreamingResponseNormalizer<R>>,
    provider: String,
    pending: VecDeque<CompletionEvent>,
    content: BTreeMap<usize, AssistantContent>,
    tools: BTreeMap<String, ToolState>,
    used_tool_ids: BTreeSet<String>,
    active_text: Option<TextState>,
    active_reasoning: Option<ReasoningState>,
    next_content_index: usize,
    usage: Option<TokenUsage>,
    finish_reason: Option<armillae_core::FinishReason>,
    provider_metadata: Value,
    final_received: bool,
    finished: bool,
    failed: bool,
}

impl<R> StreamState<R>
where
    R: Clone + Unpin + GetTokenUsage,
{
    fn new(
        stream: StreamingCompletionResponse<R>,
        provider: String,
        normalizer: Arc<dyn RigStreamingResponseNormalizer<R>>,
    ) -> Self {
        Self {
            stream,
            normalizer,
            provider,
            pending: VecDeque::from([CompletionEvent::ResponseStarted {
                id: None,
                model: None,
            }]),
            content: BTreeMap::new(),
            tools: BTreeMap::new(),
            used_tool_ids: BTreeSet::new(),
            active_text: None,
            active_reasoning: None,
            next_content_index: 0,
            usage: None,
            finish_reason: None,
            provider_metadata: Value::Object(Map::new()),
            final_received: false,
            finished: false,
            failed: false,
        }
    }

    async fn next_output(&mut self) -> Option<Result<CompletionEvent, BridgeError>> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some(Ok(event));
            }
            if self.finished || self.failed {
                return None;
            }

            match self.stream.next().await {
                Some(Ok(item)) => {
                    if self.handle_item(item).is_err() {
                        return Some(Err(self.interrupted()));
                    }
                }
                Some(Err(error)) => return Some(Err(self.interrupted_with_error(error))),
                None => {
                    if self.finish_stream().is_err() {
                        return Some(Err(self.interrupted()));
                    }
                }
            }
        }
    }

    fn handle_item(&mut self, item: StreamedAssistantContent<R>) -> Result<(), ()> {
        if self.final_received {
            return Err(());
        }

        match item {
            StreamedAssistantContent::Text(text) => {
                self.close_reasoning()?;
                self.push_text(text.text);
                if let Some(value) = text.additional_params {
                    self.pending.push_back(CompletionEvent::ProviderEvent {
                        data: self.provider_data("text_metadata", value),
                    });
                }
            }
            StreamedAssistantContent::ToolCallDelta {
                id,
                internal_call_id,
                content,
            } => {
                self.close_text();
                self.close_reasoning()?;
                self.push_tool_delta(internal_call_id, id, content)?;
            }
            StreamedAssistantContent::ToolCall {
                tool_call,
                internal_call_id,
            } => {
                self.close_text();
                self.close_reasoning()?;
                self.complete_tool(internal_call_id, tool_call)?;
            }
            StreamedAssistantContent::Reasoning(reasoning) => {
                self.close_text();
                self.complete_reasoning(reasoning)?;
            }
            StreamedAssistantContent::ReasoningDelta { id, reasoning } => {
                self.close_text();
                self.push_reasoning_delta(id, reasoning)?;
            }
            StreamedAssistantContent::Final(response) => {
                self.close_text();
                self.close_reasoning()?;
                self.usage = convert::usage_from_rig(response.token_usage());
                let facts = self.normalizer.normalize(&response)?;
                self.finish_reason = facts.finish_reason;
                self.provider_metadata = facts.provider_metadata;
                self.final_received = true;
            }
            StreamedAssistantContent::Unknown(value) => {
                self.pending.push_back(CompletionEvent::ProviderEvent {
                    data: self.provider_data("unknown_stream_item", value),
                });
            }
        }

        Ok(())
    }

    fn push_text(&mut self, text: String) {
        let index = match &self.active_text {
            Some(active) => active.index,
            None => {
                let index = self.allocate_content(ContentKind::Text);
                self.active_text = Some(TextState {
                    index,
                    text: String::new(),
                });
                index
            }
        };
        if let Some(active) = &mut self.active_text {
            active.text.push_str(&text);
        }
        self.pending
            .push_back(CompletionEvent::TextDelta { index, text });
    }

    fn close_text(&mut self) {
        let Some(active) = self.active_text.take() else {
            return;
        };
        self.content.insert(
            active.index,
            AssistantContent::Text(TextContent::new(active.text)),
        );
        self.pending.push_back(CompletionEvent::ContentCompleted {
            index: active.index,
        });
    }

    fn push_reasoning_delta(&mut self, id: Option<String>, reasoning: String) -> Result<(), ()> {
        if self
            .active_reasoning
            .as_ref()
            .is_some_and(|active| active.id != id)
        {
            self.close_reasoning()?;
        }
        if self.active_reasoning.is_none() {
            let index = self.allocate_content(ContentKind::ProviderData);
            self.active_reasoning = Some(ReasoningState {
                index,
                id: id.clone(),
                text: String::new(),
            });
        }
        let active = self.active_reasoning.as_mut().ok_or(())?;
        active.text.push_str(&reasoning);
        self.pending.push_back(CompletionEvent::ProviderEvent {
            data: self.provider_data(
                "reasoning_delta",
                json!({ "id": id, "reasoning": reasoning }),
            ),
        });
        Ok(())
    }

    fn close_reasoning(&mut self) -> Result<(), ()> {
        let Some(active) = self.active_reasoning.take() else {
            return Ok(());
        };
        let reasoning = Reasoning::new(&active.text).optional_id(active.id);
        let value = serde_json::to_value(reasoning).map_err(|_| ())?;
        self.content.insert(
            active.index,
            AssistantContent::ProviderData(self.provider_data("reasoning", value)),
        );
        self.pending.push_back(CompletionEvent::ContentCompleted {
            index: active.index,
        });
        Ok(())
    }

    fn complete_reasoning(&mut self, reasoning: Reasoning) -> Result<(), ()> {
        let value = serde_json::to_value(reasoning).map_err(|_| ())?;
        let index = self.active_reasoning.take().map_or_else(
            || self.allocate_content(ContentKind::ProviderData),
            |active| active.index,
        );
        let data = self.provider_data("reasoning", value);
        self.pending
            .push_back(CompletionEvent::ProviderEvent { data: data.clone() });
        self.content
            .insert(index, AssistantContent::ProviderData(data));
        self.pending
            .push_back(CompletionEvent::ContentCompleted { index });
        Ok(())
    }

    fn push_tool_delta(
        &mut self,
        internal_call_id: String,
        provider_id: String,
        delta: ToolCallDeltaContent,
    ) -> Result<(), ()> {
        self.ensure_tool(&internal_call_id);
        if !provider_id.is_empty() {
            self.observe_tool_id(&internal_call_id, provider_id)?;
        }

        {
            let tool = self.tools.get_mut(&internal_call_id).ok_or(())?;
            if tool.completed {
                return Err(());
            }
            match delta {
                ToolCallDeltaContent::Name(name) => tool.name_fragments.push(name),
                ToolCallDeltaContent::Delta(fragment) => tool.argument_fragments.push(fragment),
            }
        }
        self.start_tool_if_identified(&internal_call_id, None)?;
        self.flush_tool_arguments(&internal_call_id)
    }

    fn complete_tool(
        &mut self,
        internal_call_id: String,
        tool_call: RigToolCall,
    ) -> Result<(), ()> {
        self.ensure_tool(&internal_call_id);
        if !tool_call.id.is_empty() {
            self.observe_tool_id(&internal_call_id, tool_call.id.clone())?;
        }
        if self
            .tools
            .get(&internal_call_id)
            .and_then(|tool| tool.id.as_ref())
            .is_none()
        {
            let generated = self.unique_generated_id(&internal_call_id);
            self.observe_tool_id(&internal_call_id, generated)?;
        }
        self.start_tool_if_identified(&internal_call_id, Some(tool_call.function.name.clone()))?;
        self.flush_tool_arguments(&internal_call_id)?;

        let (index, id, name_fragments, argument_fragments, completed) = {
            let tool = self.tools.get(&internal_call_id).ok_or(())?;
            (
                tool.index,
                tool.id.clone().ok_or(())?,
                tool.name_fragments.concat(),
                tool.argument_fragments.concat(),
                tool.completed,
            )
        };
        if completed
            || (!tool_call.id.is_empty() && tool_call.id != id.as_str())
            || (!name_fragments.is_empty() && name_fragments != tool_call.function.name)
        {
            return Err(());
        }
        if !argument_fragments.is_empty() {
            let arguments: Value = serde_json::from_str(&argument_fragments).map_err(|_| ())?;
            if arguments != tool_call.function.arguments {
                return Err(());
            }
        }

        let call = ToolCall {
            id,
            name: tool_call.function.name,
            arguments: tool_call.function.arguments,
        };
        self.pending.push_back(CompletionEvent::ToolCallCompleted {
            index,
            call: call.clone(),
        });
        self.pending
            .push_back(CompletionEvent::ContentCompleted { index });
        self.content.insert(index, AssistantContent::ToolCall(call));
        if let Some(tool) = self.tools.get_mut(&internal_call_id) {
            tool.completed = true;
        }

        let mut metadata = Map::new();
        if let Some(call_id) = tool_call.call_id {
            metadata.insert("call_id".to_owned(), Value::String(call_id));
        }
        if let Some(signature) = tool_call.signature {
            metadata.insert("signature".to_owned(), Value::String(signature));
        }
        if let Some(additional_params) = tool_call.additional_params {
            metadata.insert("additional_params".to_owned(), additional_params);
        }
        if !metadata.is_empty() {
            self.push_provider_content("tool_call_metadata", Value::Object(metadata));
        }
        Ok(())
    }

    fn ensure_tool(&mut self, internal_call_id: &str) {
        if self.tools.contains_key(internal_call_id) {
            return;
        }
        let index = self.allocate_content(ContentKind::ToolCall);
        self.tools.insert(
            internal_call_id.to_owned(),
            ToolState {
                index,
                id: None,
                name_fragments: Vec::new(),
                argument_fragments: Vec::new(),
                emitted_arguments: 0,
                started: false,
                completed: false,
            },
        );
    }

    fn observe_tool_id(&mut self, internal_call_id: &str, id: String) -> Result<(), ()> {
        let current = self
            .tools
            .get(internal_call_id)
            .and_then(|tool| tool.id.as_ref())
            .map(ToString::to_string);
        if let Some(current) = current {
            return (current == id).then_some(()).ok_or(());
        }
        if self.used_tool_ids.contains(&id) {
            return Err(());
        }
        let id = ToolCallId::new(id).map_err(|_| ())?;
        self.used_tool_ids.insert(id.as_str().to_owned());
        self.tools.get_mut(internal_call_id).ok_or(())?.id = Some(id);
        Ok(())
    }

    fn start_tool_if_identified(
        &mut self,
        internal_call_id: &str,
        complete_name: Option<String>,
    ) -> Result<(), ()> {
        let (index, id, started) = {
            let tool = self.tools.get(internal_call_id).ok_or(())?;
            (tool.index, tool.id.clone(), tool.started)
        };
        if started || id.is_none() {
            return Ok(());
        }
        self.pending.push_back(CompletionEvent::ToolCallStarted {
            index,
            id: id.ok_or(())?,
            name: complete_name,
        });
        self.tools.get_mut(internal_call_id).ok_or(())?.started = true;
        Ok(())
    }

    fn flush_tool_arguments(&mut self, internal_call_id: &str) -> Result<(), ()> {
        let (index, fragments) = {
            let tool = self.tools.get(internal_call_id).ok_or(())?;
            if !tool.started {
                return Ok(());
            }
            (
                tool.index,
                tool.argument_fragments[tool.emitted_arguments..].to_vec(),
            )
        };
        for fragment in &fragments {
            self.pending
                .push_back(CompletionEvent::ToolCallArgumentsDelta {
                    index,
                    fragment: fragment.clone(),
                });
        }
        self.tools
            .get_mut(internal_call_id)
            .ok_or(())?
            .emitted_arguments += fragments.len();
        Ok(())
    }

    fn unique_generated_id(&mut self, internal_call_id: &str) -> String {
        let base = format!("rig-{internal_call_id}");
        if !self.used_tool_ids.contains(&base) {
            return base;
        }
        let mut suffix = 2usize;
        loop {
            let candidate = format!("{base}-{suffix}");
            if !self.used_tool_ids.contains(&candidate) {
                return candidate;
            }
            suffix += 1;
        }
    }

    fn push_provider_content(&mut self, kind: &str, value: Value) {
        let index = self.allocate_content(ContentKind::ProviderData);
        let data = self.provider_data(kind, value);
        self.pending
            .push_back(CompletionEvent::ProviderEvent { data: data.clone() });
        self.content
            .insert(index, AssistantContent::ProviderData(data));
        self.pending
            .push_back(CompletionEvent::ContentCompleted { index });
    }

    fn allocate_content(&mut self, kind: ContentKind) -> usize {
        let index = self.next_content_index;
        self.next_content_index += 1;
        self.pending
            .push_back(CompletionEvent::ContentStarted { index, kind });
        index
    }

    fn provider_data(&self, kind: &str, value: Value) -> ProviderData {
        ProviderData {
            provider: self.provider.clone(),
            kind: kind.to_owned(),
            value,
        }
    }

    fn finish_stream(&mut self) -> Result<(), ()> {
        if !self.final_received || self.tools.values().any(|tool| !tool.completed) {
            return Err(());
        }
        if self.content.len() != self.next_content_index {
            return Err(());
        }

        let content = (0..self.next_content_index)
            .map(|index| self.content.remove(&index).ok_or(()))
            .collect::<Result<Vec<_>, _>>()?;
        if let Some(usage) = &self.usage {
            self.pending.push_back(CompletionEvent::Usage {
                usage: usage.clone(),
            });
        }
        self.pending.push_back(CompletionEvent::ResponseCompleted {
            response: CompletionResponse {
                id: None,
                model: None,
                content,
                finish_reason: self.finish_reason.clone(),
                usage: self.usage.clone(),
                provider_metadata: self.provider_metadata.clone(),
            },
        });
        self.finished = true;
        Ok(())
    }

    fn interrupted(&mut self) -> BridgeError {
        self.interrupted_with_metadata(ErrorMetadata::new(&self.provider))
    }

    fn interrupted_with_error(&mut self, error: CompletionError) -> BridgeError {
        let mut metadata = ErrorMetadata::new(&self.provider);
        if let Some(status) = error.provider_response_status() {
            metadata = metadata.with_http_status(status.as_u16());
        }
        self.interrupted_with_metadata(metadata)
    }

    fn interrupted_with_metadata(&mut self, metadata: ErrorMetadata) -> BridgeError {
        self.pending.clear();
        self.failed = true;
        BridgeError::StreamInterrupted { metadata }
    }
}

struct TextState {
    index: usize,
    text: String,
}

struct ReasoningState {
    index: usize,
    id: Option<String>,
    text: String,
}

struct ToolState {
    index: usize,
    id: Option<ToolCallId>,
    name_fragments: Vec<String>,
    argument_fragments: Vec<String>,
    emitted_arguments: usize,
    started: bool,
    completed: bool,
}

#[cfg(test)]
mod tests {
    use std::{
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        task::{Context, Poll},
    };

    use armillae_core::{AssistantContent, CompletionResponse, TextContent, TokenUsage, ToolCall};
    use armillae_llm::{BridgeError, mock::contract::validate_stream_events};
    use futures::{Stream, StreamExt, executor::block_on, stream};
    use rig_core::{
        completion::{CompletionError, GetTokenUsage, Usage},
        message::ReasoningContent,
        streaming::{
            RawStreamingChoice, RawStreamingToolCall, StreamingCompletionResponse, StreamingResult,
            ToolCallDeltaContent,
        },
    };
    use serde::{Deserialize, Serialize};
    use serde_json::json;

    use super::completion_stream;

    #[derive(Clone, Debug, Deserialize, Serialize)]
    struct ProbeResponse {
        usage: Usage,
    }

    impl GetTokenUsage for ProbeResponse {
        fn token_usage(&self) -> Usage {
            self.usage
        }
    }

    fn tool_call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: id.try_into().expect("test ToolCall IDs must be non-empty"),
            name: name.to_owned(),
            arguments,
        }
    }

    #[test]
    fn interleaved_content_reassembles_with_stable_indexes_and_terminal_usage() {
        block_on(async {
            let usage = Usage {
                input_tokens: 8,
                output_tokens: 5,
                total_tokens: 13,
                cached_input_tokens: 3,
                ..Usage::default()
            };
            let items = vec![
                Ok(RawStreamingChoice::Message("start".to_owned())),
                Ok(RawStreamingChoice::ToolCallDelta {
                    id: "call-weather".to_owned(),
                    internal_call_id: "weather".to_owned(),
                    content: ToolCallDeltaContent::Name("get_".to_owned()),
                }),
                Ok(RawStreamingChoice::ToolCallDelta {
                    id: "call-dice".to_owned(),
                    internal_call_id: "dice".to_owned(),
                    content: ToolCallDeltaContent::Name("roll_dice".to_owned()),
                }),
                Ok(RawStreamingChoice::ToolCallDelta {
                    id: String::new(),
                    internal_call_id: "weather".to_owned(),
                    content: ToolCallDeltaContent::Name("weather".to_owned()),
                }),
                Ok(RawStreamingChoice::ToolCallDelta {
                    id: String::new(),
                    internal_call_id: "weather".to_owned(),
                    content: ToolCallDeltaContent::Delta("{\"city\":\"上".to_owned()),
                }),
                Ok(RawStreamingChoice::ToolCallDelta {
                    id: String::new(),
                    internal_call_id: "dice".to_owned(),
                    content: ToolCallDeltaContent::Delta("{\"sides\":".to_owned()),
                }),
                Ok(RawStreamingChoice::ToolCallDelta {
                    id: String::new(),
                    internal_call_id: "weather".to_owned(),
                    content: ToolCallDeltaContent::Delta("海\"}".to_owned()),
                }),
                Ok(RawStreamingChoice::ToolCallDelta {
                    id: String::new(),
                    internal_call_id: "dice".to_owned(),
                    content: ToolCallDeltaContent::Delta("20}".to_owned()),
                }),
                Ok(RawStreamingChoice::ToolCall(
                    RawStreamingToolCall::new(
                        "call-weather".to_owned(),
                        "get_weather".to_owned(),
                        json!({ "city": "上海" }),
                    )
                    .with_internal_call_id("weather".to_owned()),
                )),
                Ok(RawStreamingChoice::ToolCall(
                    RawStreamingToolCall::new(
                        "call-dice".to_owned(),
                        "roll_dice".to_owned(),
                        json!({ "sides": 20 }),
                    )
                    .with_internal_call_id("dice".to_owned()),
                )),
                Ok(RawStreamingChoice::ReasoningDelta {
                    id: Some("reasoning-1".to_owned()),
                    reasoning: "checking".to_owned(),
                }),
                Ok(RawStreamingChoice::Unknown(json!({
                    "type": "provider.extension"
                }))),
                Ok(RawStreamingChoice::FinalResponse(ProbeResponse { usage })),
            ];
            let inner: StreamingResult<ProbeResponse> = Box::pin(stream::iter(items));
            let mut stream =
                completion_stream(StreamingCompletionResponse::stream(inner), "deepseek");
            let mut events = Vec::new();
            while let Some(event) = stream.next().await {
                events.push(event.expect("valid stream items must convert"));
            }

            let response = validate_stream_events(&events)
                .expect("converted events must satisfy the shared streaming contract");
            assert_eq!(
                response,
                &CompletionResponse {
                    id: None,
                    model: None,
                    content: vec![
                        AssistantContent::Text(TextContent::new("start")),
                        AssistantContent::ToolCall(tool_call(
                            "call-weather",
                            "get_weather",
                            json!({ "city": "上海" }),
                        )),
                        AssistantContent::ToolCall(tool_call(
                            "call-dice",
                            "roll_dice",
                            json!({ "sides": 20 }),
                        )),
                        AssistantContent::ProviderData(armillae_core::ProviderData {
                            provider: "deepseek".to_owned(),
                            kind: "reasoning".to_owned(),
                            value: json!({
                                "id": "reasoning-1",
                                "content": [{
                                    "type": "text",
                                    "content": { "text": "checking" }
                                }]
                            }),
                        }),
                    ],
                    finish_reason: None,
                    usage: Some(TokenUsage {
                        input_tokens: Some(8),
                        output_tokens: Some(5),
                        total_tokens: Some(13),
                        cached_input_tokens: Some(3),
                    }),
                    provider_metadata: json!({}),
                }
            );
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        armillae_core::CompletionEvent::ResponseCompleted { .. }
                    ))
                    .count(),
                1
            );
            assert!(events.iter().any(|event| matches!(
                event,
                armillae_core::CompletionEvent::ProviderEvent { data }
                    if data.kind == "unknown_stream_item"
            )));
        });
    }

    #[test]
    fn complete_reasoning_finalizes_the_active_delta_block_without_duplication() {
        block_on(async {
            let items = vec![
                Ok(RawStreamingChoice::ReasoningDelta {
                    id: None,
                    reasoning: "思".to_owned(),
                }),
                Ok(RawStreamingChoice::ReasoningDelta {
                    id: None,
                    reasoning: "考".to_owned(),
                }),
                Ok(RawStreamingChoice::Reasoning {
                    id: None,
                    content: ReasoningContent::Text {
                        text: "思考".to_owned(),
                        signature: Some("signed".to_owned()),
                    },
                }),
                Ok(RawStreamingChoice::FinalResponse(ProbeResponse {
                    usage: Usage::default(),
                })),
            ];
            let inner: StreamingResult<ProbeResponse> = Box::pin(stream::iter(items));
            let mut stream =
                completion_stream(StreamingCompletionResponse::stream(inner), "anthropic");
            let mut events = Vec::new();
            while let Some(event) = stream.next().await {
                events.push(event.expect("Anthropic reasoning stream must remain valid"));
            }
            let response = validate_stream_events(&events)
                .expect("reasoning events must satisfy the shared streaming contract");

            assert_eq!(response.content.len(), 1);
            assert!(matches!(
                &response.content[0],
                AssistantContent::ProviderData(data)
                    if data.provider == "anthropic"
                        && data.kind == "reasoning"
                        && data.value["content"][0]["content"]["text"] == "思考"
                        && data.value["content"][0]["content"]["signature"] == "signed"
            ));
        });
    }

    #[test]
    fn errors_missing_final_and_incomplete_tools_interrupt_without_completion() {
        block_on(async {
            let cases: Vec<StreamingResult<ProbeResponse>> = vec![
                Box::pin(stream::iter(vec![Err(CompletionError::ProviderError(
                    "sensitive provider failure".to_owned(),
                ))])),
                Box::pin(stream::iter(vec![Ok(RawStreamingChoice::Message(
                    "partial".to_owned(),
                ))])),
                Box::pin(stream::iter(vec![
                    Ok(RawStreamingChoice::ToolCallDelta {
                        id: "call-incomplete".to_owned(),
                        internal_call_id: "incomplete".to_owned(),
                        content: ToolCallDeltaContent::Delta("{\"x\":".to_owned()),
                    }),
                    Ok(RawStreamingChoice::FinalResponse(ProbeResponse {
                        usage: Usage::default(),
                    })),
                ])),
                Box::pin(stream::iter(vec![
                    Ok(RawStreamingChoice::ToolCallDelta {
                        id: "call-invalid".to_owned(),
                        internal_call_id: "invalid".to_owned(),
                        content: ToolCallDeltaContent::Delta("{not-json".to_owned()),
                    }),
                    Ok(RawStreamingChoice::ToolCall(
                        RawStreamingToolCall::new(
                            "call-invalid".to_owned(),
                            "invalid".to_owned(),
                            json!({}),
                        )
                        .with_internal_call_id("invalid".to_owned()),
                    )),
                    Ok(RawStreamingChoice::FinalResponse(ProbeResponse {
                        usage: Usage::default(),
                    })),
                ])),
            ];

            for inner in cases {
                let mut stream =
                    completion_stream(StreamingCompletionResponse::stream(inner), "openai");
                let mut completed = false;
                let mut interrupted = 0;
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(armillae_core::CompletionEvent::ResponseCompleted { .. }) => {
                            completed = true;
                        }
                        Err(BridgeError::StreamInterrupted { metadata }) => {
                            assert_eq!(metadata.provider, "openai");
                            interrupted += 1;
                        }
                        Ok(_) => {}
                        Err(error) => panic!("unexpected streaming error: {error}"),
                    }
                }
                assert!(!completed);
                assert_eq!(interrupted, 1);
            }
        });
    }

    struct DropAwareStream {
        dropped: Arc<AtomicBool>,
    }

    impl Stream for DropAwareStream {
        type Item = Result<RawStreamingChoice<ProbeResponse>, CompletionError>;

        fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Pending
        }
    }

    impl Drop for DropAwareStream {
        fn drop(&mut self) {
            self.dropped.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn dropping_armillae_stream_drops_the_rig_provider_stream() {
        let dropped = Arc::new(AtomicBool::new(false));
        let inner: StreamingResult<ProbeResponse> = Box::pin(DropAwareStream {
            dropped: dropped.clone(),
        });
        let stream = completion_stream(
            StreamingCompletionResponse::stream(inner),
            "openai-compatible",
        );

        drop(stream);

        assert!(dropped.load(Ordering::SeqCst));
    }
}
