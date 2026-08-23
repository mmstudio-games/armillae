use armillae_core::{
    AssistantContent as ArmillaeAssistantContent, CompletionRequest as ArmillaeCompletionRequest,
    ContentPart, GenerationOptions, Message as ArmillaeMessage, OutputFormat, ProviderData,
    ProviderExtensions, Role, TextContent, TokenUsage, ToolCallId,
    ToolChoice as ArmillaeToolChoice, ToolDefinition as ArmillaeToolDefinition,
    ToolResult as ArmillaeToolResult, ToolResultContent as ArmillaeToolResultContent,
};
use armillae_llm::BridgeError;
use rig_core::{
    OneOrMany,
    completion::{ToolDefinition as RigToolDefinition, Usage as RigUsage},
    message::{
        AssistantContent as RigAssistantContent, Message as RigMessage, Text as RigText,
        ToolCall as RigToolCall, ToolChoice as RigToolChoice, ToolFunction,
        ToolResult as RigToolResult, ToolResultContent as RigToolResultContent,
        UserContent as RigUserContent,
    },
};
use serde_json::{Map, Value};

#[derive(Debug)]
pub(crate) struct RequestParts {
    pub(crate) chat_history: OneOrMany<RigMessage>,
    pub(crate) tools: Vec<RigToolDefinition>,
    pub(crate) tool_choice: Option<RigToolChoice>,
    pub(crate) output_format: Option<OutputFormat>,
    pub(crate) generation: GenerationOptions,
    pub(crate) extensions: ProviderExtensions,
}

pub(crate) fn request_parts(
    request: ArmillaeCompletionRequest,
    defaults: &GenerationOptions,
) -> Result<RequestParts, BridgeError> {
    let ArmillaeCompletionRequest {
        messages,
        tools,
        tool_choice,
        output_format,
        generation,
        extensions,
    } = request;

    Ok(RequestParts {
        chat_history: messages_to_rig(messages)?,
        tools: tools.into_iter().map(tool_definition_to_rig).collect(),
        tool_choice: tool_choice.map(tool_choice_to_rig).transpose()?,
        output_format,
        generation: merge_generation_options(defaults, generation),
        extensions,
    })
}

pub(crate) fn merge_generation_options(
    defaults: &GenerationOptions,
    request: GenerationOptions,
) -> GenerationOptions {
    GenerationOptions {
        temperature: request.temperature.or(defaults.temperature),
        max_output_tokens: request.max_output_tokens.or(defaults.max_output_tokens),
        stop: if request.stop.is_empty() {
            defaults.stop.clone()
        } else {
            request.stop
        },
        seed: request.seed.or(defaults.seed),
    }
}

pub(crate) fn messages_to_rig(
    messages: Vec<ArmillaeMessage>,
) -> Result<OneOrMany<RigMessage>, BridgeError> {
    let converted = messages
        .into_iter()
        .map(message_to_rig)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect();

    one_or_many(
        converted,
        "completion request must contain at least one message",
    )
}

fn message_to_rig(message: ArmillaeMessage) -> Result<Vec<RigMessage>, BridgeError> {
    if message.content.is_empty() {
        return invalid_request("messages must contain at least one content part");
    }

    match message.role {
        Role::System => message
            .content
            .into_iter()
            .map(|content| match content {
                ContentPart::Text(text) => Ok(RigMessage::System { content: text.text }),
                _ => invalid_request("system messages may contain only text"),
            })
            .collect(),
        Role::Developer => Err(BridgeError::UnsupportedCapability {
            capability: "role.developer".to_owned(),
        }),
        Role::User => {
            let content = message
                .content
                .into_iter()
                .map(user_content_to_rig)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(vec![RigMessage::User {
                content: one_or_many(content, "user messages must contain compatible content")?,
            }])
        }
        Role::Assistant => {
            let content = message
                .content
                .into_iter()
                .map(assistant_content_to_rig)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(vec![RigMessage::Assistant {
                id: None,
                content: one_or_many(
                    content,
                    "assistant messages must contain compatible content",
                )?,
            }])
        }
        Role::Tool => {
            let content = message
                .content
                .into_iter()
                .map(|content| match content {
                    ContentPart::ToolResult(result) => {
                        Ok(RigUserContent::ToolResult(tool_result_to_rig(result)?))
                    }
                    _ => invalid_request("tool messages may contain only ToolResult content"),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(vec![RigMessage::User {
                content: one_or_many(content, "tool messages must contain ToolResult content")?,
            }])
        }
        _ => Err(BridgeError::UnsupportedCapability {
            capability: "role.unknown".to_owned(),
        }),
    }
}

fn user_content_to_rig(content: ContentPart) -> Result<RigUserContent, BridgeError> {
    match content {
        ContentPart::Text(text) => Ok(RigUserContent::Text(RigText::new(text.text))),
        ContentPart::ToolResult(result) => {
            Ok(RigUserContent::ToolResult(tool_result_to_rig(result)?))
        }
        ContentPart::ToolCall(_) => {
            invalid_request("user messages cannot contain ToolCall content")
        }
        ContentPart::ProviderData(data) => unsupported_request_provider_data(data),
        _ => Err(BridgeError::UnsupportedCapability {
            capability: "content_part.unknown".to_owned(),
        }),
    }
}

fn assistant_content_to_rig(content: ContentPart) -> Result<RigAssistantContent, BridgeError> {
    match content {
        ContentPart::Text(text) => Ok(RigAssistantContent::Text(RigText::new(text.text))),
        ContentPart::ToolCall(call) => Ok(RigAssistantContent::ToolCall(RigToolCall {
            id: call.id.into_inner(),
            call_id: None,
            function: ToolFunction::new(call.name, call.arguments),
            signature: None,
            additional_params: None,
        })),
        ContentPart::ToolResult(_) => {
            invalid_request("assistant messages cannot contain ToolResult content")
        }
        ContentPart::ProviderData(data) => unsupported_request_provider_data(data),
        _ => Err(BridgeError::UnsupportedCapability {
            capability: "content_part.unknown".to_owned(),
        }),
    }
}

fn tool_result_to_rig(result: ArmillaeToolResult) -> Result<RigToolResult, BridgeError> {
    let content = result
        .content
        .into_iter()
        .map(|content| match content {
            ArmillaeToolResultContent::Text { text } => {
                Ok(RigToolResultContent::Text(RigText::new(text)))
            }
            ArmillaeToolResultContent::Json { value } => Ok(RigToolResultContent::Json { value }),
            _ => Err(BridgeError::UnsupportedCapability {
                capability: "tool_result_content.unknown".to_owned(),
            }),
        })
        .collect::<Result<Vec<_>, _>>()?;

    Ok(RigToolResult {
        id: result.call_id.into_inner(),
        call_id: None,
        content: one_or_many(content, "ToolResult must contain at least one content item")?,
    })
}

fn tool_definition_to_rig(tool: ArmillaeToolDefinition) -> RigToolDefinition {
    RigToolDefinition {
        name: tool.name,
        description: tool.description,
        parameters: tool.input_schema,
    }
}

fn tool_choice_to_rig(choice: ArmillaeToolChoice) -> Result<RigToolChoice, BridgeError> {
    Ok(match choice {
        ArmillaeToolChoice::Auto => RigToolChoice::Auto,
        ArmillaeToolChoice::None => RigToolChoice::None,
        ArmillaeToolChoice::Required => RigToolChoice::Required,
        ArmillaeToolChoice::Specific { name } => RigToolChoice::Specific {
            function_names: vec![name],
        },
        _ => {
            return Err(BridgeError::UnsupportedCapability {
                capability: "tool_choice.unknown".to_owned(),
            });
        }
    })
}

pub(crate) fn assistant_content_from_rig(
    choice: OneOrMany<RigAssistantContent>,
    provider: &str,
) -> Result<Vec<ArmillaeAssistantContent>, BridgeError> {
    let mut converted = Vec::new();
    for content in choice {
        match content {
            RigAssistantContent::Text(text) => {
                let RigText {
                    text,
                    additional_params,
                } = text;
                converted.push(ArmillaeAssistantContent::Text(TextContent::new(text)));
                if let Some(value) = additional_params {
                    converted.push(provider_data(provider, "text_metadata", value));
                }
            }
            RigAssistantContent::ToolCall(call) => {
                let RigToolCall {
                    id,
                    call_id,
                    function,
                    signature,
                    additional_params,
                } = call;
                converted.push(ArmillaeAssistantContent::ToolCall(
                    armillae_core::ToolCall {
                        id: tool_call_id_from_rig(id, provider)?,
                        name: function.name,
                        arguments: function.arguments,
                    },
                ));

                let mut metadata = Map::new();
                if let Some(call_id) = call_id {
                    metadata.insert("call_id".to_owned(), Value::String(call_id));
                }
                if let Some(signature) = signature {
                    metadata.insert("signature".to_owned(), Value::String(signature));
                }
                if let Some(additional_params) = additional_params {
                    metadata.insert("additional_params".to_owned(), additional_params);
                }
                if !metadata.is_empty() {
                    converted.push(provider_data(
                        provider,
                        "tool_call_metadata",
                        Value::Object(metadata),
                    ));
                }
            }
            RigAssistantContent::Reasoning(reasoning) => {
                converted.push(provider_data(
                    provider,
                    "reasoning",
                    preserve_serialized(provider, "reasoning", serde_json::to_value(reasoning))?,
                ));
            }
            RigAssistantContent::Image(image) => {
                converted.push(provider_data(
                    provider,
                    "image",
                    preserve_serialized(provider, "image", serde_json::to_value(image))?,
                ));
            }
        }
    }
    Ok(converted)
}

pub(crate) fn usage_from_rig(usage: RigUsage) -> Option<TokenUsage> {
    usage.has_values().then_some(TokenUsage {
        input_tokens: Some(usage.input_tokens),
        output_tokens: Some(usage.output_tokens),
        total_tokens: Some(usage.total_tokens),
        cached_input_tokens: Some(usage.cached_input_tokens),
    })
}

fn provider_data(provider: &str, kind: &str, value: Value) -> ArmillaeAssistantContent {
    ArmillaeAssistantContent::ProviderData(ProviderData {
        provider: provider.to_owned(),
        kind: kind.to_owned(),
        value,
    })
}

fn preserve_serialized(
    provider: &str,
    kind: &str,
    value: Result<Value, serde_json::Error>,
) -> Result<Value, BridgeError> {
    value.map_err(|error| BridgeError::InvalidProviderResponse {
        message: format!("failed to preserve {provider} {kind} content: {error}"),
        metadata: armillae_llm::ErrorMetadata::new(provider),
    })
}

fn unsupported_request_provider_data<T>(data: ProviderData) -> Result<T, BridgeError> {
    Err(BridgeError::UnsupportedCapability {
        capability: format!("request_provider_data.{}.{}", data.provider, data.kind),
    })
}

fn tool_call_id_from_rig(id: String, provider: &str) -> Result<ToolCallId, BridgeError> {
    ToolCallId::new(id).map_err(|_| BridgeError::InvalidProviderResponse {
        message: "Provider returned an empty ToolCall ID".to_owned(),
        metadata: armillae_llm::ErrorMetadata::new(provider),
    })
}

fn one_or_many<T: Clone>(items: Vec<T>, message: &str) -> Result<OneOrMany<T>, BridgeError> {
    match items.len() {
        0 => invalid_request(message),
        1 => items.into_iter().next().map(OneOrMany::one).ok_or_else(|| {
            BridgeError::InvalidRequest {
                message: message.to_owned(),
            }
        }),
        _ => OneOrMany::many(items).map_err(|_| BridgeError::InvalidRequest {
            message: message.to_owned(),
        }),
    }
}

fn invalid_request<T>(message: impl Into<String>) -> Result<T, BridgeError> {
    Err(BridgeError::InvalidRequest {
        message: message.into(),
    })
}

#[cfg(test)]
mod tests {
    use armillae_core::{
        AssistantContent as ArmillaeAssistantContent, CompletionRequest, ContentPart,
        GenerationOptions, Message, ProviderData, ProviderExtensions, Role, ToolCall, ToolCallId,
        ToolResult, ToolResultContent,
    };
    use armillae_llm::BridgeError;
    use rig_core::message::{
        AssistantContent as RigAssistantContent, Message as RigMessage, ToolCall as RigToolCall,
        ToolFunction, ToolResultContent as RigToolResultContent, UserContent as RigUserContent,
    };
    use serde_json::json;

    use super::{assistant_content_from_rig, merge_generation_options, request_parts};

    fn tool_call_id(value: &str) -> ToolCallId {
        ToolCallId::new(value).expect("fixture ToolCall IDs are non-empty")
    }

    #[test]
    fn request_generation_values_override_defaults() {
        let merged = merge_generation_options(
            &GenerationOptions {
                temperature: Some(0.2),
                max_output_tokens: Some(100),
                stop: vec!["default".to_owned()],
                seed: Some(1),
            },
            GenerationOptions {
                temperature: Some(0.8),
                max_output_tokens: None,
                stop: vec!["request".to_owned()],
                seed: None,
            },
        );

        assert_eq!(merged.temperature, Some(0.8));
        assert_eq!(merged.max_output_tokens, Some(100));
        assert_eq!(merged.stop, ["request"]);
        assert_eq!(merged.seed, Some(1));
    }

    #[test]
    fn empty_request_stop_uses_configured_default() {
        let merged = merge_generation_options(
            &GenerationOptions {
                stop: vec!["default".to_owned()],
                ..GenerationOptions::default()
            },
            GenerationOptions::default(),
        );

        assert_eq!(merged.stop, ["default"]);
    }

    #[test]
    fn assistant_text_and_multiple_tool_calls_keep_order_and_ids() {
        let request = CompletionRequest {
            messages: vec![Message::assistant(vec![
                ContentPart::text("checking"),
                ContentPart::ToolCall(ToolCall {
                    id: tool_call_id("call-1"),
                    name: "weather".to_owned(),
                    arguments: json!({ "city": "上海" }),
                }),
                ContentPart::ToolCall(ToolCall {
                    id: tool_call_id("call-2"),
                    name: "clock".to_owned(),
                    arguments: json!({ "zone": "Asia/Shanghai" }),
                }),
            ])],
            extensions: ProviderExtensions::default(),
            ..CompletionRequest::default()
        };

        let parts = request_parts(request, &GenerationOptions::default())
            .expect("portable assistant history must convert");
        let message = parts
            .chat_history
            .into_iter()
            .next()
            .expect("one assistant message must remain");
        let RigMessage::Assistant { content, .. } = message else {
            panic!("expected assistant message");
        };
        let content = content.into_iter().collect::<Vec<_>>();

        assert!(matches!(&content[0], RigAssistantContent::Text(text) if text.text == "checking"));
        assert!(matches!(&content[1], RigAssistantContent::ToolCall(call) if call.id == "call-1"));
        assert!(matches!(&content[2], RigAssistantContent::ToolCall(call) if call.id == "call-2"));
    }

    #[test]
    fn error_tool_result_preserves_content_without_wire_error_flag() {
        let request = CompletionRequest {
            messages: vec![Message::new(
                Role::Tool,
                vec![ContentPart::ToolResult(ToolResult {
                    call_id: tool_call_id("call-1"),
                    content: vec![
                        ToolResultContent::Text {
                            text: "lookup failed".to_owned(),
                        },
                        ToolResultContent::Json {
                            value: json!({ "code": "offline" }),
                        },
                    ],
                    is_error: true,
                })],
            )],
            ..CompletionRequest::default()
        };

        let parts = request_parts(request, &GenerationOptions::default())
            .expect("OpenAI-compatible ToolResult errors must not be rejected");
        let message = parts
            .chat_history
            .into_iter()
            .next()
            .expect("one tool message must remain");
        let RigMessage::User { content } = message else {
            panic!("Rig represents tool results as user content");
        };
        let RigUserContent::ToolResult(result) = content
            .into_iter()
            .next()
            .expect("the tool result must remain")
        else {
            panic!("expected tool result content");
        };
        let result_content = result.content.into_iter().collect::<Vec<_>>();

        assert_eq!(result.id, "call-1");
        assert!(
            matches!(&result_content[0], RigToolResultContent::Text(text) if text.text == "lookup failed")
        );
        assert!(
            matches!(&result_content[1], RigToolResultContent::Json { value } if value == &json!({ "code": "offline" }))
        );
    }

    #[test]
    fn developer_role_is_rejected_instead_of_becoming_system() {
        let request = CompletionRequest {
            messages: vec![Message::new(
                Role::Developer,
                vec![ContentPart::text("hidden")],
            )],
            ..CompletionRequest::default()
        };

        assert_eq!(
            request_parts(request, &GenerationOptions::default())
                .expect_err("Developer role must not be rewritten"),
            BridgeError::UnsupportedCapability {
                capability: "role.developer".to_owned(),
            }
        );
    }

    #[test]
    fn request_provider_data_is_rejected_instead_of_dropped() {
        let request = CompletionRequest {
            messages: vec![Message::new(
                Role::Assistant,
                vec![ContentPart::ProviderData(ProviderData {
                    provider: "openai".to_owned(),
                    kind: "reasoning".to_owned(),
                    value: json!({ "encrypted": "opaque" }),
                })],
            )],
            ..CompletionRequest::default()
        };

        assert_eq!(
            request_parts(request, &GenerationOptions::default())
                .expect_err("unknown request ProviderData must be explicit"),
            BridgeError::UnsupportedCapability {
                capability: "request_provider_data.openai.reasoning".to_owned(),
            }
        );
    }

    #[test]
    fn rig_specific_content_is_preserved_as_provider_data() {
        let content = assistant_content_from_rig(
            rig_core::OneOrMany::one(RigAssistantContent::reasoning("consider this")),
            "openai",
        )
        .expect("reasoning must serialize into ProviderData");

        assert!(matches!(
            &content[0],
            ArmillaeAssistantContent::ProviderData(data)
                if data.provider == "openai" && data.kind == "reasoning"
        ));
    }

    #[test]
    fn empty_provider_tool_call_id_is_rejected() {
        let result = assistant_content_from_rig(
            rig_core::OneOrMany::one(RigAssistantContent::ToolCall(RigToolCall {
                id: String::new(),
                call_id: None,
                function: ToolFunction::new("lookup".to_owned(), json!({ "query": "armillae" })),
                signature: None,
                additional_params: None,
            })),
            "openai",
        );

        assert!(matches!(
            result,
            Err(BridgeError::InvalidProviderResponse { metadata, .. })
                if metadata.provider == "openai"
        ));
    }
}
