use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use futures::{Stream, StreamExt, stream};
use rig_core::{
    OneOrMany,
    client::CompletionClient,
    completion::{
        CompletionError, CompletionModel, CompletionRequest, CompletionResponse, GetTokenUsage,
        ToolDefinition, Usage,
    },
    message::{AssistantContent, Message, ToolResultContent, UserContent},
    providers::{anthropic, openai},
    streaming::{
        RawStreamingChoice, RawStreamingToolCall, StreamedAssistantContent,
        StreamingCompletionResponse, StreamingResult, ToolCallDeltaContent,
    },
    test_utils::{RecordingHttpClient, SequencedStreamingHttpClient},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ProbeResponse {
    total_tokens: u64,
}

impl GetTokenUsage for ProbeResponse {
    fn token_usage(&self) -> Usage {
        Usage {
            total_tokens: self.total_tokens,
            ..Usage::default()
        }
    }
}

#[derive(Clone, Default)]
struct ProbeModel {
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
}

impl CompletionModel for ProbeModel {
    type Response = ProbeResponse;
    type StreamingResponse = ProbeResponse;
    type Client = ();

    fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
        Self::default()
    }

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        self.requests
            .lock()
            .expect("the P0 probe request mutex must not be poisoned")
            .push(request);

        Ok(CompletionResponse {
            choice: many(vec![
                AssistantContent::tool_call(
                    "call-weather",
                    "get_weather",
                    json!({
                        "city": "上海"
                    }),
                ),
                AssistantContent::tool_call(
                    "call-dice",
                    "roll_dice",
                    json!({
                        "sides": 20
                    }),
                ),
            ]),
            usage: Usage {
                input_tokens: 10,
                output_tokens: 5,
                total_tokens: 15,
                ..Usage::default()
            },
            raw_response: ProbeResponse { total_tokens: 15 },
            message_id: Some("message-1".to_owned()),
        })
    }

    async fn stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        let inner: StreamingResult<ProbeResponse> = Box::pin(stream::empty());
        Ok(StreamingCompletionResponse::stream(inner))
    }
}

fn many<T: Clone>(items: Vec<T>) -> OneOrMany<T> {
    OneOrMany::many(items).expect("P0 fixtures always contain at least two items")
}

fn tool_definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_owned(),
        description: format!("P0 definition for {name}"),
        parameters: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    }
}

fn tool_history() -> OneOrMany<Message> {
    many(vec![
        Message::User {
            content: OneOrMany::one(UserContent::text("Use both tools.")),
        },
        Message::Assistant {
            id: Some("assistant-message-1".to_owned()),
            content: many(vec![
                AssistantContent::tool_call(
                    "call-weather",
                    "get_weather",
                    json!({
                        "city": "上海"
                    }),
                ),
                AssistantContent::tool_call(
                    "call-dice",
                    "roll_dice",
                    json!({
                        "sides": 20
                    }),
                ),
            ]),
        },
        Message::User {
            content: many(vec![
                UserContent::tool_result(
                    "call-weather",
                    OneOrMany::one(ToolResultContent::json(json!({
                        "condition": "rain"
                    }))),
                ),
                UserContent::tool_result(
                    "call-dice",
                    OneOrMany::one(ToolResultContent::json(json!({ "value": 17 }))),
                ),
                UserContent::text("Summarize the results."),
            ]),
        },
    ])
}

fn completion_request() -> CompletionRequest {
    CompletionRequest {
        model: None,
        preamble: None,
        chat_history: tool_history(),
        documents: Vec::new(),
        tools: vec![tool_definition("get_weather"), tool_definition("roll_dice")],
        temperature: None,
        max_tokens: Some(256),
        tool_choice: None,
        additional_params: None,
        output_schema: None,
        record_telemetry_content: false,
    }
}

fn tool_call_ids(items: OneOrMany<AssistantContent>) -> Vec<String> {
    items
        .into_iter()
        .filter_map(|item| match item {
            AssistantContent::ToolCall(call) => Some(call.id),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn completion_model_can_be_called_directly_with_tools_and_history() {
    let model = ProbeModel::default();
    let response = model
        .completion(completion_request())
        .await
        .expect("the deterministic P0 model must complete");

    assert_eq!(
        tool_call_ids(response.choice),
        ["call-weather", "call-dice"]
    );
    assert_eq!(response.usage.total_tokens, 15);

    let requests = model
        .requests
        .lock()
        .expect("the P0 probe request mutex must not be poisoned");
    let request = requests
        .first()
        .expect("the direct CompletionModel call must capture one request");
    assert_eq!(request.tools.len(), 2);
    assert_eq!(request.chat_history.len(), 3);
}

#[test]
fn provider_histories_encode_tool_results_with_native_shapes() {
    let history = tool_history().into_iter().collect::<Vec<_>>();

    let openai_messages = history
        .iter()
        .cloned()
        .map(Vec::<openai::completion::Message>::try_from)
        .collect::<Result<Vec<_>, _>>()
        .expect("the P0 history must convert to OpenAI messages")
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let openai_json = serde_json::to_value(openai_messages)
        .expect("OpenAI P0 messages must serialize for inspection");

    assert_eq!(openai_json[1]["role"], "assistant");
    assert_eq!(openai_json[1]["tool_calls"][0]["id"], "call-weather");
    assert_eq!(openai_json[1]["tool_calls"][1]["id"], "call-dice");
    assert_eq!(openai_json[2]["role"], "tool");
    assert_eq!(openai_json[2]["tool_call_id"], "call-weather");
    assert_eq!(openai_json[3]["role"], "tool");
    assert_eq!(openai_json[3]["tool_call_id"], "call-dice");
    assert_eq!(openai_json[4]["role"], "user");

    let anthropic_messages = history
        .into_iter()
        .map(anthropic::completion::Message::try_from)
        .collect::<Result<Vec<_>, _>>()
        .expect("the P0 history must convert to Anthropic messages");
    let anthropic_json = serde_json::to_value(anthropic_messages)
        .expect("Anthropic P0 messages must serialize for inspection");

    assert_eq!(anthropic_json[1]["role"], "assistant");
    assert_eq!(anthropic_json[1]["content"][0]["type"], "tool_use");
    assert_eq!(anthropic_json[1]["content"][0]["id"], "call-weather");
    assert_eq!(anthropic_json[1]["content"][1]["type"], "tool_use");
    assert_eq!(anthropic_json[1]["content"][1]["id"], "call-dice");
    assert_eq!(anthropic_json[2]["role"], "user");
    assert_eq!(anthropic_json[2]["content"][0]["type"], "tool_result");
    assert_eq!(
        anthropic_json[2]["content"][0]["tool_use_id"],
        "call-weather"
    );
    assert_eq!(anthropic_json[2]["content"][1]["type"], "tool_result");
    assert_eq!(anthropic_json[2]["content"][1]["tool_use_id"], "call-dice");
}

#[tokio::test]
async fn real_provider_models_send_tools_and_normalize_tool_calls_offline() {
    let openai_response = json!({
        "id": "chatcmpl-p0",
        "object": "chat.completion",
        "created": 1,
        "model": "p0-openai",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {
                        "id": "call-weather",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"city\":\"上海\"}"
                        }
                    },
                    {
                        "id": "call-dice",
                        "type": "function",
                        "function": {
                            "name": "roll_dice",
                            "arguments": "{\"sides\":20}"
                        }
                    }
                ]
            },
            "logprobs": null,
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    });
    let openai_http = RecordingHttpClient::new(openai_response.to_string());
    let openai_client = openai::CompletionsClient::builder()
        .api_key("p0-test-key")
        .http_client(openai_http.clone())
        .build()
        .expect("the offline OpenAI P0 client must build");
    let openai_model = openai_client.completion_model("p0-openai");
    let openai_result = openai_model
        .completion(completion_request())
        .await
        .expect("the offline OpenAI P0 call must normalize");
    assert_eq!(
        tool_call_ids(openai_result.choice),
        ["call-weather", "call-dice"]
    );

    let openai_requests = openai_http.requests();
    let openai_request: Value = serde_json::from_slice(
        &openai_requests
            .first()
            .expect("the OpenAI P0 model must send one request")
            .body,
    )
    .expect("the OpenAI P0 request body must be JSON");
    assert_eq!(openai_request["tools"][0]["type"], "function");
    assert_eq!(
        openai_request["tools"][0]["function"]["name"],
        "get_weather"
    );
    assert_eq!(openai_request["tools"][1]["function"]["name"], "roll_dice");
    assert_eq!(openai_request["messages"][1]["role"], "assistant");
    assert_eq!(
        openai_request["messages"][1]["tool_calls"][0]["id"],
        "call-weather"
    );
    assert_eq!(openai_request["messages"][2]["role"], "tool");
    assert_eq!(
        openai_request["messages"][2]["tool_call_id"],
        "call-weather"
    );

    let anthropic_response = json!({
        "type": "message",
        "id": "msg-p0",
        "model": "p0-anthropic",
        "role": "assistant",
        "stop_reason": "tool_use",
        "stop_sequence": null,
        "content": [
            {
                "type": "tool_use",
                "id": "call-weather",
                "name": "get_weather",
                "input": { "city": "上海" }
            },
            {
                "type": "tool_use",
                "id": "call-dice",
                "name": "roll_dice",
                "input": { "sides": 20 }
            }
        ],
        "usage": {
            "input_tokens": 10,
            "cache_read_input_tokens": null,
            "cache_creation_input_tokens": null,
            "output_tokens": 5
        }
    });
    let anthropic_http = RecordingHttpClient::new(anthropic_response.to_string());
    let anthropic_client = anthropic::Client::builder()
        .api_key("p0-test-key")
        .http_client(anthropic_http.clone())
        .build()
        .expect("the offline Anthropic P0 client must build");
    let anthropic_model = anthropic_client.completion_model("claude-p0");
    let anthropic_result = anthropic_model
        .completion(completion_request())
        .await
        .expect("the offline Anthropic P0 call must normalize");
    assert_eq!(
        tool_call_ids(anthropic_result.choice),
        ["call-weather", "call-dice"]
    );

    let anthropic_requests = anthropic_http.requests();
    let anthropic_request: Value = serde_json::from_slice(
        &anthropic_requests
            .first()
            .expect("the Anthropic P0 model must send one request")
            .body,
    )
    .expect("the Anthropic P0 request body must be JSON");
    assert_eq!(anthropic_request["tools"][0]["name"], "get_weather");
    assert_eq!(
        anthropic_request["tools"][0]["input_schema"]["type"],
        "object"
    );
    assert_eq!(anthropic_request["tools"][1]["name"], "roll_dice");
    assert_eq!(anthropic_request["messages"][1]["role"], "assistant");
    assert_eq!(
        anthropic_request["messages"][1]["content"][0]["type"],
        "tool_use"
    );
    assert_eq!(anthropic_request["messages"][2]["role"], "user");
    assert_eq!(
        anthropic_request["messages"][2]["content"][0]["type"],
        "tool_result"
    );
    assert_eq!(
        anthropic_request["messages"][2]["content"][0]["tool_use_id"],
        "call-weather"
    );
}

#[test]
fn provider_responses_preserve_multiple_tool_call_ids_and_order() {
    let openai_response: openai::completion::CompletionResponse = serde_json::from_value(json!({
        "id": "chatcmpl-p0",
        "object": "chat.completion",
        "created": 1,
        "model": "p0-openai",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [
                    {
                        "id": "call-weather",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"city\":\"上海\"}"
                        }
                    },
                    {
                        "id": "call-dice",
                        "type": "function",
                        "function": {
                            "name": "roll_dice",
                            "arguments": "{\"sides\":20}"
                        }
                    }
                ]
            },
            "logprobs": null,
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }
    }))
    .expect("the OpenAI P0 response fixture must deserialize");
    let openai_generic: CompletionResponse<_> = openai_response
        .try_into()
        .expect("the OpenAI P0 response must normalize");
    assert_eq!(
        tool_call_ids(openai_generic.choice),
        ["call-weather", "call-dice"]
    );

    let anthropic_response: anthropic::completion::CompletionResponse =
        serde_json::from_value(json!({
            "id": "msg-p0",
            "model": "p0-anthropic",
            "role": "assistant",
            "stop_reason": "tool_use",
            "stop_sequence": null,
            "content": [
                {
                    "type": "tool_use",
                    "id": "call-weather",
                    "name": "get_weather",
                    "input": { "city": "上海" }
                },
                {
                    "type": "tool_use",
                    "id": "call-dice",
                    "name": "roll_dice",
                    "input": { "sides": 20 }
                }
            ],
            "usage": {
                "input_tokens": 10,
                "cache_read_input_tokens": null,
                "cache_creation_input_tokens": null,
                "output_tokens": 5
            }
        }))
        .expect("the Anthropic P0 response fixture must deserialize");
    let anthropic_generic: CompletionResponse<_> = anthropic_response
        .try_into()
        .expect("the Anthropic P0 response must normalize");
    assert_eq!(
        tool_call_ids(anthropic_generic.choice),
        ["call-weather", "call-dice"]
    );
}

#[derive(Debug, PartialEq)]
struct AssembledToolCall {
    index: usize,
    id: String,
    name: String,
    arguments: Value,
}

#[derive(Default)]
struct PartialToolCall {
    index: usize,
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
struct ToolDeltaAssembler {
    order: Vec<String>,
    calls: HashMap<String, PartialToolCall>,
}

impl ToolDeltaAssembler {
    fn push(&mut self, id: &str, internal_call_id: &str, content: &ToolCallDeltaContent) {
        let next_index = self.order.len();
        let call = self
            .calls
            .entry(internal_call_id.to_owned())
            .or_insert_with(|| {
                self.order.push(internal_call_id.to_owned());
                PartialToolCall {
                    index: next_index,
                    id: id.to_owned(),
                    ..PartialToolCall::default()
                }
            });

        if call.id.is_empty() && !id.is_empty() {
            call.id = id.to_owned();
        }
        match content {
            ToolCallDeltaContent::Name(fragment) => call.name.push_str(fragment),
            ToolCallDeltaContent::Delta(fragment) => call.arguments.push_str(fragment),
        }
    }

    fn finish(self) -> Vec<AssembledToolCall> {
        self.order
            .into_iter()
            .map(|internal_call_id| {
                let call = self
                    .calls
                    .get(&internal_call_id)
                    .expect("every ordered internal call ID must have an accumulator");
                AssembledToolCall {
                    index: call.index,
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: serde_json::from_str(&call.arguments)
                        .expect("completed P0 tool arguments must be valid JSON"),
                }
            })
            .collect()
    }
}

#[tokio::test]
async fn interleaved_stream_deltas_reassemble_by_stable_internal_call_id() {
    let items = vec![
        RawStreamingChoice::Message("start".to_owned()),
        RawStreamingChoice::ToolCallDelta {
            id: "call-weather".to_owned(),
            internal_call_id: "internal-weather".to_owned(),
            content: ToolCallDeltaContent::Name("get_".to_owned()),
        },
        RawStreamingChoice::ToolCallDelta {
            id: "call-dice".to_owned(),
            internal_call_id: "internal-dice".to_owned(),
            content: ToolCallDeltaContent::Name("roll_".to_owned()),
        },
        RawStreamingChoice::ToolCallDelta {
            id: "call-weather".to_owned(),
            internal_call_id: "internal-weather".to_owned(),
            content: ToolCallDeltaContent::Name("weather".to_owned()),
        },
        RawStreamingChoice::ToolCallDelta {
            id: "call-weather".to_owned(),
            internal_call_id: "internal-weather".to_owned(),
            content: ToolCallDeltaContent::Delta("{\"city\":\"上".to_owned()),
        },
        RawStreamingChoice::ToolCallDelta {
            id: "call-dice".to_owned(),
            internal_call_id: "internal-dice".to_owned(),
            content: ToolCallDeltaContent::Name("dice".to_owned()),
        },
        RawStreamingChoice::ToolCallDelta {
            id: "call-dice".to_owned(),
            internal_call_id: "internal-dice".to_owned(),
            content: ToolCallDeltaContent::Delta("{\"sides\":".to_owned()),
        },
        RawStreamingChoice::ToolCallDelta {
            id: "call-weather".to_owned(),
            internal_call_id: "internal-weather".to_owned(),
            content: ToolCallDeltaContent::Delta("海\"}".to_owned()),
        },
        RawStreamingChoice::ToolCallDelta {
            id: "call-dice".to_owned(),
            internal_call_id: "internal-dice".to_owned(),
            content: ToolCallDeltaContent::Delta("20}".to_owned()),
        },
        RawStreamingChoice::ToolCall(
            RawStreamingToolCall::new(
                "call-weather".to_owned(),
                "get_weather".to_owned(),
                json!({ "city": "上海" }),
            )
            .with_internal_call_id("internal-weather".to_owned()),
        ),
        RawStreamingChoice::ToolCall(
            RawStreamingToolCall::new(
                "call-dice".to_owned(),
                "roll_dice".to_owned(),
                json!({ "sides": 20 }),
            )
            .with_internal_call_id("internal-dice".to_owned()),
        ),
        RawStreamingChoice::FinalResponse(ProbeResponse { total_tokens: 20 }),
        RawStreamingChoice::FinalResponse(ProbeResponse { total_tokens: 99 }),
    ]
    .into_iter()
    .map(Ok);
    let inner: StreamingResult<ProbeResponse> = Box::pin(stream::iter(items));
    let mut response = StreamingCompletionResponse::stream(inner);
    let mut assembler = ToolDeltaAssembler::default();
    let mut text_deltas = Vec::new();
    let mut completed_ids = Vec::new();
    let mut final_count = 0;

    while let Some(item) = response.next().await {
        match item.expect("the P0 stream must not fail") {
            StreamedAssistantContent::Text(text) => text_deltas.push(text.text),
            StreamedAssistantContent::ToolCallDelta {
                id,
                internal_call_id,
                content,
            } => assembler.push(&id, &internal_call_id, &content),
            StreamedAssistantContent::ToolCall { tool_call, .. } => {
                completed_ids.push(tool_call.id);
            }
            StreamedAssistantContent::Final(_) => final_count += 1,
            StreamedAssistantContent::Reasoning(_)
            | StreamedAssistantContent::ReasoningDelta { .. }
            | StreamedAssistantContent::Unknown(_) => {}
        }
    }

    assert_eq!(text_deltas, ["start"]);
    assert_eq!(completed_ids, ["call-weather", "call-dice"]);
    assert_eq!(final_count, 1);
    assert_eq!(
        assembler.finish(),
        [
            AssembledToolCall {
                index: 0,
                id: "call-weather".to_owned(),
                name: "get_weather".to_owned(),
                arguments: json!({ "city": "上海" }),
            },
            AssembledToolCall {
                index: 1,
                id: "call-dice".to_owned(),
                name: "roll_dice".to_owned(),
                arguments: json!({ "sides": 20 }),
            },
        ]
    );
    assert_eq!(response.usage().total_tokens, 20);
}

#[tokio::test]
async fn openai_stream_survives_arbitrary_http_and_utf8_chunk_boundaries() {
    let events = [
        json!({
            "id": "stream-p0",
            "model": "p0-openai",
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "id": "call-weather",
                        "function": { "name": "get_weather", "arguments": "" }
                    }]
                },
                "finish_reason": null
            }],
            "usage": null
        }),
        json!({
            "id": "stream-p0",
            "model": "p0-openai",
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 1,
                        "id": "call-dice",
                        "function": { "name": "roll_dice", "arguments": "" }
                    }]
                },
                "finish_reason": null
            }],
            "usage": null
        }),
        json!({
            "id": "stream-p0",
            "model": "p0-openai",
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": { "arguments": "{\"city\":\"上" }
                    }]
                },
                "finish_reason": null
            }],
            "usage": null
        }),
        json!({
            "id": "stream-p0",
            "model": "p0-openai",
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 1,
                        "function": { "arguments": "{\"sides\":" }
                    }]
                },
                "finish_reason": null
            }],
            "usage": null
        }),
        json!({
            "id": "stream-p0",
            "model": "p0-openai",
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 0,
                        "function": { "arguments": "海\"}" }
                    }]
                },
                "finish_reason": null
            }],
            "usage": null
        }),
        json!({
            "id": "stream-p0",
            "model": "p0-openai",
            "choices": [{
                "delta": {
                    "tool_calls": [{
                        "index": 1,
                        "function": { "arguments": "20}" }
                    }]
                },
                "finish_reason": null
            }],
            "usage": null
        }),
        json!({
            "id": "stream-p0",
            "model": "p0-openai",
            "choices": [{
                "delta": {},
                "finish_reason": "tool_calls"
            }],
            "usage": null
        }),
        json!({
            "id": "stream-p0",
            "model": "p0-openai",
            "choices": [],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        }),
    ];
    let mut sse = events
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect::<String>();
    sse.push_str("data: [DONE]\n\n");

    let chunks = sse
        .as_bytes()
        .chunks(7)
        .map(|chunk| Ok::<_, rig_core::http_client::Error>(chunk.to_vec().into()))
        .collect();
    let http_client = SequencedStreamingHttpClient::new(chunks);
    let client = openai::CompletionsClient::builder()
        .api_key("p0-test-key")
        .http_client(http_client)
        .build()
        .expect("the offline OpenAI streaming P0 client must build");
    let model = client.completion_model("p0-openai");
    let mut stream = model
        .stream(completion_request())
        .await
        .expect("the offline OpenAI P0 stream must start");
    let mut internal_ids = HashMap::<String, String>::new();
    let mut complete_calls = Vec::new();

    while let Some(item) = stream.next().await {
        match item.expect("the OpenAI P0 stream must parse every byte chunk") {
            StreamedAssistantContent::ToolCallDelta {
                id,
                internal_call_id,
                ..
            } => {
                if !id.is_empty() {
                    let existing = internal_ids.entry(id).or_insert(internal_call_id.clone());
                    assert_eq!(existing, &internal_call_id);
                }
            }
            StreamedAssistantContent::ToolCall { tool_call, .. } => {
                complete_calls.push(tool_call);
            }
            StreamedAssistantContent::Text(_)
            | StreamedAssistantContent::Final(_)
            | StreamedAssistantContent::Reasoning(_)
            | StreamedAssistantContent::ReasoningDelta { .. }
            | StreamedAssistantContent::Unknown(_) => {}
        }
    }

    assert_eq!(complete_calls.len(), 2);
    assert_eq!(complete_calls[0].id, "call-weather");
    assert_eq!(complete_calls[0].function.name, "get_weather");
    assert_eq!(
        complete_calls[0].function.arguments,
        json!({ "city": "上海" })
    );
    assert_eq!(complete_calls[1].id, "call-dice");
    assert_eq!(complete_calls[1].function.name, "roll_dice");
    assert_eq!(complete_calls[1].function.arguments, json!({ "sides": 20 }));
    assert_eq!(stream.usage().total_tokens, 15);
}

struct DropAwareStream {
    dropped: Arc<AtomicBool>,
}

impl Stream for DropAwareStream {
    type Item = Result<RawStreamingChoice<ProbeResponse>, CompletionError>;

    fn poll_next(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}

impl Drop for DropAwareStream {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

struct DropAwareCompletionFuture {
    dropped: Arc<AtomicBool>,
}

impl Future for DropAwareCompletionFuture {
    type Output = Result<CompletionResponse<ProbeResponse>, CompletionError>;

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}

impl Drop for DropAwareCompletionFuture {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

#[derive(Clone)]
struct DropAwareModel {
    completion_dropped: Arc<AtomicBool>,
}

impl CompletionModel for DropAwareModel {
    type Response = ProbeResponse;
    type StreamingResponse = ProbeResponse;
    type Client = Arc<AtomicBool>;

    fn make(client: &Self::Client, _model: impl Into<String>) -> Self {
        Self {
            completion_dropped: Arc::clone(client),
        }
    }

    fn completion(
        &self,
        _request: CompletionRequest,
    ) -> impl Future<Output = Result<CompletionResponse<Self::Response>, CompletionError>> + Send
    {
        DropAwareCompletionFuture {
            dropped: Arc::clone(&self.completion_dropped),
        }
    }

    fn stream(
        &self,
        _request: CompletionRequest,
    ) -> impl Future<
        Output = Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError>,
    > + Send {
        std::future::ready(Err(CompletionError::ResponseError(
            "the P0 drop-aware model does not provide a stream".to_owned(),
        )))
    }
}

#[test]
fn dropping_completion_future_and_stream_release_their_inner_resources() {
    let completion_dropped = Arc::new(AtomicBool::new(false));
    let model = DropAwareModel::make(&completion_dropped, "p0-model");
    let future = model.completion(completion_request());
    drop(future);
    assert!(completion_dropped.load(Ordering::SeqCst));

    let stream_dropped = Arc::new(AtomicBool::new(false));
    let inner: StreamingResult<ProbeResponse> = Box::pin(DropAwareStream {
        dropped: Arc::clone(&stream_dropped),
    });
    let response = StreamingCompletionResponse::stream(inner);
    drop(response);
    assert!(stream_dropped.load(Ordering::SeqCst));
}
