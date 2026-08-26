use std::sync::Arc;

use armillae_core::FinishReason;
use armillae_llm::{
    BridgeCapabilities, BridgeConfig, BridgeError, ErrorMetadata, LlmBridge,
    OutputFormatCapabilities, ToolChoiceCapabilities,
};
use rig_core::{client::CompletionClient, http_client::HttpClientExt, providers::deepseek};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, Value};

use crate::{
    RigBridge,
    request::OpenAiRequestMapper,
    response::{NormalizedResponseFacts, RigResponseNormalizer},
};

use super::{build_http_client, validate_named_config};

pub(crate) fn create(
    config: BridgeConfig,
    credential: Option<SecretString>,
) -> Result<Arc<dyn LlmBridge>, BridgeError> {
    let (config, credential, request_mapper) =
        validate_named_config(config, credential, "deepseek", "DeepSeek")?;
    let http_client = build_http_client(&config)?;

    create_validated(config, credential, request_mapper, http_client)
}

fn create_validated<H>(
    config: BridgeConfig,
    credential: SecretString,
    request_mapper: OpenAiRequestMapper,
    http_client: H,
) -> Result<Arc<dyn LlmBridge>, BridgeError>
where
    H: HttpClientExt + Clone + Default + std::fmt::Debug + 'static,
{
    let mut client_builder = deepseek::Client::builder()
        .api_key(credential.expose_secret().to_owned())
        .http_client(http_client);
    if let Some(endpoint) = &config.endpoint {
        client_builder = client_builder.base_url(endpoint.as_str());
    }
    let client = client_builder
        .build()
        .map_err(|_| BridgeError::InvalidConfiguration {
            message: "failed to construct Rig DeepSeek client".to_owned(),
        })?;
    let model_name = config.model.clone();
    let model = client.completion_model(config.model);
    let bridge = RigBridge::new(
        model,
        model_name,
        capabilities(),
        config.defaults,
        Arc::new(request_mapper),
        Arc::new(DeepSeekResponseNormalizer),
    )?;

    Ok(Arc::new(bridge))
}

const fn capabilities() -> BridgeCapabilities {
    BridgeCapabilities {
        streaming: true,
        tool_calling: true,
        parallel_tool_calls: true,
        tool_choice: ToolChoiceCapabilities {
            auto: true,
            none: true,
            required: false,
            specific: false,
        },
        output_format: OutputFormatCapabilities {
            json_object: true,
            json_schema: false,
        },
        system_message: true,
        developer_message: false,
    }
}

#[derive(Clone, Copy, Debug)]
struct DeepSeekResponseNormalizer;

impl RigResponseNormalizer<deepseek::CompletionResponse> for DeepSeekResponseNormalizer {
    fn provider(&self) -> &str {
        "deepseek"
    }

    fn normalize(
        &self,
        raw_response: &deepseek::CompletionResponse,
    ) -> Result<NormalizedResponseFacts, BridgeError> {
        validate_optional_fact(raw_response.id.as_deref(), "response id")?;
        validate_optional_fact(raw_response.model.as_deref(), "response model")?;
        let choice =
            raw_response
                .choices
                .first()
                .ok_or_else(|| BridgeError::InvalidProviderResponse {
                    message: "DeepSeek response contained no choices".to_owned(),
                    metadata: ErrorMetadata::new("deepseek"),
                })?;

        let mut metadata = Map::new();
        if let Some(fingerprint) = &raw_response.system_fingerprint {
            metadata.insert(
                "system_fingerprint".to_owned(),
                Value::String(fingerprint.clone()),
            );
        }

        Ok(NormalizedResponseFacts {
            id: raw_response.id.clone(),
            model: raw_response.model.clone(),
            finish_reason: Some(finish_reason(&choice.finish_reason)),
            provider_metadata: Value::Object(metadata),
        })
    }
}

fn validate_optional_fact(value: Option<&str>, name: &str) -> Result<(), BridgeError> {
    if value.is_some_and(|value| value.trim().is_empty()) {
        return Err(BridgeError::InvalidProviderResponse {
            message: format!("DeepSeek {name} is empty"),
            metadata: ErrorMetadata::new("deepseek"),
        });
    }
    Ok(())
}

fn finish_reason(reason: &str) -> FinishReason {
    match reason {
        "stop" => FinishReason::Stop,
        "length" | "max_tokens" => FinishReason::Length,
        "tool_calls" | "function_call" => FinishReason::ToolCall,
        "content_filter" => FinishReason::ContentFilter,
        "cancelled" => FinishReason::Cancelled,
        other => FinishReason::Unknown(other.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use armillae_core::{
        AssistantContent, CompletionRequest, CompletionResponse, FinishReason, Message,
        OutputFormat, TextContent, TokenUsage, ToolChoice, ToolDefinition,
    };
    use armillae_llm::{
        BridgeError,
        mock::contract::{validate_stream_events, verify_completion},
    };
    use futures::StreamExt;
    use rig_core::test_utils::RecordingHttpClient;
    use serde_json::{Value, json};

    use super::{
        DeepSeekResponseNormalizer, RigResponseNormalizer, capabilities, create, create_validated,
    };
    use crate::providers::{
        test_support::{
            expected_text_stream, resolved_config, streaming_client, text_stream_client,
        },
        validate_named_config,
    };

    #[test]
    fn configuration_and_capability_profile_are_explicit() {
        let (config, credential) = resolved_config("deepseek", None, true, json!({}));
        let bridge = create(config, credential)
            .expect("valid DeepSeek configuration must construct a bridge");
        let (config, credential) = resolved_config("deepseek", None, false, json!({}));
        let missing_credential = create(config, credential);
        let (config, credential) =
            resolved_config("deepseek", None, true, json!({ "thinking": "disabled" }));
        let unknown_options = create(config, credential);

        assert_eq!(bridge.capabilities(), capabilities());
        assert!(bridge.capabilities().streaming);
        assert!(matches!(
            missing_credential,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
        assert!(matches!(
            unknown_options,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
    }

    #[test]
    fn deepseek_streams_over_its_native_openai_compatible_client() {
        let (config, credential) =
            resolved_config("deepseek", Some("http://deepseek.test"), true, json!({}));
        futures::executor::block_on(async {
            let (config, credential, request_mapper) =
                validate_named_config(config, credential, "deepseek", "DeepSeek")
                    .expect("DeepSeek stream config must validate");
            let bridge = create_validated(config, credential, request_mapper, text_stream_client())
                .expect("DeepSeek stream bridge must construct");
            let request = CompletionRequest {
                messages: vec![Message::user("hello")],
                ..CompletionRequest::default()
            };

            let mut stream = bridge
                .stream(request)
                .await
                .expect("DeepSeek stream must start");
            let mut events = Vec::new();
            while let Some(event) = stream.next().await {
                events.push(event.expect("DeepSeek stream item must convert"));
            }
            let response = validate_stream_events(&events)
                .expect("DeepSeek must satisfy the shared streaming event contract");
            assert_eq!(response, &expected_text_stream());
        });
    }

    #[test]
    fn deepseek_stream_preserves_reasoning_and_cached_usage() {
        let (config, credential) =
            resolved_config("deepseek", Some("http://deepseek.test"), true, json!({}));
        futures::executor::block_on(async {
            let events = vec![
                json!({
                    "id": "deepseek-stream",
                    "model": "deepseek-reasoner",
                    "choices": [{
                        "delta": { "reasoning_content": "思" },
                        "finish_reason": null
                    }],
                    "usage": null
                }),
                json!({
                    "id": "deepseek-stream",
                    "model": "deepseek-reasoner",
                    "choices": [{
                        "delta": { "reasoning_content": "考" },
                        "finish_reason": null
                    }],
                    "usage": null
                }),
                json!({
                    "id": "deepseek-stream",
                    "model": "deepseek-reasoner",
                    "choices": [{
                        "delta": { "content": "答案" },
                        "finish_reason": null
                    }],
                    "usage": null
                }),
                json!({
                    "id": "deepseek-stream",
                    "model": "deepseek-reasoner",
                    "choices": [{
                        "delta": {},
                        "finish_reason": "stop"
                    }],
                    "usage": null
                }),
                json!({
                    "id": "deepseek-stream",
                    "model": "deepseek-reasoner",
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 6,
                        "completion_tokens": 4,
                        "total_tokens": 10,
                        "prompt_cache_hit_tokens": 2,
                        "prompt_cache_miss_tokens": 4,
                        "prompt_tokens_details": { "cached_tokens": 2 },
                        "completion_tokens_details": { "reasoning_tokens": 2 }
                    }
                }),
            ];
            let (config, credential, request_mapper) =
                validate_named_config(config, credential, "deepseek", "DeepSeek")
                    .expect("DeepSeek reasoning stream config must validate");
            let bridge =
                create_validated(config, credential, request_mapper, streaming_client(events))
                    .expect("DeepSeek reasoning stream bridge must construct");
            let mut stream = bridge
                .stream(CompletionRequest {
                    messages: vec![Message::user("reason")],
                    ..CompletionRequest::default()
                })
                .await
                .expect("DeepSeek reasoning stream must start");
            let mut observed = Vec::new();
            while let Some(event) = stream.next().await {
                observed.push(event.expect("DeepSeek reasoning item must convert"));
            }
            let response = validate_stream_events(&observed)
                .expect("DeepSeek reasoning events must satisfy the shared contract");

            assert!(matches!(
                &response.content[0],
                AssistantContent::ProviderData(data)
                    if data.provider == "deepseek"
                        && data.kind == "reasoning"
                        && data.value["content"][0]["content"]["text"] == "思考"
            ));
            assert!(matches!(
                &response.content[1],
                AssistantContent::Text(text) if text.text == "答案"
            ));
            assert_eq!(
                response
                    .usage
                    .as_ref()
                    .expect("DeepSeek stream usage must be retained")
                    .cached_input_tokens,
                Some(2)
            );
        });
    }

    #[test]
    fn openai_wire_flattens_messages_and_satisfies_shared_completion_contract() {
        let (config, credential) =
            resolved_config("deepseek", Some("http://deepseek.test"), true, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(deepseek_text_response().to_string());
            let (config, credential, request_mapper) =
                validate_named_config(config, credential, "deepseek", "DeepSeek")
                    .expect("DeepSeek test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client.clone())
                .expect("DeepSeek test bridge must construct");
            let request = CompletionRequest {
                messages: vec![Message::user("hello")],
                output_format: Some(OutputFormat::JsonObject),
                ..CompletionRequest::default()
            };
            let expected = CompletionResponse {
                id: Some("chatcmpl-deepseek".to_owned()),
                model: Some("deepseek-chat".to_owned()),
                content: vec![AssistantContent::Text(TextContent::new("hello"))],
                finish_reason: Some(FinishReason::Stop),
                usage: Some(TokenUsage {
                    input_tokens: Some(5),
                    output_tokens: Some(2),
                    total_tokens: Some(7),
                    cached_input_tokens: Some(3),
                }),
                provider_metadata: json!({ "system_fingerprint": "fp-deepseek" }),
            };

            verify_completion(bridge.as_ref(), request, &expected)
                .await
                .expect("DeepSeek bridge must satisfy the shared completion contract");

            let requests = http_client.requests();
            let captured = requests
                .first()
                .expect("DeepSeek bridge must issue one request");
            assert_eq!(captured.uri, "http://deepseek.test/chat/completions");
            assert_eq!(
                captured
                    .headers
                    .get("authorization")
                    .expect("DeepSeek client must attach bearer authorization")
                    .to_str()
                    .expect("test authorization header must be text"),
                "Bearer named-provider-test-secret"
            );
            let body: Value = serde_json::from_slice(&captured.body)
                .expect("captured DeepSeek request must be JSON");
            assert_eq!(body["model"], "test-model");
            assert_eq!(body["messages"][0]["content"], "hello");
            assert_eq!(body["response_format"]["type"], "json_object");
        });
    }

    #[test]
    fn tool_calls_reasoning_and_cached_usage_are_preserved() {
        let (config, credential) =
            resolved_config("deepseek", Some("http://deepseek.test"), true, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(
                json!({
                    "id": "chatcmpl-deepseek-tools",
                    "model": "deepseek-reasoner",
                    "system_fingerprint": null,
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "",
                            "reasoning_content": "thinking",
                            "tool_calls": [{
                                "id": "call-weather",
                                "index": 0,
                                "type": "function",
                                "function": {
                                    "name": "weather",
                                    "arguments": "{\"city\":\"上海\"}"
                                }
                            }]
                        },
                        "logprobs": null,
                        "finish_reason": "tool_calls"
                    }],
                    "usage": {
                        "prompt_tokens": 8,
                        "completion_tokens": 4,
                        "prompt_cache_hit_tokens": 6,
                        "prompt_cache_miss_tokens": 2,
                        "total_tokens": 12,
                        "prompt_tokens_details": { "cached_tokens": 6 }
                    }
                })
                .to_string(),
            );
            let (config, credential, request_mapper) =
                validate_named_config(config, credential, "deepseek", "DeepSeek")
                    .expect("DeepSeek test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client)
                .expect("DeepSeek test bridge must construct");

            let response = bridge
                .complete(CompletionRequest {
                    messages: vec![Message::user("use weather")],
                    tools: vec![tool_definition()],
                    tool_choice: Some(ToolChoice::Auto),
                    ..CompletionRequest::default()
                })
                .await
                .expect("DeepSeek must return its ToolCall response");

            assert_eq!(response.finish_reason, Some(FinishReason::ToolCall));
            assert_eq!(
                response
                    .tool_calls()
                    .map(|call| call.id.as_str())
                    .collect::<Vec<_>>(),
                ["call-weather"]
            );
            assert!(response.content.iter().any(|content| {
                matches!(
                    content,
                    AssistantContent::ProviderData(data)
                        if data.provider == "deepseek" && data.kind == "reasoning"
                )
            }));
            assert_eq!(
                response
                    .usage
                    .expect("DeepSeek usage must be present")
                    .cached_input_tokens,
                Some(6)
            );
        });
    }

    #[test]
    fn unsupported_tool_choices_and_json_schema_fail_before_transport() {
        let (config, credential) =
            resolved_config("deepseek", Some("http://deepseek.test"), true, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(deepseek_text_response().to_string());
            let (config, credential, request_mapper) =
                validate_named_config(config, credential, "deepseek", "DeepSeek")
                    .expect("DeepSeek test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client.clone())
                .expect("DeepSeek test bridge must construct");

            let specific = bridge
                .complete(CompletionRequest {
                    messages: vec![Message::user("hello")],
                    tools: vec![tool_definition()],
                    tool_choice: Some(ToolChoice::Specific {
                        name: "weather".to_owned(),
                    }),
                    ..CompletionRequest::default()
                })
                .await;
            let schema = bridge
                .complete(CompletionRequest {
                    messages: vec![Message::user("hello")],
                    output_format: Some(OutputFormat::JsonSchema {
                        name: "answer".to_owned(),
                        schema: json!({ "type": "object" }),
                        strict: true,
                    }),
                    ..CompletionRequest::default()
                })
                .await;

            assert!(matches!(
                specific,
                Err(BridgeError::UnsupportedCapability { capability })
                    if capability == "tool_choice.specific"
            ));
            assert!(matches!(
                schema,
                Err(BridgeError::UnsupportedCapability { capability })
                    if capability == "output_format.json_schema"
            ));
            assert!(http_client.requests().is_empty());
        });
    }

    #[test]
    fn provider_rejection_keeps_deepseek_identity() {
        let (config, credential) =
            resolved_config("deepseek", Some("http://deepseek.test"), true, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::with_error_response(
                "400".parse().expect("400 must be a valid HTTP status code"),
                json!({ "error": { "message": "unsupported" } }).to_string(),
            );
            let (config, credential, request_mapper) =
                validate_named_config(config, credential, "deepseek", "DeepSeek")
                    .expect("DeepSeek test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client)
                .expect("DeepSeek test bridge must construct");

            let error = bridge
                .complete(CompletionRequest {
                    messages: vec![Message::user("hello")],
                    ..CompletionRequest::default()
                })
                .await
                .expect_err("remote rejection must remain explicit");

            assert!(matches!(
                error,
                BridgeError::ProviderRejected { metadata, .. }
                    if metadata.provider == "deepseek" && metadata.http_status == Some(400)
            ));
        });
    }

    #[test]
    fn normalizer_preserves_missing_facts_and_unknown_finish_reason() {
        let raw: rig_core::providers::deepseek::CompletionResponse =
            serde_json::from_value(json!({
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "hello" },
                    "logprobs": null,
                    "finish_reason": "future_reason"
                }],
                "usage": {
                    "prompt_tokens": 1,
                    "completion_tokens": 1,
                    "prompt_cache_hit_tokens": 0,
                    "prompt_cache_miss_tokens": 1,
                    "total_tokens": 2
                }
            }))
            .expect("DeepSeek response fixture must deserialize");

        let facts = DeepSeekResponseNormalizer
            .normalize(&raw)
            .expect("missing optional facts remain valid");

        assert_eq!(facts.id, None);
        assert_eq!(facts.model, None);
        assert_eq!(
            facts.finish_reason,
            Some(FinishReason::Unknown("future_reason".to_owned()))
        );
    }

    fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: "weather".to_owned(),
            description: "Get weather".to_owned(),
            input_schema: json!({ "type": "object" }),
        }
    }

    fn deepseek_text_response() -> Value {
        json!({
            "id": "chatcmpl-deepseek",
            "model": "deepseek-chat",
            "system_fingerprint": "fp-deepseek",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "hello" },
                "logprobs": null,
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 5,
                "completion_tokens": 2,
                "prompt_cache_hit_tokens": 3,
                "prompt_cache_miss_tokens": 2,
                "total_tokens": 7,
                "prompt_tokens_details": { "cached_tokens": 3 }
            }
        })
    }
}
