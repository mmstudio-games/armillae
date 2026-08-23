use std::{sync::Arc, time::Duration};

use armillae_llm::{
    BoxFuture, BridgeCapabilities, BridgeConfig, BridgeError, BridgeFactory, LlmBridge,
    OutputFormatCapabilities, ResolvedBridgeConfig, ToolChoiceCapabilities,
};
use rig_core::{
    client::CompletionClient,
    http_client::{HttpClientExt, ReqwestClient},
    providers::openai,
};
use secrecy::{ExposeSecret, SecretString};

use crate::{RigBridge, request::OpenAiRequestMapper, response::OpenAiResponseNormalizer};

/// Constructs non-streaming Rig bridges for the first-phase Provider set.
#[derive(Clone, Copy, Debug, Default)]
pub struct RigBridgeFactory;

impl BridgeFactory for RigBridgeFactory {
    fn driver(&self) -> &'static str {
        "rig"
    }

    fn create<'a>(
        &'a self,
        config: ResolvedBridgeConfig,
    ) -> BoxFuture<'a, Result<Arc<dyn LlmBridge>, BridgeError>> {
        Box::pin(async move { create_bridge(config) })
    }
}

fn create_bridge(config: ResolvedBridgeConfig) -> Result<Arc<dyn LlmBridge>, BridgeError> {
    let (config, credential) = validate_config(config)?;

    let http_client = ReqwestClient::builder()
        .connect_timeout(Duration::from_millis(config.transport.connect_timeout_ms))
        .timeout(Duration::from_millis(config.transport.request_timeout_ms))
        .build()
        .map_err(|_| BridgeError::InvalidConfiguration {
            message: "failed to construct Rig HTTP client".to_owned(),
        })?;

    create_openai_bridge(config, credential, http_client)
}

fn validate_config(
    config: ResolvedBridgeConfig,
) -> Result<(BridgeConfig, SecretString), BridgeError> {
    let (config, credential) = config.into_parts();
    if config.driver != "rig" {
        return invalid_configuration(format!(
            "RigBridgeFactory cannot construct driver: {}",
            config.driver
        ));
    }

    match config.provider.as_str() {
        "openai" | "openai-compatible" => {}
        provider => {
            return invalid_configuration(format!(
                "RigBridgeFactory does not support provider: {provider}"
            ));
        }
    }

    if config.provider == "openai-compatible" && config.endpoint.is_none() {
        return invalid_configuration("openai-compatible requires an explicit custom endpoint");
    }

    let credential = credential.ok_or_else(|| BridgeError::InvalidConfiguration {
        message: format!("{} requires a credential", config.provider),
    })?;
    OpenAiRequestMapper::new(config.provider_options.clone())?;

    Ok((config, credential))
}

fn create_openai_bridge<H>(
    config: BridgeConfig,
    credential: SecretString,
    http_client: H,
) -> Result<Arc<dyn LlmBridge>, BridgeError>
where
    H: HttpClientExt + Clone + Default + std::fmt::Debug + 'static,
{
    let request_mapper = OpenAiRequestMapper::new(config.provider_options)?;
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
    let model = client.completion_model(config.model);
    let provider = config.provider;
    let bridge = RigBridge::new(
        model,
        openai_capabilities(),
        config.defaults,
        Arc::new(request_mapper),
        Arc::new(OpenAiResponseNormalizer::new(provider)),
    )?;

    Ok(Arc::new(bridge))
}

const fn openai_capabilities() -> BridgeCapabilities {
    BridgeCapabilities {
        streaming: false,
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
        BoxFuture, BridgeConfig, BridgeError, BridgeFactory, CredentialRef, SecretResolver,
        SecretString, mock::contract::verify_completion,
    };
    use rig_core::test_utils::{MockHttpResponse, RecordingHttpClient};
    use serde_json::{Value, json};

    use super::{RigBridgeFactory, create_openai_bridge, openai_capabilities, validate_config};

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
        driver: &str,
        provider: &str,
        endpoint: Option<&str>,
        with_credential: bool,
        provider_options: Value,
    ) -> armillae_llm::ResolvedBridgeConfig {
        let mut builder = BridgeConfig::builder(driver, provider, "test-model")
            .provider_options(provider_options);
        if let Some(endpoint) = endpoint {
            builder = builder.endpoint(
                endpoint
                    .parse()
                    .expect("factory test endpoint must be a valid URL"),
            );
        }
        if with_credential {
            builder = builder.credential(CredentialRef::Resolver {
                key: "factory-test".to_owned(),
            });
        }
        let config = builder
            .build()
            .expect("factory test config must pass common validation");

        futures::executor::block_on(config.resolve(
            with_credential.then_some(&StaticSecretResolver as &dyn SecretResolver),
            None,
        ))
        .expect("factory test config must resolve")
    }

    #[test]
    fn factory_advertises_rig_driver_and_openai_profile() {
        let factory = RigBridgeFactory;
        assert_eq!(factory.driver(), "rig");

        let bridge = futures::executor::block_on(factory.create(resolved_config(
            "rig",
            "openai",
            None,
            true,
            json!({}),
        )))
        .expect("valid OpenAI configuration must construct a bridge");

        assert_eq!(bridge.capabilities(), openai_capabilities());
    }

    #[test]
    fn openai_compatible_requires_endpoint_and_credential() {
        let factory = RigBridgeFactory;
        let missing_endpoint = futures::executor::block_on(factory.create(resolved_config(
            "rig",
            "openai-compatible",
            None,
            true,
            json!({}),
        )));
        let missing_credential = futures::executor::block_on(factory.create(resolved_config(
            "rig",
            "openai-compatible",
            Some("http://localhost:8080/v1"),
            false,
            json!({}),
        )));

        assert!(matches!(
            missing_endpoint,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
        assert!(matches!(
            missing_credential,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
    }

    #[test]
    fn factory_rejects_wrong_driver_provider_and_options() {
        let factory = RigBridgeFactory;
        let wrong_driver = futures::executor::block_on(factory.create(resolved_config(
            "other",
            "openai",
            None,
            true,
            json!({}),
        )));
        let unsupported_provider = futures::executor::block_on(factory.create(resolved_config(
            "rig",
            "anthropic",
            None,
            true,
            json!({}),
        )));
        let invalid_options = futures::executor::block_on(factory.create(resolved_config(
            "rig",
            "openai",
            None,
            true,
            json!({ "temperature": 0.5 }),
        )));

        assert!(matches!(
            wrong_driver,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
        assert!(matches!(
            unsupported_provider,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
        assert!(matches!(
            invalid_options,
            Err(BridgeError::InvalidConfiguration { .. })
        ));
    }

    #[test]
    fn compatible_endpoint_uses_openai_wire_contract_and_shared_completion_contract() {
        let resolved = resolved_config(
            "rig",
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
            let (config, credential) =
                validate_config(resolved).expect("compatible test config must validate");
            let bridge = create_openai_bridge(config, credential, http_client.clone())
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
        let resolved = resolved_config(
            "rig",
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
            let (config, credential) =
                validate_config(resolved).expect("compatible test config must validate");
            let bridge = create_openai_bridge(config, credential, http_client)
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
    fn explicit_tool_results_can_be_sent_in_a_follow_up_model_call() {
        let resolved = resolved_config(
            "rig",
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
            let (config, credential) =
                validate_config(resolved).expect("tool flow config must validate");
            let bridge = create_openai_bridge(config, credential, http_client.clone())
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
