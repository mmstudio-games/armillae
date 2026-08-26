use std::sync::Arc;

use armillae_core::{AssistantContent, FinishReason, ToolCallId};
use armillae_llm::{
    BridgeCapabilities, BridgeConfig, BridgeError, ErrorMetadata, LlmBridge,
    OutputFormatCapabilities, ToolChoiceCapabilities,
};
use rig_core::{client::CompletionClient, http_client::HttpClientExt, providers::ollama};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};

use crate::{
    RigBridge,
    request::OllamaRequestMapper,
    response::{
        NormalizedResponseFacts, NormalizedStreamingResponseFacts, RigResponseNormalizer,
        RigStreamingResponseNormalizer,
    },
};

use super::build_http_client;

pub(crate) fn create(
    config: BridgeConfig,
    credential: Option<SecretString>,
) -> Result<Arc<dyn LlmBridge>, BridgeError> {
    let (config, credential, request_mapper) = validate_config(config, credential)?;
    let http_client = build_http_client(&config)?;

    create_validated(config, credential, request_mapper, http_client)
}

fn validate_config(
    config: BridgeConfig,
    credential: Option<SecretString>,
) -> Result<(BridgeConfig, Option<SecretString>, OllamaRequestMapper), BridgeError> {
    if config.provider != "ollama" {
        return invalid_configuration(format!(
            "Ollama provider module cannot construct provider: {}",
            config.provider
        ));
    }
    let request_mapper = OllamaRequestMapper::new(config.provider_options.clone())?;

    Ok((config, credential, request_mapper))
}

fn create_validated<H>(
    config: BridgeConfig,
    credential: Option<SecretString>,
    request_mapper: OllamaRequestMapper,
    http_client: H,
) -> Result<Arc<dyn LlmBridge>, BridgeError>
where
    H: HttpClientExt + Clone + Default + std::fmt::Debug + 'static,
{
    let api_key = credential
        .map(|credential| ollama::OllamaApiKey::from(credential.expose_secret().to_owned()))
        .unwrap_or_default();
    let mut client_builder = ollama::Client::builder()
        .api_key(api_key)
        .http_client(http_client);
    if let Some(endpoint) = &config.endpoint {
        client_builder = client_builder.base_url(endpoint.as_str());
    }
    let client = client_builder
        .build()
        .map_err(|_| BridgeError::InvalidConfiguration {
            message: "failed to construct Rig Ollama client".to_owned(),
        })?;
    let model_name = config.model.clone();
    let model = client.completion_model(config.model);
    let bridge = RigBridge::new_with_streaming_normalizer(
        model,
        model_name,
        capabilities(),
        config.defaults,
        Arc::new(request_mapper),
        Arc::new(OllamaResponseNormalizer),
        Arc::new(OllamaStreamingResponseNormalizer),
    )?;

    Ok(Arc::new(bridge))
}

const fn capabilities() -> BridgeCapabilities {
    BridgeCapabilities {
        streaming: true,
        tool_calling: true,
        parallel_tool_calls: true,
        tool_choice: ToolChoiceCapabilities {
            auto: false,
            none: false,
            required: false,
            specific: false,
        },
        output_format: OutputFormatCapabilities::all(),
        system_message: true,
        developer_message: false,
    }
}

#[derive(Clone, Copy, Debug)]
struct OllamaResponseNormalizer;

impl RigResponseNormalizer<ollama::CompletionResponse> for OllamaResponseNormalizer {
    fn provider(&self) -> &str {
        "ollama"
    }

    fn normalize(
        &self,
        raw_response: &ollama::CompletionResponse,
    ) -> Result<NormalizedResponseFacts, BridgeError> {
        if raw_response.model.trim().is_empty() {
            return invalid_provider_response("Ollama response model is empty");
        }
        if !raw_response.done {
            return invalid_provider_response("Ollama non-streaming response is not complete");
        }

        Ok(NormalizedResponseFacts {
            id: None,
            model: Some(raw_response.model.clone()),
            finish_reason: raw_response
                .done_reason
                .as_deref()
                .map(ollama_finish_reason),
            provider_metadata: completion_metadata(raw_response),
        })
    }

    fn normalize_content(
        &self,
        mut content: Vec<AssistantContent>,
    ) -> Result<Vec<AssistantContent>, BridgeError> {
        let mut call_index = 0usize;
        for item in &mut content {
            if let AssistantContent::ToolCall(call) = item {
                call_index += 1;
                call.id = ToolCallId::new(format!("ollama-call-{call_index}")).map_err(|_| {
                    BridgeError::InvalidProviderResponse {
                        message: "failed to generate an Ollama ToolCall ID".to_owned(),
                        metadata: ErrorMetadata::new("ollama"),
                    }
                })?;
            }
        }
        Ok(content)
    }
}

#[derive(Clone, Copy, Debug)]
struct OllamaStreamingResponseNormalizer;

impl RigStreamingResponseNormalizer<ollama::StreamingCompletionResponse>
    for OllamaStreamingResponseNormalizer
{
    fn normalize(
        &self,
        raw_response: &ollama::StreamingCompletionResponse,
    ) -> Result<NormalizedStreamingResponseFacts, ()> {
        Ok(NormalizedStreamingResponseFacts {
            finish_reason: raw_response
                .done_reason
                .as_deref()
                .map(ollama_finish_reason),
            provider_metadata: streaming_metadata(raw_response),
        })
    }
}

fn ollama_finish_reason(reason: &str) -> FinishReason {
    match reason {
        "stop" => FinishReason::Stop,
        "length" | "max_tokens" => FinishReason::Length,
        "tool_calls" | "function_call" => FinishReason::ToolCall,
        "cancelled" => FinishReason::Cancelled,
        other => FinishReason::Unknown(other.to_owned()),
    }
}

fn completion_metadata(response: &ollama::CompletionResponse) -> Value {
    let mut metadata = Map::new();
    if !response.created_at.is_empty() {
        metadata.insert(
            "created_at".to_owned(),
            Value::String(response.created_at.clone()),
        );
    }
    insert_duration_metadata(
        &mut metadata,
        response.total_duration,
        response.load_duration,
        response.prompt_eval_duration,
        response.eval_duration,
    );
    Value::Object(metadata)
}

fn streaming_metadata(response: &ollama::StreamingCompletionResponse) -> Value {
    let mut metadata = Map::new();
    insert_duration_metadata(
        &mut metadata,
        response.total_duration,
        response.load_duration,
        response.prompt_eval_duration,
        response.eval_duration,
    );
    Value::Object(metadata)
}

fn insert_duration_metadata(
    metadata: &mut Map<String, Value>,
    total_duration: Option<u64>,
    load_duration: Option<u64>,
    prompt_eval_duration: Option<u64>,
    eval_duration: Option<u64>,
) {
    for (name, value) in [
        ("total_duration_ns", total_duration),
        ("load_duration_ns", load_duration),
        ("prompt_eval_duration_ns", prompt_eval_duration),
        ("eval_duration_ns", eval_duration),
    ] {
        if let Some(value) = value {
            metadata.insert(name.to_owned(), json!(value));
        }
    }
}

fn invalid_configuration<T>(message: impl Into<String>) -> Result<T, BridgeError> {
    Err(BridgeError::InvalidConfiguration {
        message: message.into(),
    })
}

fn invalid_provider_response<T>(message: impl Into<String>) -> Result<T, BridgeError> {
    Err(BridgeError::InvalidProviderResponse {
        message: message.into(),
        metadata: ErrorMetadata::new("ollama"),
    })
}

#[cfg(test)]
mod tests {
    use armillae_core::{
        AssistantContent, CompletionEvent, CompletionRequest, CompletionResponse, ContentPart,
        FinishReason, GenerationOptions, Message, OutputFormat, ProviderExtensions, Role,
        TokenUsage, ToolCall, ToolCallId, ToolChoice, ToolDefinition, ToolResult,
        ToolResultContent,
    };
    use armillae_llm::{
        BridgeError,
        mock::contract::{validate_stream_events, verify_completion},
    };
    use futures::StreamExt;
    use rig_core::test_utils::{RecordingHttpClient, SequencedStreamingHttpClient};
    use serde_json::{Value, json};

    use super::{capabilities, create, create_validated, validate_config};
    use crate::providers::test_support::resolved_config;

    fn tool_call_id(value: &str) -> ToolCallId {
        ToolCallId::new(value).expect("fixture ToolCall IDs are non-empty")
    }

    fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: "lookup".to_owned(),
            description: "Look up a value".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"]
            }),
        }
    }

    fn completion_response() -> Value {
        json!({
            "model": "qwen3:8b",
            "created_at": "2026-08-26T00:00:00Z",
            "message": {
                "role": "assistant",
                "thinking": "checking",
                "content": "using tools",
                "tool_calls": [
                    {
                        "type": "function",
                        "function": { "name": "lookup", "arguments": { "query": "first" } }
                    },
                    {
                        "type": "function",
                        "function": { "name": "lookup", "arguments": { "query": "second" } }
                    }
                ]
            },
            "done": true,
            "done_reason": "tool_calls",
            "total_duration": 100,
            "load_duration": 10,
            "prompt_eval_count": 4,
            "prompt_eval_duration": 20,
            "eval_count": 3,
            "eval_duration": 70
        })
    }

    #[test]
    fn configuration_and_capability_profile_are_explicit() {
        let (config, credential) = resolved_config("ollama", None, false, json!({}));
        let bridge = create(config, credential)
            .expect("Ollama must support its default local endpoint without credentials");
        let (config, credential) = resolved_config("ollama", None, false, json!({ "think": true }));
        let unknown_options = create(config, credential);

        assert_eq!(bridge.capabilities(), capabilities());
        assert!(bridge.capabilities().streaming);
        assert!(bridge.capabilities().tool_calling);
        assert_eq!(
            bridge.capabilities().tool_choice,
            armillae_llm::ToolChoiceCapabilities::default()
        );
        assert!(matches!(
            unknown_options,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
    }

    #[test]
    fn native_wire_and_response_satisfy_the_shared_completion_contract() {
        let (config, credential) =
            resolved_config("ollama", Some("http://ollama.test"), false, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(completion_response().to_string());
            let (config, credential, request_mapper) =
                validate_config(config, credential).expect("Ollama test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client.clone())
                .expect("Ollama test bridge must construct");
            let request = CompletionRequest {
                messages: vec![
                    Message::new(Role::System, vec![ContentPart::text("rules")]),
                    Message::user("use tools"),
                ],
                tools: vec![tool_definition()],
                output_format: Some(OutputFormat::JsonSchema {
                    name: "answer".to_owned(),
                    schema: json!({
                        "type": "object",
                        "properties": { "answer": { "type": "string" } },
                        "required": ["answer"]
                    }),
                    strict: true,
                }),
                generation: GenerationOptions {
                    temperature: Some(0.25),
                    max_output_tokens: Some(64),
                    stop: vec!["END".to_owned()],
                    seed: Some(7),
                },
                ..CompletionRequest::default()
            };
            let expected = CompletionResponse {
                id: None,
                model: Some("qwen3:8b".to_owned()),
                content: vec![
                    AssistantContent::ProviderData(armillae_core::ProviderData {
                        provider: "ollama".to_owned(),
                        kind: "reasoning".to_owned(),
                        value: json!({
                            "id": null,
                            "content": [{
                                "type": "text",
                                "content": { "text": "checking" }
                            }]
                        }),
                    }),
                    AssistantContent::Text(armillae_core::TextContent::new("using tools")),
                    AssistantContent::ToolCall(ToolCall {
                        id: tool_call_id("ollama-call-1"),
                        name: "lookup".to_owned(),
                        arguments: json!({ "query": "first" }),
                    }),
                    AssistantContent::ToolCall(ToolCall {
                        id: tool_call_id("ollama-call-2"),
                        name: "lookup".to_owned(),
                        arguments: json!({ "query": "second" }),
                    }),
                ],
                finish_reason: Some(FinishReason::ToolCall),
                usage: Some(TokenUsage {
                    input_tokens: Some(4),
                    output_tokens: Some(3),
                    total_tokens: Some(7),
                    cached_input_tokens: Some(0),
                }),
                provider_metadata: json!({
                    "created_at": "2026-08-26T00:00:00Z",
                    "total_duration_ns": 100,
                    "load_duration_ns": 10,
                    "prompt_eval_duration_ns": 20,
                    "eval_duration_ns": 70
                }),
            };

            verify_completion(bridge.as_ref(), request, &expected)
                .await
                .expect("Ollama must satisfy the shared completion contract");

            let captured = http_client
                .requests()
                .into_iter()
                .next()
                .expect("Ollama bridge must issue one request");
            assert_eq!(captured.uri, "http://ollama.test/api/chat");
            assert!(captured.headers.get("authorization").is_none());
            let body: Value = serde_json::from_slice(&captured.body)
                .expect("captured Ollama request must be JSON");
            assert_eq!(body["options"]["temperature"], 0.25);
            assert_eq!(body["options"]["num_predict"], 64);
            assert_eq!(body["options"]["stop"], json!(["END"]));
            assert_eq!(body["options"]["seed"], 7);
            assert_eq!(body["format"]["type"], "object");
            assert_eq!(body["tools"][0]["function"]["name"], "lookup");
        });
    }

    #[test]
    fn tool_results_map_generated_ids_back_to_native_tool_names() {
        let (config, credential) =
            resolved_config("ollama", Some("http://ollama.test"), false, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(completion_response().to_string());
            let (config, credential, request_mapper) =
                validate_config(config, credential).expect("Ollama test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client.clone())
                .expect("Ollama test bridge must construct");
            let call = ToolCall {
                id: tool_call_id("ollama-call-1"),
                name: "lookup".to_owned(),
                arguments: json!({ "query": "first" }),
            };
            let request = CompletionRequest {
                messages: vec![
                    Message::user("use tools"),
                    Message::assistant(vec![ContentPart::ToolCall(call.clone())]),
                    Message::new(
                        Role::Tool,
                        vec![ContentPart::ToolResult(ToolResult {
                            call_id: call.id,
                            content: vec![ToolResultContent::Json {
                                value: json!({ "answer": 42 }),
                            }],
                            is_error: true,
                        })],
                    ),
                ],
                ..CompletionRequest::default()
            };

            bridge
                .complete(request)
                .await
                .expect("Ollama ToolResult history must remain callable");

            let captured = http_client
                .requests()
                .into_iter()
                .next()
                .expect("Ollama bridge must issue one request");
            let body: Value = serde_json::from_slice(&captured.body)
                .expect("captured Ollama request must be JSON");
            let tool_message = body["messages"]
                .as_array()
                .and_then(|messages| messages.iter().find(|message| message["role"] == "tool"))
                .expect("Ollama request must contain a native tool result");
            assert_eq!(tool_message["tool_name"], "lookup");
            assert_eq!(tool_message["content"], r#"{"answer":42}"#);
        });
    }

    #[test]
    fn unsupported_or_unmappable_requests_fail_before_transport() {
        let (config, credential) =
            resolved_config("ollama", Some("http://ollama.test"), false, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(completion_response().to_string());
            let (config, credential, request_mapper) =
                validate_config(config, credential).expect("Ollama test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client.clone())
                .expect("Ollama test bridge must construct");

            let tool_choice = bridge
                .complete(CompletionRequest {
                    messages: vec![Message::user("hello")],
                    tools: vec![tool_definition()],
                    tool_choice: Some(ToolChoice::Auto),
                    ..CompletionRequest::default()
                })
                .await;
            let non_strict_schema = bridge
                .complete(CompletionRequest {
                    messages: vec![Message::user("hello")],
                    output_format: Some(OutputFormat::JsonSchema {
                        name: "answer".to_owned(),
                        schema: json!({ "type": "object" }),
                        strict: false,
                    }),
                    ..CompletionRequest::default()
                })
                .await;
            let orphan_result = bridge
                .complete(CompletionRequest {
                    messages: vec![Message::tool_result(ToolResult {
                        call_id: tool_call_id("orphan"),
                        content: vec![ToolResultContent::Text {
                            text: "result".to_owned(),
                        }],
                        is_error: false,
                    })],
                    ..CompletionRequest::default()
                })
                .await;
            let unknown_extension = bridge
                .complete(CompletionRequest {
                    messages: vec![Message::user("hello")],
                    extensions: ProviderExtensions {
                        values: [("ollama.think".to_owned(), json!(true))]
                            .into_iter()
                            .collect(),
                    },
                    ..CompletionRequest::default()
                })
                .await;

            assert!(matches!(
                tool_choice,
                Err(BridgeError::UnsupportedCapability { .. })
            ));
            assert!(matches!(
                non_strict_schema,
                Err(BridgeError::UnsupportedCapability { .. })
            ));
            assert!(matches!(
                orphan_result,
                Err(BridgeError::InvalidRequest { .. })
            ));
            assert!(matches!(
                unknown_extension,
                Err(BridgeError::InvalidRequest { .. })
            ));
            assert!(http_client.requests().is_empty());
        });
    }

    #[test]
    fn provider_rejection_keeps_ollama_identity() {
        let (config, credential) =
            resolved_config("ollama", Some("http://ollama.test"), false, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::with_error_response(
                "400".parse().expect("400 must be a valid HTTP status code"),
                json!({ "error": "unknown model" }).to_string(),
            );
            let (config, credential, request_mapper) =
                validate_config(config, credential).expect("Ollama test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client)
                .expect("Ollama test bridge must construct");

            let error = bridge
                .complete(CompletionRequest {
                    messages: vec![Message::user("hello")],
                    ..CompletionRequest::default()
                })
                .await
                .expect_err("Ollama rejection must remain explicit");
            assert!(matches!(
                error,
                BridgeError::ProviderRejected { metadata, .. }
                    if metadata.provider == "ollama" && metadata.http_status == Some(400)
            ));
        });
    }

    #[test]
    fn ndjson_stream_preserves_text_multiple_tools_usage_and_finish_reason() {
        let (config, credential) =
            resolved_config("ollama", Some("http://ollama.test"), false, json!({}));
        futures::executor::block_on(async {
            let responses = [
                json!({
                    "model": "qwen3:8b",
                    "created_at": "2026-08-26T00:00:00Z",
                    "message": { "role": "assistant", "content": "你" },
                    "done": false
                }),
                json!({
                    "model": "qwen3:8b",
                    "created_at": "2026-08-26T00:00:01Z",
                    "message": { "role": "assistant", "content": "好" },
                    "done": false
                }),
                json!({
                    "model": "qwen3:8b",
                    "created_at": "2026-08-26T00:00:02Z",
                    "message": {
                        "role": "assistant",
                        "content": "",
                        "tool_calls": [
                            { "function": { "name": "lookup", "arguments": { "query": "一" } } },
                            { "function": { "name": "lookup", "arguments": { "query": "二" } } }
                        ]
                    },
                    "done": false
                }),
                json!({
                    "model": "qwen3:8b",
                    "created_at": "2026-08-26T00:00:03Z",
                    "message": { "role": "assistant", "content": "" },
                    "done": true,
                    "done_reason": "stop",
                    "total_duration": 100,
                    "load_duration": 10,
                    "prompt_eval_count": 5,
                    "prompt_eval_duration": 20,
                    "eval_count": 3,
                    "eval_duration": 70,
                    "future_field": { "not_exposed_by_rig": true }
                }),
            ];
            let ndjson = responses
                .into_iter()
                .map(|response| format!("{response}\n"))
                .collect::<String>();
            let chunks = ndjson
                .as_bytes()
                .chunks(3)
                .map(|chunk| Ok::<_, rig_core::http_client::Error>(chunk.to_vec().into()))
                .collect();
            let http_client = SequencedStreamingHttpClient::new(chunks);
            let (config, credential, request_mapper) =
                validate_config(config, credential).expect("Ollama stream config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client)
                .expect("Ollama stream bridge must construct");
            let mut stream = bridge
                .stream(CompletionRequest {
                    messages: vec![Message::user("hello")],
                    ..CompletionRequest::default()
                })
                .await
                .expect("Ollama stream must start");
            let mut events = Vec::new();
            while let Some(event) = stream.next().await {
                events.push(event.expect("Ollama NDJSON item must convert"));
            }

            let response = validate_stream_events(&events)
                .expect("Ollama must satisfy the shared streaming contract");
            assert_eq!(response.finish_reason, Some(FinishReason::Stop));
            assert_eq!(
                response.usage,
                Some(TokenUsage {
                    input_tokens: Some(5),
                    output_tokens: Some(3),
                    total_tokens: Some(8),
                    cached_input_tokens: Some(0),
                })
            );
            assert_eq!(response.provider_metadata["total_duration_ns"], 100);
            assert!(matches!(
                &response.content[0],
                AssistantContent::Text(text) if text.text == "你好"
            ));
            let calls = response
                .content
                .iter()
                .filter_map(|content| match content {
                    AssistantContent::ToolCall(call) => Some(call),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(calls.len(), 2);
            assert_eq!(calls[0].name, "lookup");
            assert_eq!(calls[1].name, "lookup");
            assert_ne!(calls[0].id, calls[1].id);
            assert!(events.iter().all(|event| !matches!(
                event,
                CompletionEvent::ProviderEvent { data }
                    if data.kind == "unknown_stream_item"
            )));
        });
    }
}
