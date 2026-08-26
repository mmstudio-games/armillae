use armillae_core::{
    AssistantContent as ArmillaeAssistantContent, CompletionRequest as ArmillaeCompletionRequest,
    ContentPart, GenerationOptions, Message as ArmillaeMessage, OutputFormat, ProviderData,
    ProviderExtensions, Role, TextContent, TokenUsage, ToolCallId,
    ToolChoice as ArmillaeToolChoice, ToolDefinition as ArmillaeToolDefinition,
    ToolResult as ArmillaeToolResult, ToolResultContent as ArmillaeToolResultContent,
};
use armillae_llm::{
    BridgeError, CompatibilityAction, CompatibilityFact, MessageContentLocation, ProjectionReport,
};
use rig_core::{
    OneOrMany,
    completion::{ToolDefinition as RigToolDefinition, Usage as RigUsage},
    message::{
        AssistantContent as RigAssistantContent, Message as RigMessage, Reasoning as RigReasoning,
        ReasoningContent as RigReasoningContent, Text as RigText, ToolCall as RigToolCall,
        ToolChoice as RigToolChoice, ToolFunction, ToolResult as RigToolResult,
        ToolResultContent as RigToolResultContent, UserContent as RigUserContent,
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
    pub(crate) projection_report: ProjectionReport,
}

pub(crate) fn request_parts(
    request: ArmillaeCompletionRequest,
    defaults: &GenerationOptions,
    target_provider: &str,
) -> Result<RequestParts, BridgeError> {
    let ArmillaeCompletionRequest {
        messages,
        tools,
        tool_choice,
        output_format,
        generation,
        extensions,
    } = request;

    let (chat_history, facts) = messages_to_rig(messages, target_provider)?;

    Ok(RequestParts {
        chat_history,
        tools: tools.into_iter().map(tool_definition_to_rig).collect(),
        tool_choice: tool_choice.map(tool_choice_to_rig).transpose()?,
        output_format,
        generation: merge_generation_options(defaults, generation),
        extensions,
        projection_report: ProjectionReport {
            target_provider: target_provider.to_owned(),
            facts,
        },
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
    target_provider: &str,
) -> Result<(OneOrMany<RigMessage>, Vec<CompatibilityFact>), BridgeError> {
    let mut converted = Vec::new();
    let mut facts = Vec::new();
    for (message_index, message) in messages.into_iter().enumerate() {
        converted.extend(message_to_rig(
            message,
            target_provider,
            message_index,
            &mut facts,
        )?);
    }

    let messages = one_or_many(
        converted,
        "completion request must contain at least one message",
    )?;
    Ok((messages, facts))
}

fn message_to_rig(
    message: ArmillaeMessage,
    target_provider: &str,
    message_index: usize,
    facts: &mut Vec<CompatibilityFact>,
) -> Result<Vec<RigMessage>, BridgeError> {
    if message.content.is_empty() {
        return invalid_request("messages must contain at least one content part");
    }

    match message.role {
        Role::System => {
            let mut converted = Vec::new();
            for (content_index, content) in message.content.into_iter().enumerate() {
                match content {
                    ContentPart::Text(text) => {
                        converted.push(RigMessage::System { content: text.text });
                    }
                    ContentPart::ProviderData(data) => handle_non_assistant_provider_data(
                        data,
                        target_provider,
                        message_index,
                        content_index,
                        facts,
                    )?,
                    _ => return invalid_request("system messages may contain only text"),
                }
            }
            Ok(converted)
        }
        Role::Developer => Err(BridgeError::UnsupportedCapability {
            capability: "role.developer".to_owned(),
        }),
        Role::User => {
            let mut content = Vec::new();
            for (content_index, item) in message.content.into_iter().enumerate() {
                match item {
                    ContentPart::ProviderData(data) => handle_non_assistant_provider_data(
                        data,
                        target_provider,
                        message_index,
                        content_index,
                        facts,
                    )?,
                    item => content.push(user_content_to_rig(item)?),
                }
            }
            if content.is_empty() {
                return Ok(Vec::new());
            }
            Ok(vec![RigMessage::User {
                content: one_or_many(content, "user messages must contain compatible content")?,
            }])
        }
        Role::Assistant => {
            let content =
                assistant_contents_to_rig(message.content, target_provider, message_index, facts)?;
            if content.is_empty() {
                return Ok(Vec::new());
            }
            Ok(vec![RigMessage::Assistant {
                id: None,
                content: one_or_many(
                    content,
                    "assistant messages must contain compatible content",
                )?,
            }])
        }
        Role::Tool => {
            let mut content = Vec::new();
            for (content_index, item) in message.content.into_iter().enumerate() {
                match item {
                    ContentPart::ToolResult(result) => {
                        content.push(RigUserContent::ToolResult(tool_result_to_rig(result)?));
                    }
                    ContentPart::ProviderData(data) => handle_non_assistant_provider_data(
                        data,
                        target_provider,
                        message_index,
                        content_index,
                        facts,
                    )?,
                    _ => {
                        return invalid_request(
                            "tool messages may contain only ToolResult content",
                        );
                    }
                }
            }
            if content.is_empty() {
                return Ok(Vec::new());
            }
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
        ContentPart::ProviderData(_) => {
            invalid_request("ProviderData must be handled by target Provider projection")
        }
        _ => Err(BridgeError::UnsupportedCapability {
            capability: "content_part.unknown".to_owned(),
        }),
    }
}

fn assistant_contents_to_rig(
    content: Vec<ContentPart>,
    target_provider: &str,
    message_index: usize,
    facts: &mut Vec<CompatibilityFact>,
) -> Result<Vec<RigAssistantContent>, BridgeError> {
    let mut converted = Vec::new();
    for (content_index, item) in content.into_iter().enumerate() {
        match item {
            ContentPart::Text(text) => {
                converted.push(RigAssistantContent::Text(RigText::new(text.text)));
            }
            ContentPart::ToolCall(call) => {
                converted.push(RigAssistantContent::ToolCall(RigToolCall {
                    id: call.id.into_inner(),
                    call_id: None,
                    function: ToolFunction::new(call.name, call.arguments),
                    signature: None,
                    additional_params: None,
                }));
            }
            ContentPart::ToolResult(_) => {
                return invalid_request("assistant messages cannot contain ToolResult content");
            }
            ContentPart::ProviderData(data) => {
                project_assistant_provider_data(
                    data,
                    target_provider,
                    message_index,
                    content_index,
                    &mut converted,
                    facts,
                )?;
            }
            _ => {
                return Err(BridgeError::UnsupportedCapability {
                    capability: "content_part.unknown".to_owned(),
                });
            }
        }
    }
    Ok(converted)
}

fn project_assistant_provider_data(
    data: ProviderData,
    target_provider: &str,
    message_index: usize,
    content_index: usize,
    converted: &mut Vec<RigAssistantContent>,
    facts: &mut Vec<CompatibilityFact>,
) -> Result<(), BridgeError> {
    if data.provider != target_provider {
        record_not_forwarded(data, target_provider, message_index, content_index, facts);
        return Ok(());
    }

    match data.kind.as_str() {
        "reasoning" => {
            let reasoning = serde_json::from_value::<RigReasoning>(data.value).map_err(|_| {
                projection_incompatible(target_provider, message_index, content_index, "reasoning")
            })?;
            if !reasoning_is_replayable(target_provider, &reasoning) {
                return Err(projection_incompatible(
                    target_provider,
                    message_index,
                    content_index,
                    "reasoning",
                ));
            }
            converted.push(RigAssistantContent::Reasoning(reasoning));
            Ok(())
        }
        "tool_call_metadata" => replay_tool_call_metadata(
            data.value,
            target_provider,
            message_index,
            content_index,
            converted,
        ),
        _ => {
            record_not_forwarded(data, target_provider, message_index, content_index, facts);
            Ok(())
        }
    }
}

fn replay_tool_call_metadata(
    value: Value,
    target_provider: &str,
    message_index: usize,
    content_index: usize,
    converted: &mut [RigAssistantContent],
) -> Result<(), BridgeError> {
    let Value::Object(mut metadata) = value else {
        return Err(projection_incompatible(
            target_provider,
            message_index,
            content_index,
            "tool_call_metadata",
        ));
    };
    let call_id = take_optional_non_empty_string(&mut metadata, "call_id").ok_or_else(|| {
        projection_incompatible(
            target_provider,
            message_index,
            content_index,
            "tool_call_metadata",
        )
    })?;
    let signature =
        take_optional_non_empty_string(&mut metadata, "signature").ok_or_else(|| {
            projection_incompatible(
                target_provider,
                message_index,
                content_index,
                "tool_call_metadata",
            )
        })?;
    let additional_params = metadata.remove("additional_params");
    if !metadata.is_empty()
        || (call_id.is_none() && signature.is_none() && additional_params.is_none())
    {
        return Err(projection_incompatible(
            target_provider,
            message_index,
            content_index,
            "tool_call_metadata",
        ));
    }

    let Some(RigAssistantContent::ToolCall(call)) = converted.last_mut() else {
        return Err(projection_incompatible(
            target_provider,
            message_index,
            content_index,
            "tool_call_metadata",
        ));
    };
    call.call_id = call_id;
    call.signature = signature;
    call.additional_params = additional_params;
    Ok(())
}

fn reasoning_is_replayable(target_provider: &str, reasoning: &RigReasoning) -> bool {
    if reasoning.id.is_some() || reasoning.content.is_empty() {
        return false;
    }
    if target_provider == "anthropic" {
        return true;
    }
    reasoning.content.iter().all(|content| {
        matches!(
            content,
            RigReasoningContent::Text {
                text,
                signature: None,
            } if !text.is_empty()
        )
    })
}

fn take_optional_non_empty_string(
    object: &mut Map<String, Value>,
    name: &str,
) -> Option<Option<String>> {
    match object.remove(name) {
        None => Some(None),
        Some(Value::String(value)) if !value.is_empty() => Some(Some(value)),
        Some(_) => None,
    }
}

fn handle_non_assistant_provider_data(
    data: ProviderData,
    target_provider: &str,
    message_index: usize,
    content_index: usize,
    facts: &mut Vec<CompatibilityFact>,
) -> Result<(), BridgeError> {
    if data.provider == target_provider
        && matches!(data.kind.as_str(), "reasoning" | "tool_call_metadata")
    {
        return Err(projection_incompatible(
            target_provider,
            message_index,
            content_index,
            &data.kind,
        ));
    }
    record_not_forwarded(data, target_provider, message_index, content_index, facts);
    Ok(())
}

fn record_not_forwarded(
    data: ProviderData,
    target_provider: &str,
    message_index: usize,
    content_index: usize,
    facts: &mut Vec<CompatibilityFact>,
) {
    facts.push(CompatibilityFact {
        location: MessageContentLocation {
            message_index,
            content_index,
        },
        source_provider: data.provider,
        target_provider: target_provider.to_owned(),
        kind: data.kind,
        action: CompatibilityAction::NotForwarded,
        lossy: true,
    });
}

fn projection_incompatible(
    target_provider: &str,
    message_index: usize,
    content_index: usize,
    kind: &str,
) -> BridgeError {
    BridgeError::ProjectionIncompatible {
        target_provider: target_provider.to_owned(),
        message_index,
        content_index,
        kind: kind.to_owned(),
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
                if reasoning_is_semantically_empty(&reasoning) {
                    continue;
                }
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

pub(crate) fn reasoning_is_semantically_empty(reasoning: &RigReasoning) -> bool {
    reasoning.id.is_none()
        && matches!(
            reasoning.content.as_slice(),
            [RigReasoningContent::Text {
                text,
                signature: None,
            }] if text.is_empty()
        )
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
    use armillae_llm::{BridgeError, CompatibilityAction};
    use rig_core::{
        OneOrMany,
        message::{
            AssistantContent as RigAssistantContent, Message as RigMessage,
            Reasoning as RigReasoning, ToolCall as RigToolCall, ToolFunction,
            ToolResultContent as RigToolResultContent, UserContent as RigUserContent,
        },
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
    fn semantically_empty_reasoning_is_absent_for_every_supported_provider() {
        for provider in [
            "openai",
            "openai-compatible",
            "deepseek",
            "minimax",
            "moonshot",
            "anthropic",
            "ollama",
        ] {
            let converted = assistant_content_from_rig(
                OneOrMany::one(RigAssistantContent::Reasoning(RigReasoning::new(""))),
                provider,
            )
            .unwrap_or_else(|error| panic!("{provider} empty reasoning must normalize: {error}"));

            assert!(
                converted.is_empty(),
                "{provider} empty reasoning must not enter canonical history"
            );
        }
    }

    #[test]
    fn state_bearing_and_noncanonical_empty_reasoning_are_preserved() {
        let reasonings = vec![
            RigReasoning::new("").with_id("reasoning-id".to_owned()),
            RigReasoning::new_with_signature("", Some("signature".to_owned())),
            RigReasoning::encrypted(""),
            RigReasoning::redacted(""),
            RigReasoning::summaries(vec![String::new()]),
            RigReasoning::multi(vec![String::new(), String::new()]),
            serde_json::from_value(json!({ "id": null, "content": [] }))
                .expect("empty reasoning fixture must deserialize"),
        ];
        let choice = OneOrMany::many(reasonings.into_iter().map(RigAssistantContent::Reasoning))
            .expect("meaningful reasoning fixtures must be non-empty");

        let converted = assistant_content_from_rig(choice, "anthropic")
            .expect("state-bearing reasoning must normalize");

        assert_eq!(converted.len(), 7);
        assert!(converted.iter().all(|content| matches!(
            content,
            ArmillaeAssistantContent::ProviderData(data)
                if data.provider == "anthropic" && data.kind == "reasoning"
        )));
        assert!(matches!(
            &converted[0],
            ArmillaeAssistantContent::ProviderData(data)
                if data.value["id"] == "reasoning-id"
        ));
        assert!(matches!(
            &converted[1],
            ArmillaeAssistantContent::ProviderData(data)
                if data.value["content"][0]["content"]["signature"] == "signature"
        ));
        assert!(matches!(
            &converted[2],
            ArmillaeAssistantContent::ProviderData(data)
                if data.value["content"][0]["type"] == "encrypted"
        ));
        assert!(matches!(
            &converted[3],
            ArmillaeAssistantContent::ProviderData(data)
                if data.value["content"][0]["type"] == "redacted"
        ));
        assert!(matches!(
            &converted[4],
            ArmillaeAssistantContent::ProviderData(data)
                if data.value["content"][0]["type"] == "summary"
        ));
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

        let parts = request_parts(request, &GenerationOptions::default(), "openai")
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

        let parts = request_parts(request, &GenerationOptions::default(), "openai")
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
            request_parts(request, &GenerationOptions::default(), "openai")
                .expect_err("Developer role must not be rewritten"),
            BridgeError::UnsupportedCapability {
                capability: "role.developer".to_owned(),
            }
        );
    }

    #[test]
    fn same_provider_reasoning_round_trips_for_every_supported_target() {
        for provider in [
            "openai",
            "openai-compatible",
            "deepseek",
            "minimax",
            "moonshot",
            "anthropic",
            "ollama",
        ] {
            let canonical = assistant_content_from_rig(
                rig_core::OneOrMany::one(RigAssistantContent::reasoning("consider this")),
                provider,
            )
            .expect("reasoning response content must normalize");
            let request = CompletionRequest {
                messages: vec![Message::assistant(
                    canonical.into_iter().map(ContentPart::from).collect(),
                )],
                ..CompletionRequest::default()
            };

            let parts = request_parts(request, &GenerationOptions::default(), provider)
                .expect("same-Provider reasoning must project");
            assert!(parts.projection_report.is_exact());
            let RigMessage::Assistant { content, .. } = parts
                .chat_history
                .into_iter()
                .next()
                .expect("projected assistant message must remain")
            else {
                panic!("expected assistant message for {provider}");
            };
            assert!(matches!(
                content.into_iter().next(),
                Some(RigAssistantContent::Reasoning(reasoning))
                    if reasoning.display_text() == "consider this"
            ));
        }
    }

    #[test]
    fn foreign_and_unknown_provider_data_are_reported_without_mutating_canonical_history() {
        let request = CompletionRequest {
            messages: vec![
                Message::assistant(vec![
                    ContentPart::text("visible"),
                    ContentPart::ProviderData(ProviderData {
                        provider: "deepseek".to_owned(),
                        kind: "reasoning".to_owned(),
                        value: serde_json::to_value(rig_core::message::Reasoning::new("private"))
                            .expect("fixture reasoning must serialize"),
                    }),
                    ContentPart::ProviderData(ProviderData {
                        provider: "anthropic".to_owned(),
                        kind: "future_private_block".to_owned(),
                        value: json!({ "opaque": true }),
                    }),
                ]),
                Message::user("continue"),
            ],
            ..CompletionRequest::default()
        };
        let original = request.clone();

        let parts = request_parts(request, &GenerationOptions::default(), "anthropic")
            .expect("foreign and unknown ProviderData must not block projection");

        assert_eq!(original.messages[0].content.len(), 3);
        assert_eq!(parts.projection_report.facts.len(), 2);
        assert!(parts.projection_report.facts.iter().all(|fact| {
            fact.target_provider == "anthropic"
                && fact.action == CompatibilityAction::NotForwarded
                && fact.lossy
        }));
        let RigMessage::Assistant { content, .. } = parts
            .chat_history
            .into_iter()
            .next()
            .expect("portable assistant content must remain")
        else {
            panic!("expected assistant message");
        };
        let projected = content.into_iter().collect::<Vec<_>>();
        assert_eq!(projected.len(), 1);
        assert!(matches!(&projected[0], RigAssistantContent::Text(text) if text.text == "visible"));
    }

    #[test]
    fn malformed_same_provider_replay_data_is_projection_incompatible() {
        let request = CompletionRequest {
            messages: vec![Message::assistant(vec![ContentPart::ProviderData(
                ProviderData {
                    provider: "deepseek".to_owned(),
                    kind: "reasoning".to_owned(),
                    value: json!({ "not": "rig reasoning" }),
                },
            )])],
            ..CompletionRequest::default()
        };

        assert_eq!(
            request_parts(request, &GenerationOptions::default(), "deepseek")
                .expect_err("malformed known replay data must fail projection"),
            BridgeError::ProjectionIncompatible {
                target_provider: "deepseek".to_owned(),
                message_index: 0,
                content_index: 0,
                kind: "reasoning".to_owned(),
            }
        );
    }

    #[test]
    fn same_provider_tool_call_metadata_replays_onto_the_preceding_call() {
        let canonical = assistant_content_from_rig(
            rig_core::OneOrMany::one(RigAssistantContent::ToolCall(RigToolCall {
                id: "call-1".to_owned(),
                call_id: Some("provider-call-1".to_owned()),
                function: ToolFunction::new("lookup".to_owned(), json!({ "q": "armillae" })),
                signature: Some("signature-1".to_owned()),
                additional_params: Some(json!({ "future": true })),
            })),
            "openai",
        )
        .expect("ToolCall metadata must normalize");
        let request = CompletionRequest {
            messages: vec![Message::assistant(
                canonical.into_iter().map(ContentPart::from).collect(),
            )],
            ..CompletionRequest::default()
        };

        let parts = request_parts(request, &GenerationOptions::default(), "openai")
            .expect("same-Provider ToolCall metadata must replay");
        assert!(parts.projection_report.is_exact());
        let RigMessage::Assistant { content, .. } = parts
            .chat_history
            .into_iter()
            .next()
            .expect("assistant ToolCall must remain")
        else {
            panic!("expected assistant message");
        };
        let Some(RigAssistantContent::ToolCall(call)) = content.into_iter().next() else {
            panic!("expected replayed ToolCall");
        };
        assert_eq!(call.id, "call-1");
        assert_eq!(call.call_id.as_deref(), Some("provider-call-1"));
        assert_eq!(call.signature.as_deref(), Some("signature-1"));
        assert_eq!(call.additional_params, Some(json!({ "future": true })));
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
