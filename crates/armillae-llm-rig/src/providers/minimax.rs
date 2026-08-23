use std::sync::Arc;

use armillae_llm::{
    BridgeCapabilities, BridgeConfig, BridgeError, LlmBridge, OutputFormatCapabilities,
    ToolChoiceCapabilities,
};
use rig_core::{client::CompletionClient, http_client::HttpClientExt, providers::minimax};
use secrecy::{ExposeSecret, SecretString};

use crate::{RigBridge, request::OpenAiRequestMapper, response::OpenAiResponseNormalizer};

use super::{build_http_client, validate_named_config};

pub(crate) fn create(
    config: BridgeConfig,
    credential: Option<SecretString>,
) -> Result<Arc<dyn LlmBridge>, BridgeError> {
    let (config, credential, request_mapper) =
        validate_named_config(config, credential, "minimax", "MiniMax")?;
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
    let mut client_builder = minimax::Client::builder()
        .api_key(credential.expose_secret().to_owned())
        .http_client(http_client);
    if let Some(endpoint) = &config.endpoint {
        client_builder = client_builder.base_url(endpoint.as_str());
    }
    let client = client_builder
        .build()
        .map_err(|_| BridgeError::InvalidConfiguration {
            message: "failed to construct Rig MiniMax client".to_owned(),
        })?;
    let model = client.completion_model(config.model);
    let bridge = RigBridge::new(
        model,
        capabilities(),
        config.defaults,
        Arc::new(request_mapper),
        Arc::new(OpenAiResponseNormalizer::new("minimax")),
    )?;

    Ok(Arc::new(bridge))
}

const fn capabilities() -> BridgeCapabilities {
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

#[cfg(test)]
mod tests {
    use armillae_core::{
        AssistantContent, CompletionRequest, CompletionResponse, FinishReason, Message,
        OutputFormat, TokenUsage, ToolCall, ToolCallId, ToolChoice, ToolDefinition,
    };
    use armillae_llm::{BridgeError, mock::contract::verify_completion};
    use rig_core::test_utils::RecordingHttpClient;
    use serde_json::{Value, json};

    use super::{
        super::{test_support::resolved_config, validate_named_config},
        capabilities, create, create_validated,
    };

    #[test]
    fn configuration_and_capability_profile_are_explicit() {
        let (config, credential) = resolved_config("minimax", None, true, json!({}));
        let bridge = create(config, credential)
            .expect("valid MiniMax configuration must construct a bridge");
        let (config, credential) = resolved_config("minimax", None, false, json!({}));
        let missing_credential = create(config, credential);
        let (config, credential) =
            resolved_config("minimax", None, true, json!({ "future": true }));
        let unknown_options = create(config, credential);

        assert_eq!(bridge.capabilities(), capabilities());
        assert!(!bridge.capabilities().streaming);
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
    fn openai_wire_supports_specific_tool_choice_and_json_schema() {
        let (config, credential) =
            resolved_config("minimax", Some("http://minimax.test/v1"), true, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(openai_text_response().to_string());
            let (config, credential, request_mapper) =
                validate_named_config(config, credential, "minimax", "MiniMax")
                    .expect("MiniMax test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client.clone())
                .expect("MiniMax test bridge must construct");
            let request = CompletionRequest {
                messages: vec![Message::user("hello")],
                tools: vec![tool_definition()],
                tool_choice: Some(ToolChoice::Specific {
                    name: "lookup".to_owned(),
                }),
                output_format: Some(OutputFormat::JsonSchema {
                    name: "answer".to_owned(),
                    schema: json!({ "type": "object" }),
                    strict: true,
                }),
                ..CompletionRequest::default()
            };
            let expected = CompletionResponse {
                id: Some("chatcmpl-minimax".to_owned()),
                model: Some("MiniMax-M2".to_owned()),
                content: vec![AssistantContent::ToolCall(ToolCall {
                    id: ToolCallId::new("call-lookup")
                        .expect("fixture ToolCall ID must be non-empty"),
                    name: "lookup".to_owned(),
                    arguments: json!({ "query": "hello" }),
                })],
                finish_reason: Some(FinishReason::ToolCall),
                usage: Some(TokenUsage {
                    input_tokens: Some(3),
                    output_tokens: Some(2),
                    total_tokens: Some(5),
                    cached_input_tokens: Some(0),
                }),
                provider_metadata: json!({}),
            };

            verify_completion(bridge.as_ref(), request, &expected)
                .await
                .expect("MiniMax bridge must satisfy the shared completion contract");

            let requests = http_client.requests();
            let captured = requests
                .first()
                .expect("MiniMax bridge must issue one request");
            assert_eq!(captured.uri, "http://minimax.test/v1/chat/completions");
            assert_eq!(
                captured
                    .headers
                    .get("authorization")
                    .expect("MiniMax client must attach bearer authorization")
                    .to_str()
                    .expect("test authorization header must be text"),
                "Bearer named-provider-test-secret"
            );
            let body: Value = serde_json::from_slice(&captured.body)
                .expect("captured MiniMax request must be JSON");
            assert_eq!(body["model"], "test-model");
            assert_eq!(body["tool_choice"]["function"]["name"], "lookup");
            assert_eq!(body["response_format"]["type"], "json_schema");
            assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        });
    }

    #[test]
    fn provider_rejection_keeps_minimax_identity() {
        let (config, credential) =
            resolved_config("minimax", Some("http://minimax.test/v1"), true, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::with_error_response(
                "400".parse().expect("400 must be a valid HTTP status code"),
                json!({ "error": { "message": "unsupported" } }).to_string(),
            );
            let (config, credential, request_mapper) =
                validate_named_config(config, credential, "minimax", "MiniMax")
                    .expect("MiniMax test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client)
                .expect("MiniMax test bridge must construct");

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
                    if metadata.provider == "minimax" && metadata.http_status == Some(400)
            ));
        });
    }

    fn tool_definition() -> ToolDefinition {
        ToolDefinition {
            name: "lookup".to_owned(),
            description: "Look up an answer".to_owned(),
            input_schema: json!({ "type": "object" }),
        }
    }

    fn openai_text_response() -> Value {
        json!({
            "id": "chatcmpl-minimax",
            "object": "chat.completion",
            "created": 1,
            "model": "MiniMax-M2",
            "system_fingerprint": null,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-lookup",
                        "type": "function",
                        "function": {
                            "name": "lookup",
                            "arguments": "{\"query\":\"hello\"}"
                        }
                    }]
                },
                "logprobs": null,
                "finish_reason": "tool_calls"
            }],
            "usage": {
                "prompt_tokens": 3,
                "completion_tokens": 2,
                "total_tokens": 5
            }
        })
    }
}
