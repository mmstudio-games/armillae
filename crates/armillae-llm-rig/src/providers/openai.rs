use std::{sync::Arc, time::Duration};

use armillae_llm::{
    BridgeCapabilities, BridgeConfig, BridgeError, LlmBridge, OutputFormatCapabilities,
    ToolChoiceCapabilities,
};
use rig_core::{
    client::CompletionClient,
    http_client::{HttpClientExt, ReqwestClient},
    providers::openai,
};
use secrecy::{ExposeSecret, SecretString};

use crate::{RigBridge, request::OpenAiRequestMapper, response::OpenAiResponseNormalizer};

pub(crate) fn create(
    config: BridgeConfig,
    credential: Option<SecretString>,
) -> Result<Arc<dyn LlmBridge>, BridgeError> {
    let (config, credential, request_mapper) = validate_config(config, credential)?;
    let http_client = ReqwestClient::builder()
        .connect_timeout(Duration::from_millis(config.transport.connect_timeout_ms))
        .timeout(Duration::from_millis(config.transport.request_timeout_ms))
        .build()
        .map_err(|_| BridgeError::InvalidConfiguration {
            message: "failed to construct Rig HTTP client".to_owned(),
        })?;

    create_validated(config, credential, request_mapper, http_client)
}

fn validate_config(
    config: BridgeConfig,
    credential: Option<SecretString>,
) -> Result<(BridgeConfig, SecretString, OpenAiRequestMapper), BridgeError> {
    match config.provider.as_str() {
        "openai" | "openai-compatible" => {}
        provider => {
            return invalid_configuration(format!(
                "OpenAI provider module does not support provider: {provider}"
            ));
        }
    }

    if config.provider == "openai-compatible" && config.endpoint.is_none() {
        return invalid_configuration("openai-compatible requires an explicit custom endpoint");
    }

    let credential = credential.ok_or_else(|| BridgeError::InvalidConfiguration {
        message: format!("{} requires a credential", config.provider),
    })?;
    let request_mapper = match config.provider.as_str() {
        "openai" => OpenAiRequestMapper::new(config.provider_options.clone())?,
        "openai-compatible" => {
            OpenAiRequestMapper::for_openai_compatible(config.provider_options.clone())?
        }
        provider => {
            return invalid_configuration(format!(
                "OpenAI provider module does not support provider: {provider}"
            ));
        }
    };

    Ok((config, credential, request_mapper))
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
    let mut client_builder = openai::CompletionsClient::builder()
        .api_key(credential.expose_secret().to_owned())
        .http_client(http_client);
    if let Some(endpoint) = &config.endpoint {
        client_builder = client_builder.base_url(endpoint.as_str());
    }
    let client = client_builder
        .build()
        .map_err(|_| BridgeError::InvalidConfiguration {
            message: "failed to construct Rig OpenAI client".to_owned(),
        })?;
    let model_name = config.model.clone();
    let model = client.completion_model(config.model);
    let provider = config.provider;
    let bridge = RigBridge::new(
        model,
        model_name,
        capabilities(),
        config.defaults,
        Arc::new(request_mapper),
        Arc::new(OpenAiResponseNormalizer::new(provider)),
    )?;

    Ok(Arc::new(bridge))
}

const fn capabilities() -> BridgeCapabilities {
    BridgeCapabilities {
        streaming: true,
        tool_calling: true,
        parallel_tool_calls: true,
        tool_choice: ToolChoiceCapabilities::all(),
        output_format: OutputFormatCapabilities::all(),
        system_message: true,
        developer_message: false,
    }
}

fn invalid_configuration<T>(message: impl Into<String>) -> Result<T, BridgeError> {
    Err(BridgeError::InvalidConfiguration {
        message: message.into(),
    })
}

#[cfg(test)]
mod tests {
    use armillae_core::{
        AssistantContent, CompletionRequest, CompletionResponse, FinishReason, Message,
        TextContent, TokenUsage, ToolCallId, ToolChoice, ToolDefinition, ToolResult,
        ToolResultContent,
    };
    use armillae_llm::{
        BoxFuture, BridgeConfig, BridgeError, BridgeResolveContext, CredentialRef, SecretResolver,
        SecretString,
        mock::contract::{verify_completion, verify_stream},
    };
    use rig_core::test_utils::{MockHttpResponse, RecordingHttpClient};
    use serde_json::{Value, json};

    use super::{capabilities, create, create_validated, validate_config};
    use crate::providers::test_support::{
        capture_info_logs, expected_text_stream, streaming_client, text_stream_client,
    };

    fn tool_call_id(value: &str) -> ToolCallId {
        ToolCallId::new(value).expect("fixture ToolCall IDs are non-empty")
    }

    struct StaticSecretResolver;

    impl SecretResolver for StaticSecretResolver {
        fn resolve<'a>(
            &'a self,
            _key: &'a str,
        ) -> BoxFuture<'a, Result<SecretString, BridgeError>> {
            Box::pin(async { Ok(SecretString::from("factory-test-secret")) })
        }
    }

    fn resolved_config(
        provider: &str,
        endpoint: Option<&str>,
        with_credential: bool,
        provider_options: Value,
    ) -> (BridgeConfig, Option<SecretString>) {
        let mut builder =
            BridgeConfig::builder(provider, "test-model").provider_options(provider_options);
        if let Some(endpoint) = endpoint {
            builder = builder.endpoint(
                endpoint
                    .parse()
                    .expect("provider test endpoint must be a valid URL"),
            );
        }
        if with_credential {
            builder = builder.credential(CredentialRef::Resolver {
                key: "factory-test".to_owned(),
            });
        }
        let config = builder
            .build()
            .expect("provider test config must pass common validation");
        let context = BridgeResolveContext::new();
        let context = if with_credential {
            context.secret_resolver(&StaticSecretResolver as &dyn SecretResolver)
        } else {
            context
        };
        let resolved = futures::executor::block_on(config.resolve_with(context))
            .expect("provider test config must resolve");

        resolved.into_parts()
    }

    #[test]
    fn openai_profile_matches_the_existing_capability_contract() {
        let (config, credential) = resolved_config("openai", None, true, json!({}));
        let bridge =
            create(config, credential).expect("valid OpenAI configuration must construct a bridge");

        assert_eq!(bridge.capabilities(), capabilities());
    }

    #[test]
    fn openai_and_compatible_stream_across_arbitrary_utf8_byte_boundaries() {
        let configs = ["openai", "openai-compatible"].map(|provider| {
            (
                provider,
                resolved_config(provider, Some("http://stream.test/v1"), true, json!({})),
            )
        });
        futures::executor::block_on(async {
            for (provider, (config, credential)) in configs {
                let (config, credential, request_mapper) = validate_config(config, credential)
                    .unwrap_or_else(|error| {
                        panic!("{provider} stream config must validate: {error}")
                    });
                let bridge =
                    create_validated(config, credential, request_mapper, text_stream_client())
                        .unwrap_or_else(|error| {
                            panic!("{provider} stream bridge must construct: {error}")
                        });
                let request = CompletionRequest {
                    messages: vec![Message::user("hello")],
                    ..CompletionRequest::default()
                };

                verify_stream(bridge.as_ref(), request, &expected_text_stream())
                    .await
                    .unwrap_or_else(|error| panic!("{provider} stream contract failed: {error}"));
            }
        });
    }

    #[test]
    fn openai_stream_reassembles_interleaved_tool_calls() {
        let (config, credential) =
            resolved_config("openai", Some("http://stream.test/v1"), true, json!({}));
        futures::executor::block_on(async {
            let events = vec![
                json!({
                    "id": "tool-stream",
                    "model": "provider-model",
                    "choices": [{
                        "delta": { "tool_calls": [{
                            "index": 0,
                            "id": "call-weather",
                            "function": { "name": "weather", "arguments": "{\"city\":\"上" }
                        }] },
                        "finish_reason": null
                    }],
                    "usage": null
                }),
                json!({
                    "id": "tool-stream",
                    "model": "provider-model",
                    "choices": [{
                        "delta": { "tool_calls": [{
                            "index": 1,
                            "id": "call-dice",
                            "function": { "name": "dice", "arguments": "{\"sides\":" }
                        }] },
                        "finish_reason": null
                    }],
                    "usage": null
                }),
                json!({
                    "id": "tool-stream",
                    "model": "provider-model",
                    "choices": [{
                        "delta": { "tool_calls": [{
                            "index": 0,
                            "function": { "arguments": "海\"}" }
                        }] },
                        "finish_reason": null
                    }],
                    "usage": null
                }),
                json!({
                    "id": "tool-stream",
                    "model": "provider-model",
                    "choices": [{
                        "delta": { "tool_calls": [{
                            "index": 1,
                            "function": { "arguments": "20}" }
                        }] },
                        "finish_reason": null
                    }],
                    "usage": null
                }),
                json!({
                    "id": "tool-stream",
                    "model": "provider-model",
                    "choices": [{
                        "delta": {},
                        "finish_reason": "tool_calls"
                    }],
                    "usage": null
                }),
                json!({
                    "id": "tool-stream",
                    "model": "provider-model",
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 7,
                        "completion_tokens": 4,
                        "total_tokens": 11
                    }
                }),
            ];
            let (config, credential, request_mapper) = validate_config(config, credential)
                .expect("OpenAI tool stream config must validate");
            let bridge =
                create_validated(config, credential, request_mapper, streaming_client(events))
                    .expect("OpenAI tool stream bridge must construct");
            let request = CompletionRequest {
                messages: vec![Message::user("use tools")],
                ..CompletionRequest::default()
            };
            let expected = CompletionResponse {
                id: None,
                model: None,
                content: vec![
                    AssistantContent::ToolCall(armillae_core::ToolCall {
                        id: ToolCallId::new("call-weather")
                            .expect("fixture ToolCall ID must be non-empty"),
                        name: "weather".to_owned(),
                        arguments: json!({ "city": "上海" }),
                    }),
                    AssistantContent::ToolCall(armillae_core::ToolCall {
                        id: ToolCallId::new("call-dice")
                            .expect("fixture ToolCall ID must be non-empty"),
                        name: "dice".to_owned(),
                        arguments: json!({ "sides": 20 }),
                    }),
                ],
                finish_reason: None,
                usage: Some(TokenUsage {
                    input_tokens: Some(7),
                    output_tokens: Some(4),
                    total_tokens: Some(11),
                    cached_input_tokens: Some(0),
                }),
                provider_metadata: json!({}),
            };

            verify_stream(bridge.as_ref(), request, &expected)
                .await
                .expect("OpenAI interleaved ToolCalls must satisfy the shared stream contract");
        });
    }

    #[test]
    fn openai_configuration_requirements_are_preserved() {
        let (config, credential) = resolved_config("openai-compatible", None, true, json!({}));
        let missing_endpoint = create(config, credential);
        let (config, credential) = resolved_config(
            "openai-compatible",
            Some("http://localhost:8080/v1"),
            false,
            json!({}),
        );
        let missing_credential = create(config, credential);
        let (config, credential) =
            resolved_config("openai", None, true, json!({ "temperature": 0.5 }));
        let invalid_options = create(config, credential);

        assert!(matches!(
            missing_endpoint,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
        assert!(matches!(
            missing_credential,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
        assert!(matches!(
            invalid_options,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
    }

    #[test]
    fn compatible_endpoint_uses_openai_wire_contract_and_shared_completion_contract() {
        let (config, credential) = resolved_config(
            "openai-compatible",
            Some("http://compatible.test/v1"),
            true,
            json!({}),
        );
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(
                json!({
                    "id": "chatcmpl-factory",
                    "object": "chat.completion",
                    "created": 1,
                    "model": "compatible-model",
                    "system_fingerprint": "fp-compatible",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "hello"
                        },
                        "logprobs": null,
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 3,
                        "completion_tokens": 2,
                        "total_tokens": 5
                    }
                })
                .to_string(),
            );
            let (config, credential, request_mapper) =
                validate_config(config, credential).expect("compatible test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client.clone())
                .expect("compatible test bridge must construct");
            let request = CompletionRequest {
                messages: vec![Message::user("hello")],
                ..CompletionRequest::default()
            };
            let expected = CompletionResponse {
                id: Some("chatcmpl-factory".to_owned()),
                model: Some("compatible-model".to_owned()),
                content: vec![AssistantContent::Text(TextContent::new("hello"))],
                finish_reason: Some(FinishReason::Stop),
                usage: Some(TokenUsage {
                    input_tokens: Some(3),
                    output_tokens: Some(2),
                    total_tokens: Some(5),
                    cached_input_tokens: Some(0),
                }),
                provider_metadata: json!({ "system_fingerprint": "fp-compatible" }),
            };

            verify_completion(bridge.as_ref(), request, &expected)
                .await
                .expect("OpenAI-compatible bridge must satisfy the shared completion contract");

            let requests = http_client.requests();
            let captured = requests
                .first()
                .expect("the compatible bridge must issue one request");
            assert_eq!(captured.uri, "http://compatible.test/v1/chat/completions");
            assert_eq!(
                captured
                    .headers
                    .get("authorization")
                    .expect("Rig OpenAI client must attach bearer authorization")
                    .to_str()
                    .expect("test authorization header must be text"),
                "Bearer factory-test-secret"
            );
            let body: Value = serde_json::from_slice(&captured.body)
                .expect("captured OpenAI-compatible request must be JSON");
            assert_eq!(body["model"], "test-model");
            assert_eq!(body["messages"][0]["role"], "user");
            assert_eq!(body["messages"][0]["content"], "hello");
        });
    }

    #[test]
    fn compatible_remote_capability_mismatch_is_provider_rejected() {
        let (config, credential) = resolved_config(
            "openai-compatible",
            Some("http://compatible.test/v1"),
            true,
            json!({}),
        );
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::with_error_response(
                "400".parse().expect("400 must be a valid HTTP status code"),
                json!({ "error": { "message": "unsupported field" } }).to_string(),
            );
            let (config, credential, request_mapper) =
                validate_config(config, credential).expect("compatible test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client)
                .expect("compatible test bridge must construct");

            let error = bridge
                .complete(CompletionRequest {
                    messages: vec![Message::user("hello")],
                    ..CompletionRequest::default()
                })
                .await
                .expect_err("remote rejection must not be silently downgraded");

            assert!(matches!(
                error,
                BridgeError::ProviderRejected { metadata, .. }
                    if metadata.provider == "openai-compatible"
                        && metadata.http_status == Some(400)
            ));
        });
    }

    #[test]
    fn default_info_tracing_excludes_credentials_headers_and_content() {
        let (config, credential) = resolved_config(
            "openai-compatible",
            Some("http://compatible.test/v1"),
            true,
            json!({}),
        );
        let logs = capture_info_logs(|| {
            futures::executor::block_on(async {
                let http_client = RecordingHttpClient::new(
                    json!({
                        "id": "chatcmpl-safe-logs",
                        "object": "chat.completion",
                        "created": 1,
                        "model": "compatible-model",
                        "choices": [{
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": "response-content-secret-marker"
                            },
                            "logprobs": null,
                            "finish_reason": "stop"
                        }],
                        "usage": {
                            "prompt_tokens": 3,
                            "completion_tokens": 2,
                            "total_tokens": 5
                        }
                    })
                    .to_string(),
                );
                let (config, credential, request_mapper) = validate_config(config, credential)
                    .expect("compatible tracing config must validate");
                let bridge =
                    create_validated(config, credential, request_mapper, http_client.clone())
                        .expect("compatible tracing bridge must construct");

                bridge
                    .complete(CompletionRequest {
                        messages: vec![Message::user("request-content-secret-marker")],
                        ..CompletionRequest::default()
                    })
                    .await
                    .expect("compatible tracing request must complete");

                let captured = http_client
                    .requests()
                    .into_iter()
                    .next()
                    .expect("compatible tracing request must reach transport");
                assert_eq!(
                    captured
                        .headers
                        .get("authorization")
                        .expect("test request must contain authorization")
                        .to_str()
                        .expect("test authorization must be text"),
                    "Bearer factory-test-secret"
                );
            });
        });

        assert!(logs.contains("LLM Bridge call completed"));
        for secret in [
            "factory-test-secret",
            "authorization",
            "request-content-secret-marker",
            "response-content-secret-marker",
        ] {
            assert!(!logs.to_ascii_lowercase().contains(secret));
        }
    }

    #[test]
    fn explicit_tool_results_can_be_sent_in_a_follow_up_model_call() {
        let (config, credential) = resolved_config(
            "openai-compatible",
            Some("http://compatible.test/v1"),
            true,
            json!({}),
        );
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(
                json!({
                    "id": "chatcmpl-tools",
                    "object": "chat.completion",
                    "created": 1,
                    "model": "compatible-model",
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
                                    "id": "call-clock",
                                    "type": "function",
                                    "function": {
                                        "name": "get_clock",
                                        "arguments": "{\"zone\":\"Asia/Shanghai\"}"
                                    }
                                }
                            ]
                        },
                        "logprobs": null,
                        "finish_reason": "tool_calls"
                    }],
                    "usage": {
                        "prompt_tokens": 8,
                        "completion_tokens": 4,
                        "total_tokens": 12
                    }
                })
                .to_string(),
            );
            let (config, credential, request_mapper) =
                validate_config(config, credential).expect("tool flow config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client.clone())
                .expect("tool flow bridge must construct");
            let tools = vec![
                ToolDefinition {
                    name: "get_weather".to_owned(),
                    description: "Get weather".to_owned(),
                    input_schema: json!({
                        "type": "object",
                        "properties": { "city": { "type": "string" } },
                        "required": ["city"]
                    }),
                },
                ToolDefinition {
                    name: "get_clock".to_owned(),
                    description: "Get local time".to_owned(),
                    input_schema: json!({
                        "type": "object",
                        "properties": { "zone": { "type": "string" } },
                        "required": ["zone"]
                    }),
                },
            ];
            let user_message = Message::user("Check both tools");
            let first = bridge
                .complete(CompletionRequest {
                    messages: vec![user_message.clone()],
                    tools: tools.clone(),
                    tool_choice: Some(ToolChoice::Auto),
                    ..CompletionRequest::default()
                })
                .await
                .expect("first model call must return tool calls");

            assert_eq!(first.finish_reason, Some(FinishReason::ToolCall));
            assert_eq!(
                first
                    .tool_calls()
                    .map(|call| call.id.as_str())
                    .collect::<Vec<_>>(),
                ["call-weather", "call-clock"]
            );

            http_client.set_response(MockHttpResponse::success(
                json!({
                    "id": "chatcmpl-final",
                    "object": "chat.completion",
                    "created": 2,
                    "model": "compatible-model",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": "It is rainy and 10:30."
                        },
                        "logprobs": null,
                        "finish_reason": "stop"
                    }],
                    "usage": {
                        "prompt_tokens": 18,
                        "completion_tokens": 6,
                        "total_tokens": 24
                    }
                })
                .to_string(),
            ));
            let second = bridge
                .complete(CompletionRequest {
                    messages: vec![
                        user_message,
                        first.as_assistant_message(),
                        Message::tool_result(ToolResult {
                            call_id: tool_call_id("call-weather"),
                            content: vec![ToolResultContent::Json {
                                value: json!({ "condition": "rain" }),
                            }],
                            is_error: false,
                        }),
                        Message::tool_result(ToolResult {
                            call_id: tool_call_id("call-clock"),
                            content: vec![ToolResultContent::Text {
                                text: "10:30".to_owned(),
                            }],
                            is_error: true,
                        }),
                    ],
                    tools,
                    ..CompletionRequest::default()
                })
                .await
                .expect("follow-up model call must accept explicit tool results");

            assert_eq!(second.finish_reason, Some(FinishReason::Stop));
            assert!(matches!(
                &second.content[0],
                AssistantContent::Text(text) if text.text == "It is rainy and 10:30."
            ));

            let requests = http_client.requests();
            let body: Value = serde_json::from_slice(
                &requests
                    .get(1)
                    .expect("tool flow must issue a follow-up request")
                    .body,
            )
            .expect("follow-up request must be JSON");
            assert_eq!(body["messages"][1]["tool_calls"][0]["id"], "call-weather");
            assert_eq!(body["messages"][1]["tool_calls"][1]["id"], "call-clock");
            assert_eq!(body["messages"][2]["tool_call_id"], "call-weather");
            assert_eq!(body["messages"][3]["tool_call_id"], "call-clock");
            assert_eq!(body["messages"][3]["content"], "10:30");
        });
    }
}
