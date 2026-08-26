use std::sync::Arc;

use armillae_core::FinishReason;
use armillae_llm::{
    BridgeCapabilities, BridgeConfig, BridgeError, ErrorMetadata, LlmBridge,
    OutputFormatCapabilities, ToolChoiceCapabilities,
};
use rig_core::{client::CompletionClient, http_client::HttpClientExt, providers::anthropic};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value, json};

use crate::{
    RigBridge,
    request::AnthropicRequestMapper,
    response::{NormalizedResponseFacts, RigResponseNormalizer},
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
) -> Result<(BridgeConfig, SecretString, AnthropicRequestMapper), BridgeError> {
    if config.provider != "anthropic" {
        return invalid_configuration(format!(
            "Anthropic provider module cannot construct provider: {}",
            config.provider
        ));
    }
    if config.defaults.seed.is_some() {
        return invalid_configuration("Anthropic generation defaults cannot set seed");
    }
    let credential = credential.ok_or_else(|| BridgeError::InvalidConfiguration {
        message: "anthropic requires a credential".to_owned(),
    })?;
    let request_mapper = AnthropicRequestMapper::new(config.provider_options.clone())?;

    Ok((config, credential, request_mapper))
}

fn create_validated<H>(
    config: BridgeConfig,
    credential: SecretString,
    request_mapper: AnthropicRequestMapper,
    http_client: H,
) -> Result<Arc<dyn LlmBridge>, BridgeError>
where
    H: HttpClientExt + Clone + Default + std::fmt::Debug + 'static,
{
    let mut client_builder = anthropic::Client::builder()
        .api_key(credential.expose_secret().to_owned())
        .http_client(http_client);
    if let Some(endpoint) = &config.endpoint {
        client_builder = client_builder.base_url(endpoint.as_str());
    }
    let client = client_builder
        .build()
        .map_err(|_| BridgeError::InvalidConfiguration {
            message: "failed to construct Rig Anthropic client".to_owned(),
        })?;
    let model = client.completion_model(config.model);
    let bridge = RigBridge::new(
        model,
        capabilities(),
        config.defaults,
        Arc::new(request_mapper),
        Arc::new(AnthropicResponseNormalizer),
    )?;

    Ok(Arc::new(bridge))
}

const fn capabilities() -> BridgeCapabilities {
    BridgeCapabilities {
        streaming: true,
        tool_calling: true,
        parallel_tool_calls: true,
        tool_choice: ToolChoiceCapabilities::all(),
        output_format: OutputFormatCapabilities {
            json_object: false,
            json_schema: true,
        },
        system_message: true,
        developer_message: false,
    }
}

#[derive(Clone, Copy, Debug)]
struct AnthropicResponseNormalizer;

impl RigResponseNormalizer<anthropic::completion::CompletionResponse>
    for AnthropicResponseNormalizer
{
    fn provider(&self) -> &str {
        "anthropic"
    }

    fn normalize(
        &self,
        raw_response: &anthropic::completion::CompletionResponse,
    ) -> Result<NormalizedResponseFacts, BridgeError> {
        if raw_response.id.trim().is_empty() {
            return invalid_provider_response("Anthropic response id is empty");
        }
        if raw_response.model.trim().is_empty() {
            return invalid_provider_response("Anthropic response model is empty");
        }
        if raw_response.role != "assistant" {
            return invalid_provider_response("Anthropic response role is not assistant");
        }

        let mut metadata = Map::new();
        if let Some(stop_sequence) = &raw_response.stop_sequence {
            metadata.insert(
                "stop_sequence".to_owned(),
                Value::String(stop_sequence.clone()),
            );
        }
        if let Some(tokens) = raw_response.usage.cache_creation_input_tokens {
            metadata.insert("cache_creation_input_tokens".to_owned(), json!(tokens));
        }

        Ok(NormalizedResponseFacts {
            id: Some(raw_response.id.clone()),
            model: Some(raw_response.model.clone()),
            finish_reason: raw_response
                .stop_reason
                .as_deref()
                .map(anthropic_finish_reason),
            provider_metadata: Value::Object(metadata),
        })
    }
}

fn anthropic_finish_reason(reason: &str) -> FinishReason {
    match reason {
        "end_turn" | "stop_sequence" => FinishReason::Stop,
        "max_tokens" => FinishReason::Length,
        "tool_use" => FinishReason::ToolCall,
        other => FinishReason::Unknown(other.to_owned()),
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
        metadata: ErrorMetadata::new("anthropic"),
    })
}

#[cfg(test)]
mod tests {
    use armillae_core::{
        AssistantContent, CompletionEvent, CompletionRequest, CompletionResponse, ContentPart,
        FinishReason, GenerationOptions, Message, OutputFormat, ProviderExtensions, Role,
        TextContent, TokenUsage, ToolCall, ToolCallId, ToolChoice, ToolDefinition, ToolResult,
        ToolResultContent,
    };
    use armillae_llm::{
        BridgeError,
        mock::contract::{validate_stream_events, verify_completion, verify_stream},
    };
    use futures::StreamExt;
    use rig_core::test_utils::{RecordingHttpClient, SequencedStreamingHttpClient};
    use serde_json::{Value, json};

    use super::{
        AnthropicResponseNormalizer, RigResponseNormalizer, capabilities, create, create_validated,
        validate_config,
    };
    use crate::providers::test_support::resolved_config;

    fn tool_call_id(value: &str) -> ToolCallId {
        ToolCallId::new(value).expect("fixture ToolCall IDs are non-empty")
    }

    fn generation() -> GenerationOptions {
        GenerationOptions {
            max_output_tokens: Some(512),
            ..GenerationOptions::default()
        }
    }

    fn strict_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "answer": { "type": "string" }
            },
            "required": ["answer"],
            "additionalProperties": false
        })
    }

    fn text_response() -> Value {
        json!({
            "type": "message",
            "id": "msg-anthropic",
            "model": "claude-test",
            "role": "assistant",
            "content": [{ "type": "text", "text": "hello" }],
            "stop_reason": "stop_sequence",
            "stop_sequence": "END",
            "usage": {
                "input_tokens": 3,
                "cache_read_input_tokens": 2,
                "cache_creation_input_tokens": 1,
                "output_tokens": 2
            }
        })
    }

    fn streaming_client(events: Vec<Value>) -> SequencedStreamingHttpClient {
        let sse = events
            .into_iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>();
        let chunks = sse
            .as_bytes()
            .chunks(4)
            .map(|chunk| Ok::<_, rig_core::http_client::Error>(chunk.to_vec().into()))
            .collect();
        SequencedStreamingHttpClient::new(chunks)
    }

    fn message_start() -> Value {
        json!({
            "type": "message_start",
            "message": {
                "id": "msg-stream",
                "role": "assistant",
                "content": [],
                "model": "claude-test",
                "stop_reason": null,
                "stop_sequence": null,
                "usage": {
                    "input_tokens": 3,
                    "cache_read_input_tokens": 1,
                    "cache_creation_input_tokens": 1,
                    "output_tokens": 0
                }
            }
        })
    }

    fn message_delta(stop_reason: &str, output_tokens: usize) -> Value {
        json!({
            "type": "message_delta",
            "delta": {
                "stop_reason": stop_reason,
                "stop_sequence": null
            },
            "usage": {
                "output_tokens": output_tokens,
                "cache_read_input_tokens": 1,
                "cache_creation_input_tokens": 1
            }
        })
    }

    fn stream_usage(output_tokens: u64) -> TokenUsage {
        TokenUsage {
            input_tokens: Some(3),
            output_tokens: Some(output_tokens),
            total_tokens: Some(3 + 1 + 1 + output_tokens),
            cached_input_tokens: Some(1),
        }
    }

    #[test]
    fn configuration_and_capability_profile_are_explicit() {
        let (config, credential) = resolved_config("anthropic", None, true, json!({}));
        let bridge = create(config, credential)
            .expect("valid Anthropic configuration must construct a bridge");
        let (config, credential) = resolved_config("anthropic", None, false, json!({}));
        let missing_credential = create(config, credential);
        let (config, credential) =
            resolved_config("anthropic", None, true, json!({ "thinking": true }));
        let unknown_options = create(config, credential);
        let (mut config, credential) = resolved_config("anthropic", None, true, json!({}));
        config.defaults.seed = Some(7);
        let unsupported_defaults = create(config, credential);

        assert_eq!(bridge.capabilities(), capabilities());
        assert!(bridge.capabilities().streaming);
        assert!(!bridge.capabilities().output_format.json_object);
        assert!(bridge.capabilities().output_format.json_schema);
        assert!(matches!(
            missing_credential,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
        assert!(matches!(
            unknown_options,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
        assert!(matches!(
            unsupported_defaults,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
    }

    #[test]
    fn native_wire_and_response_satisfy_the_shared_completion_contract() {
        let (config, credential) = resolved_config(
            "anthropic",
            Some("http://anthropic.test/v1/messages"),
            true,
            json!({}),
        );
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(text_response().to_string());
            let (config, credential, request_mapper) =
                validate_config(config, credential).expect("Anthropic test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client.clone())
                .expect("Anthropic test bridge must construct");
            let request = CompletionRequest {
                messages: vec![
                    Message::new(Role::System, vec![ContentPart::text("rules")]),
                    Message::user("hello"),
                ],
                tools: vec![ToolDefinition {
                    name: "lookup".to_owned(),
                    description: "Lookup a value".to_owned(),
                    input_schema: strict_schema(),
                }],
                tool_choice: Some(ToolChoice::Specific {
                    name: "lookup".to_owned(),
                }),
                output_format: Some(OutputFormat::JsonSchema {
                    name: "answer".to_owned(),
                    schema: strict_schema(),
                    strict: true,
                }),
                generation: GenerationOptions {
                    temperature: Some(0.25),
                    max_output_tokens: Some(512),
                    stop: vec!["END".to_owned()],
                    seed: None,
                },
                ..CompletionRequest::default()
            };
            let expected = CompletionResponse {
                id: Some("msg-anthropic".to_owned()),
                model: Some("claude-test".to_owned()),
                content: vec![AssistantContent::Text(TextContent::new("hello"))],
                finish_reason: Some(FinishReason::Stop),
                usage: Some(TokenUsage {
                    input_tokens: Some(3),
                    output_tokens: Some(2),
                    total_tokens: Some(8),
                    cached_input_tokens: Some(2),
                }),
                provider_metadata: json!({
                    "stop_sequence": "END",
                    "cache_creation_input_tokens": 1
                }),
            };

            verify_completion(bridge.as_ref(), request, &expected)
                .await
                .expect("Anthropic bridge must satisfy the shared completion contract");

            let requests = http_client.requests();
            let captured = requests
                .first()
                .expect("Anthropic bridge must issue one request");
            assert_eq!(captured.uri, "http://anthropic.test/v1/messages");
            assert_eq!(
                captured
                    .headers
                    .get("x-api-key")
                    .expect("Anthropic client must attach x-api-key")
                    .to_str()
                    .expect("test credential header must be text"),
                "named-provider-test-secret"
            );
            assert_eq!(
                captured
                    .headers
                    .get("anthropic-version")
                    .expect("Anthropic version header must be present"),
                "2023-06-01"
            );
            let body: Value = serde_json::from_slice(&captured.body)
                .expect("captured Anthropic request must be JSON");
            assert_eq!(body["model"], "test-model");
            assert_eq!(body["max_tokens"], 512);
            assert_eq!(body["system"][0]["text"], "rules");
            assert_eq!(body["messages"][0]["role"], "user");
            assert_eq!(body["messages"][0]["content"][0]["text"], "hello");
            assert_eq!(body["stop_sequences"], json!(["END"]));
            assert_eq!(
                body["tool_choice"],
                json!({ "type": "tool", "name": "lookup" })
            );
            assert_eq!(body["tools"][0]["name"], "lookup");
            assert_eq!(body["output_config"]["format"]["type"], "json_schema");
            assert_eq!(
                body["output_config"]["format"]["schema"]["additionalProperties"],
                false
            );
        });
    }

    #[test]
    fn multiple_tool_calls_and_follow_up_results_keep_order_and_ids() {
        let response = json!({
            "type": "message",
            "id": "msg-tools",
            "model": "claude-test",
            "role": "assistant",
            "content": [
                { "type": "tool_use", "id": "tool-weather", "name": "weather", "input": { "city": "上海" } },
                { "type": "tool_use", "id": "tool-dice", "name": "dice", "input": { "sides": 20 } }
            ],
            "stop_reason": "tool_use",
            "stop_sequence": null,
            "usage": { "input_tokens": 4, "output_tokens": 3 }
        });
        let (config, credential) =
            resolved_config("anthropic", Some("http://anthropic.test"), true, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(response.to_string());
            let (config, credential, request_mapper) =
                validate_config(config, credential).expect("Anthropic tool config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client.clone())
                .expect("Anthropic tool bridge must construct");
            let request = CompletionRequest {
                messages: vec![Message::user("use tools")],
                generation: generation(),
                ..CompletionRequest::default()
            };
            let expected = CompletionResponse {
                id: Some("msg-tools".to_owned()),
                model: Some("claude-test".to_owned()),
                content: vec![
                    AssistantContent::ToolCall(ToolCall {
                        id: tool_call_id("tool-weather"),
                        name: "weather".to_owned(),
                        arguments: json!({ "city": "上海" }),
                    }),
                    AssistantContent::ToolCall(ToolCall {
                        id: tool_call_id("tool-dice"),
                        name: "dice".to_owned(),
                        arguments: json!({ "sides": 20 }),
                    }),
                ],
                finish_reason: Some(FinishReason::ToolCall),
                usage: Some(TokenUsage {
                    input_tokens: Some(4),
                    output_tokens: Some(3),
                    total_tokens: Some(7),
                    cached_input_tokens: Some(0),
                }),
                provider_metadata: json!({}),
            };
            verify_completion(bridge.as_ref(), request, &expected)
                .await
                .expect("Anthropic ToolCalls must satisfy the shared completion contract");

            let follow_up = CompletionRequest {
                messages: vec![
                    Message::assistant(vec![
                        ContentPart::ToolCall(ToolCall {
                            id: tool_call_id("tool-weather"),
                            name: "weather".to_owned(),
                            arguments: json!({ "city": "上海" }),
                        }),
                        ContentPart::ToolCall(ToolCall {
                            id: tool_call_id("tool-dice"),
                            name: "dice".to_owned(),
                            arguments: json!({ "sides": 20 }),
                        }),
                    ]),
                    Message::new(
                        Role::Tool,
                        vec![
                            ContentPart::ToolResult(ToolResult {
                                call_id: tool_call_id("tool-weather"),
                                content: vec![ToolResultContent::Text {
                                    text: "sunny".to_owned(),
                                }],
                                is_error: false,
                            }),
                            ContentPart::ToolResult(ToolResult {
                                call_id: tool_call_id("tool-dice"),
                                content: vec![ToolResultContent::Json {
                                    value: json!({ "value": 17 }),
                                }],
                                is_error: false,
                            }),
                        ],
                    ),
                ],
                generation: generation(),
                ..CompletionRequest::default()
            };
            bridge
                .complete(follow_up)
                .await
                .expect("non-error Anthropic ToolResults must remain sendable");

            let requests = http_client.requests();
            let body: Value = serde_json::from_slice(
                &requests
                    .get(1)
                    .expect("follow-up must issue a second request")
                    .body,
            )
            .expect("follow-up request must be JSON");
            assert_eq!(body["messages"][0]["content"][0]["id"], "tool-weather");
            assert_eq!(body["messages"][0]["content"][1]["id"], "tool-dice");
            assert_eq!(
                body["messages"][1]["content"][0]["tool_use_id"],
                "tool-weather"
            );
            assert_eq!(
                body["messages"][1]["content"][1]["tool_use_id"],
                "tool-dice"
            );
            assert_eq!(body["messages"][1]["content"][0].get("is_error"), None);
            assert_eq!(body["messages"][1]["content"][1].get("is_error"), None);
            assert_eq!(
                body["messages"][1]["content"][1]["content"][0]["text"],
                "{\"value\":17}"
            );
        });
    }

    #[test]
    fn unsupported_or_lossy_requests_are_rejected_before_transport() {
        let (config, credential) =
            resolved_config("anthropic", Some("http://anthropic.test"), true, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(text_response().to_string());
            let (config, credential, request_mapper) = validate_config(config, credential)
                .expect("Anthropic validation config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client.clone())
                .expect("Anthropic validation bridge must construct");

            let missing_max = CompletionRequest {
                messages: vec![Message::user("hello")],
                ..CompletionRequest::default()
            };
            let mid_system = CompletionRequest {
                messages: vec![
                    Message::user("hello"),
                    Message::new(Role::System, vec![ContentPart::text("late")]),
                ],
                generation: generation(),
                ..CompletionRequest::default()
            };
            let error_result = CompletionRequest {
                messages: vec![Message::tool_result(ToolResult {
                    call_id: tool_call_id("tool-error"),
                    content: vec![ToolResultContent::Text {
                        text: "failed".to_owned(),
                    }],
                    is_error: true,
                })],
                generation: generation(),
                ..CompletionRequest::default()
            };
            let seeded = CompletionRequest {
                messages: vec![Message::user("hello")],
                generation: GenerationOptions {
                    seed: Some(7),
                    ..generation()
                },
                ..CompletionRequest::default()
            };
            let non_strict_schema = CompletionRequest {
                messages: vec![Message::user("hello")],
                output_format: Some(OutputFormat::JsonSchema {
                    name: "answer".to_owned(),
                    schema: strict_schema(),
                    strict: false,
                }),
                generation: generation(),
                ..CompletionRequest::default()
            };
            let lossy_schema = CompletionRequest {
                messages: vec![Message::user("hello")],
                output_format: Some(OutputFormat::JsonSchema {
                    name: "answer".to_owned(),
                    schema: json!({
                        "type": "object",
                        "properties": { "score": { "type": "number", "minimum": 0 } },
                        "required": ["score"],
                        "additionalProperties": false
                    }),
                    strict: true,
                }),
                generation: generation(),
                ..CompletionRequest::default()
            };
            let extension = CompletionRequest {
                messages: vec![Message::user("hello")],
                generation: generation(),
                extensions: ProviderExtensions {
                    values: [("anthropic.thinking".to_owned(), json!(true))]
                        .into_iter()
                        .collect(),
                },
                ..CompletionRequest::default()
            };

            for request in [
                missing_max,
                mid_system,
                error_result,
                lossy_schema,
                extension,
            ] {
                assert!(matches!(
                    bridge.complete(request).await,
                    Err(BridgeError::InvalidRequest { .. })
                ));
            }
            assert!(matches!(
                bridge.complete(seeded).await,
                Err(BridgeError::UnsupportedCapability { .. })
            ));
            assert!(matches!(
                bridge.complete(non_strict_schema).await,
                Err(BridgeError::UnsupportedCapability { .. })
            ));
            assert!(http_client.requests().is_empty());
        });
    }

    #[test]
    fn native_text_stream_ignores_rig_filtered_unknown_sse_and_keeps_usage() {
        let events = vec![
            message_start(),
            json!({ "type": "future_anthropic_event", "value": "ignored-by-rig" }),
            json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": { "type": "text", "text": "" }
            }),
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": "你" }
            }),
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": "好" }
            }),
            json!({ "type": "content_block_stop", "index": 0 }),
            message_delta("end_turn", 2),
        ];
        let (config, credential) =
            resolved_config("anthropic", Some("http://anthropic.test"), true, json!({}));
        futures::executor::block_on(async {
            let (config, credential, request_mapper) =
                validate_config(config, credential).expect("Anthropic stream config must validate");
            let bridge =
                create_validated(config, credential, request_mapper, streaming_client(events))
                    .expect("Anthropic stream bridge must construct");
            let request = CompletionRequest {
                messages: vec![Message::user("hello")],
                generation: generation(),
                ..CompletionRequest::default()
            };
            let expected = CompletionResponse {
                id: None,
                model: None,
                content: vec![AssistantContent::Text(TextContent::new("你好"))],
                finish_reason: None,
                usage: Some(stream_usage(2)),
                provider_metadata: json!({}),
            };

            let observed = verify_stream(bridge.as_ref(), request, &expected)
                .await
                .expect("Anthropic text stream must satisfy the shared contract");
            assert!(!observed.iter().any(|event| matches!(
                event,
                CompletionEvent::ProviderEvent { data }
                    if data.kind == "unknown_stream_item"
            )));
        });
    }

    #[test]
    fn native_stream_reassembles_tool_calls_and_signed_reasoning() {
        let tool_events = vec![
            message_start(),
            json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": { "type": "tool_use", "id": "tool-stream", "name": "weather", "input": {} }
            }),
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "input_json_delta", "partial_json": "{\"city\":\"上" }
            }),
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "input_json_delta", "partial_json": "海\"}" }
            }),
            json!({ "type": "content_block_stop", "index": 0 }),
            message_delta("tool_use", 3),
        ];
        let reasoning_events = vec![
            message_start(),
            json!({
                "type": "content_block_start",
                "index": 0,
                "content_block": { "type": "thinking", "thinking": "", "signature": null }
            }),
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "thinking_delta", "thinking": "思" }
            }),
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "thinking_delta", "thinking": "考" }
            }),
            json!({
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "signature_delta", "signature": "signed" }
            }),
            json!({ "type": "content_block_stop", "index": 0 }),
            message_delta("end_turn", 2),
        ];
        let (tool_config, tool_credential) =
            resolved_config("anthropic", Some("http://anthropic.test"), true, json!({}));
        let (reasoning_config, reasoning_credential) =
            resolved_config("anthropic", Some("http://anthropic.test"), true, json!({}));

        futures::executor::block_on(async {
            let mut tool_stream = anthropic_stream(tool_config, tool_credential, tool_events).await;
            let mut tool_observed = Vec::new();
            while let Some(event) = tool_stream.next().await {
                tool_observed.push(event.expect("Anthropic tool stream item must convert"));
            }
            let tool_response = validate_stream_events(&tool_observed)
                .expect("Anthropic tool stream must satisfy the shared contract");
            assert_eq!(
                tool_response.content,
                vec![AssistantContent::ToolCall(ToolCall {
                    id: tool_call_id("tool-stream"),
                    name: "weather".to_owned(),
                    arguments: json!({ "city": "上海" }),
                })]
            );

            let mut reasoning_stream =
                anthropic_stream(reasoning_config, reasoning_credential, reasoning_events).await;
            let mut reasoning_observed = Vec::new();
            while let Some(event) = reasoning_stream.next().await {
                reasoning_observed
                    .push(event.expect("Anthropic reasoning stream item must convert"));
            }
            let reasoning_response = validate_stream_events(&reasoning_observed)
                .expect("Anthropic reasoning stream must satisfy the shared contract");
            assert_eq!(reasoning_response.content.len(), 1);
            assert!(matches!(
                &reasoning_response.content[0],
                AssistantContent::ProviderData(data)
                    if data.provider == "anthropic"
                        && data.kind == "reasoning"
                        && data.value["content"][0]["content"]["text"] == "思考"
                        && data.value["content"][0]["content"]["signature"] == "signed"
            ));
        });
    }

    async fn anthropic_stream(
        config: armillae_llm::BridgeConfig,
        credential: Option<armillae_llm::SecretString>,
        events: Vec<Value>,
    ) -> armillae_llm::CompletionStream {
        let (config, credential, request_mapper) =
            validate_config(config, credential).expect("Anthropic stream config must validate");
        let bridge = create_validated(config, credential, request_mapper, streaming_client(events))
            .expect("Anthropic stream bridge must construct");
        bridge
            .stream(CompletionRequest {
                messages: vec![Message::user("stream")],
                generation: generation(),
                ..CompletionRequest::default()
            })
            .await
            .expect("Anthropic stream must start")
    }

    #[test]
    fn response_normalizer_preserves_unknown_stop_reason_and_rejects_empty_facts() {
        let mut raw: rig_core::providers::anthropic::completion::CompletionResponse =
            serde_json::from_value(json!({
                "id": "msg-normalizer",
                "model": "claude-test",
                "role": "assistant",
                "content": [{ "type": "text", "text": "hello" }],
                "stop_reason": "future_reason",
                "stop_sequence": null,
                "usage": { "input_tokens": 1, "output_tokens": 1 }
            }))
            .expect("Anthropic normalizer fixture must deserialize");
        let facts = AnthropicResponseNormalizer
            .normalize(&raw)
            .expect("unknown Anthropic stop reasons remain valid");
        assert_eq!(
            facts.finish_reason,
            Some(FinishReason::Unknown("future_reason".to_owned()))
        );

        raw.id.clear();
        assert!(matches!(
            AnthropicResponseNormalizer.normalize(&raw),
            Err(BridgeError::InvalidProviderResponse { .. })
        ));

        let error = AnthropicResponseNormalizer.normalize_error(
            rig_core::completion::CompletionError::ProviderError(
                "sensitive Anthropic response".to_owned(),
            ),
        );
        assert!(matches!(
            &error,
            BridgeError::ProviderRejected { metadata, .. }
                if metadata.provider == "anthropic"
        ));
        assert!(!error.to_string().contains("sensitive Anthropic response"));
        assert_eq!(
            super::anthropic_finish_reason("max_tokens"),
            FinishReason::Length
        );
    }
}
