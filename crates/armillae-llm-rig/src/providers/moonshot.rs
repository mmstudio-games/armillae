use std::sync::Arc;

use armillae_llm::{
    BridgeCapabilities, BridgeConfig, BridgeError, LlmBridge, OutputFormatCapabilities,
    ToolChoiceCapabilities,
};
use rig_core::{client::CompletionClient, http_client::HttpClientExt, providers::moonshot};
use secrecy::{ExposeSecret, SecretString};

use crate::{RigBridge, request::OpenAiRequestMapper, response::OpenAiResponseNormalizer};

use super::{build_http_client, validate_named_config};

pub(crate) fn create(
    config: BridgeConfig,
    credential: Option<SecretString>,
) -> Result<Arc<dyn LlmBridge>, BridgeError> {
    let (config, credential, request_mapper) =
        validate_named_config(config, credential, "moonshot", "Moonshot")?;
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
    let mut client_builder = moonshot::Client::builder()
        .api_key(credential.expose_secret().to_owned())
        .http_client(http_client);
    if let Some(endpoint) = &config.endpoint {
        client_builder = client_builder.base_url(endpoint.as_str());
    }
    let client = client_builder
        .build()
        .map_err(|_| BridgeError::InvalidConfiguration {
            message: "failed to construct Rig Moonshot client".to_owned(),
        })?;
    let model = client.completion_model(config.model);
    let bridge = RigBridge::new(
        model,
        capabilities(),
        config.defaults,
        Arc::new(request_mapper),
        Arc::new(OpenAiResponseNormalizer::new("moonshot")),
    )?;

    Ok(Arc::new(bridge))
}

const fn capabilities() -> BridgeCapabilities {
    BridgeCapabilities {
        streaming: false,
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

#[cfg(test)]
mod tests {
    use armillae_core::{
        AssistantContent, CompletionRequest, CompletionResponse, FinishReason, Message,
        OutputFormat, TextContent, TokenUsage, ToolCall, ToolCallId, ToolChoice, ToolDefinition,
    };
    use armillae_llm::{BridgeError, mock::contract::verify_completion};
    use rig_core::test_utils::RecordingHttpClient;
    use serde_json::{Value, json};

    use super::{capabilities, create, create_validated};
    use crate::providers::{test_support::resolved_config, validate_named_config};

    #[test]
    fn configuration_and_capability_profile_are_explicit() {
        let (config, credential) = resolved_config("moonshot", None, true, json!({}));
        let bridge = create(config, credential)
            .expect("valid Moonshot configuration must construct a bridge");
        let (config, credential) = resolved_config("moonshot", None, false, json!({}));
        let missing_credential = create(config, credential);
        let (config, credential) =
            resolved_config("moonshot", None, true, json!({ "future": true }));
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
    fn openai_wire_preserves_json_object_and_shared_completion_contract() {
        let (config, credential) =
            resolved_config("moonshot", Some("http://moonshot.test/v1"), true, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(openai_text_response().to_string());
            let (config, credential, request_mapper) =
                validate_named_config(config, credential, "moonshot", "Moonshot")
                    .expect("Moonshot test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client.clone())
                .expect("Moonshot test bridge must construct");
            let request = CompletionRequest {
                messages: vec![Message::user("hello")],
                tools: vec![tool_definition()],
                tool_choice: Some(ToolChoice::Auto),
                output_format: Some(OutputFormat::JsonObject),
                ..CompletionRequest::default()
            };
            let expected = CompletionResponse {
                id: Some("chatcmpl-moonshot".to_owned()),
                model: Some("kimi-k2".to_owned()),
                content: vec![
                    AssistantContent::Text(TextContent::new("checking")),
                    AssistantContent::ToolCall(ToolCall {
                        id: ToolCallId::new("call-lookup")
                            .expect("fixture ToolCall ID must be non-empty"),
                        name: "lookup".to_owned(),
                        arguments: json!({ "query": "hello" }),
                    }),
                ],
                finish_reason: Some(FinishReason::ToolCall),
                usage: Some(TokenUsage {
                    input_tokens: Some(4),
                    output_tokens: Some(2),
                    total_tokens: Some(6),
                    cached_input_tokens: Some(0),
                }),
                provider_metadata: json!({}),
            };

            verify_completion(bridge.as_ref(), request, &expected)
                .await
                .expect("Moonshot bridge must satisfy the shared completion contract");

            let requests = http_client.requests();
            let captured = requests
                .first()
                .expect("Moonshot bridge must issue one request");
            assert_eq!(captured.uri, "http://moonshot.test/v1/chat/completions");
            let body: Value = serde_json::from_slice(&captured.body)
                .expect("captured Moonshot request must be JSON");
            assert_eq!(body["model"], "test-model");
            assert_eq!(body["response_format"]["type"], "json_object");
        });
    }

    #[test]
    fn unsupported_tool_choices_and_json_schema_fail_before_transport() {
        let (config, credential) =
            resolved_config("moonshot", Some("http://moonshot.test/v1"), true, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::new(openai_text_response().to_string());
            let (config, credential, request_mapper) =
                validate_named_config(config, credential, "moonshot", "Moonshot")
                    .expect("Moonshot test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client.clone())
                .expect("Moonshot test bridge must construct");

            let required = bridge
                .complete(CompletionRequest {
                    messages: vec![Message::user("hello")],
                    tools: vec![tool_definition()],
                    tool_choice: Some(ToolChoice::Required),
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
                required,
                Err(BridgeError::UnsupportedCapability { capability })
                    if capability == "tool_choice.required"
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
    fn provider_rejection_keeps_moonshot_identity() {
        let (config, credential) =
            resolved_config("moonshot", Some("http://moonshot.test/v1"), true, json!({}));
        futures::executor::block_on(async {
            let http_client = RecordingHttpClient::with_error_response(
                "400".parse().expect("400 must be a valid HTTP status code"),
                json!({ "error": { "message": "unsupported" } }).to_string(),
            );
            let (config, credential, request_mapper) =
                validate_named_config(config, credential, "moonshot", "Moonshot")
                    .expect("Moonshot test config must validate");
            let bridge = create_validated(config, credential, request_mapper, http_client)
                .expect("Moonshot test bridge must construct");

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
                    if metadata.provider == "moonshot" && metadata.http_status == Some(400)
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
            "id": "chatcmpl-moonshot",
            "object": "chat.completion",
            "created": 1,
            "model": "kimi-k2",
            "system_fingerprint": null,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "checking",
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
                "prompt_tokens": 4,
                "completion_tokens": 2,
                "total_tokens": 6
            }
        })
    }
}
