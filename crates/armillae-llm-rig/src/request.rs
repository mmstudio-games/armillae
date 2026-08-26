use armillae_core::{
    CompletionRequest as ArmillaeCompletionRequest, ContentPart, GenerationOptions, OutputFormat,
    Role, ToolCallId,
};
use armillae_llm::BridgeError;
use rig_core::completion::CompletionRequest as RigCompletionRequest;
use serde_json::{Map, Value, json};

use crate::convert;

pub(crate) trait RigRequestMapper: Send + Sync {
    fn map_request(
        &self,
        request: ArmillaeCompletionRequest,
        defaults: &GenerationOptions,
    ) -> Result<RigCompletionRequest, BridgeError>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct AnthropicRequestMapper;

impl AnthropicRequestMapper {
    pub(crate) fn new(provider_options: Value) -> Result<Self, BridgeError> {
        let Value::Object(options) = provider_options else {
            return invalid_configuration("Anthropic provider_options must be a JSON object");
        };
        if !options.is_empty() {
            return invalid_configuration("Anthropic provider_options are not supported");
        }
        Ok(Self)
    }
}

impl RigRequestMapper for AnthropicRequestMapper {
    fn map_request(
        &self,
        request: ArmillaeCompletionRequest,
        defaults: &GenerationOptions,
    ) -> Result<RigCompletionRequest, BridgeError> {
        validate_anthropic_messages(&request)?;
        let parts = convert::request_parts(request, defaults)?;
        if !parts.extensions.values.is_empty() {
            return invalid_request("Anthropic request extensions are not supported");
        }
        if parts.generation.seed.is_some() {
            return Err(BridgeError::UnsupportedCapability {
                capability: "generation.seed".to_owned(),
            });
        }
        let Some(max_tokens) = parts.generation.max_output_tokens else {
            return invalid_request("Anthropic requires max_output_tokens");
        };

        let mut additional_params = Map::new();
        if !parts.generation.stop.is_empty() {
            additional_params.insert("stop_sequences".to_owned(), json!(parts.generation.stop));
        }
        let mut mapped = RigCompletionRequest {
            model: None,
            preamble: None,
            chat_history: parts.chat_history,
            documents: Vec::new(),
            tools: parts.tools,
            temperature: parts.generation.temperature,
            max_tokens: Some(max_tokens),
            tool_choice: parts.tool_choice,
            additional_params: (!additional_params.is_empty())
                .then_some(Value::Object(additional_params)),
            output_schema: None,
            record_telemetry_content: false,
        };
        apply_anthropic_output_format(&mut mapped, parts.output_format)?;
        Ok(mapped)
    }
}

fn validate_anthropic_messages(request: &ArmillaeCompletionRequest) -> Result<(), BridgeError> {
    let mut saw_non_system = false;

    for message in &request.messages {
        if message.role == Role::System {
            if saw_non_system {
                return invalid_request(
                    "Anthropic system messages must precede all conversation messages",
                );
            }
        } else {
            saw_non_system = true;
        }

        for content in &message.content {
            if matches!(content, ContentPart::ToolResult(result) if result.is_error) {
                return invalid_request(
                    "Rig 0.41 cannot preserve Anthropic ToolResult.is_error = true",
                );
            }
        }
    }

    if !saw_non_system {
        return invalid_request("Anthropic requires at least one non-system message");
    }
    Ok(())
}

fn apply_anthropic_output_format(
    request: &mut RigCompletionRequest,
    output_format: Option<OutputFormat>,
) -> Result<(), BridgeError> {
    match output_format {
        None | Some(OutputFormat::Text) => Ok(()),
        Some(OutputFormat::JsonSchema {
            name,
            schema,
            strict,
        }) => {
            if name.trim().is_empty() {
                return invalid_request("JSON Schema output name must not be empty");
            }
            if !strict {
                return Err(BridgeError::UnsupportedCapability {
                    capability: "output_format.json_schema.non_strict".to_owned(),
                });
            }
            validate_anthropic_schema(&schema)?;
            request.output_schema =
                Some(
                    serde_json::from_value(schema).map_err(|_| BridgeError::InvalidRequest {
                        message: "Anthropic output schema is not a valid JSON Schema".to_owned(),
                    })?,
                );
            Ok(())
        }
        Some(OutputFormat::JsonObject) => Err(BridgeError::UnsupportedCapability {
            capability: "output_format.json_object".to_owned(),
        }),
        Some(_) => Err(BridgeError::UnsupportedCapability {
            capability: "output_format.unknown".to_owned(),
        }),
    }
}

fn validate_anthropic_schema(schema: &Value) -> Result<(), BridgeError> {
    let Value::Object(object) = schema else {
        return invalid_request("Anthropic output schema must be a JSON object");
    };

    let is_object = object.get("type") == Some(&Value::String("object".to_owned()))
        || object.contains_key("properties");
    if is_object {
        if object.get("additionalProperties") != Some(&Value::Bool(false)) {
            return invalid_request(
                "Anthropic object schemas require additionalProperties = false",
            );
        }
        if let Some(Value::Object(properties)) = object.get("properties") {
            let Some(Value::Array(required)) = object.get("required") else {
                return invalid_request("Anthropic object schemas require every property");
            };
            let required = required
                .iter()
                .map(Value::as_str)
                .collect::<Option<std::collections::BTreeSet<_>>>()
                .ok_or_else(|| BridgeError::InvalidRequest {
                    message: "Anthropic schema required entries must be strings".to_owned(),
                })?;
            if required.len() != properties.len()
                || properties
                    .keys()
                    .any(|name| !required.contains(name.as_str()))
            {
                return invalid_request("Anthropic object schemas require every property");
            }
        }
    }

    if matches!(
        object.get("type").and_then(Value::as_str),
        Some("integer" | "number")
    ) && [
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
    ]
    .iter()
    .any(|name| object.contains_key(*name))
    {
        return invalid_request("Anthropic numeric schemas do not support numeric constraints");
    }
    if object.contains_key("oneOf") {
        return invalid_request("Anthropic output schemas do not support oneOf");
    }

    for name in ["$defs", "properties"] {
        if let Some(Value::Object(children)) = object.get(name) {
            for child in children.values() {
                validate_anthropic_schema(child)?;
            }
        }
    }
    if let Some(items) = object.get("items") {
        validate_anthropic_schema(items)?;
    }
    for name in ["anyOf", "allOf"] {
        if let Some(Value::Array(children)) = object.get(name) {
            for child in children {
                validate_anthropic_schema(child)?;
            }
        }
    }

    Ok(())
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OllamaRequestMapper;

impl OllamaRequestMapper {
    pub(crate) fn new(provider_options: Value) -> Result<Self, BridgeError> {
        let Value::Object(options) = provider_options else {
            return invalid_configuration("Ollama provider_options must be a JSON object");
        };
        if !options.is_empty() {
            return invalid_configuration("Ollama provider_options are not supported");
        }
        Ok(Self)
    }
}

impl RigRequestMapper for OllamaRequestMapper {
    fn map_request(
        &self,
        mut request: ArmillaeCompletionRequest,
        defaults: &GenerationOptions,
    ) -> Result<RigCompletionRequest, BridgeError> {
        rewrite_ollama_tool_result_ids(&mut request)?;
        let parts = convert::request_parts(request, defaults)?;
        if !parts.extensions.values.is_empty() {
            return invalid_request("Ollama request extensions are not supported");
        }
        if parts.tool_choice.is_some() {
            return Err(BridgeError::UnsupportedCapability {
                capability: "tool_choice".to_owned(),
            });
        }

        let mut additional_params = Map::new();
        if !parts.generation.stop.is_empty() {
            additional_params.insert("stop".to_owned(), json!(parts.generation.stop));
        }
        if let Some(seed) = parts.generation.seed {
            additional_params.insert("seed".to_owned(), json!(seed));
        }

        let mut mapped = RigCompletionRequest {
            model: None,
            preamble: None,
            chat_history: parts.chat_history,
            documents: Vec::new(),
            tools: parts.tools,
            temperature: parts.generation.temperature,
            max_tokens: parts.generation.max_output_tokens,
            tool_choice: None,
            additional_params: (!additional_params.is_empty())
                .then_some(Value::Object(additional_params)),
            output_schema: None,
            record_telemetry_content: false,
        };
        apply_ollama_output_format(&mut mapped, parts.output_format)?;
        Ok(mapped)
    }
}

fn rewrite_ollama_tool_result_ids(
    request: &mut ArmillaeCompletionRequest,
) -> Result<(), BridgeError> {
    let mut tool_names = std::collections::BTreeMap::new();

    for message in &mut request.messages {
        for content in &mut message.content {
            match content {
                ContentPart::ToolCall(call) => {
                    if call.name.trim().is_empty() {
                        return invalid_request("Ollama ToolCall names must not be empty");
                    }
                    tool_names.insert(call.id.as_str().to_owned(), call.name.clone());
                }
                ContentPart::ToolResult(result) => {
                    let name = tool_names.get(result.call_id.as_str()).ok_or_else(|| {
                        BridgeError::InvalidRequest {
                            message: "Ollama ToolResult requires its preceding Assistant ToolCall"
                                .to_owned(),
                        }
                    })?;
                    result.call_id =
                        ToolCallId::new(name.clone()).map_err(|_| BridgeError::InvalidRequest {
                            message: "Ollama ToolResult resolved to an empty tool name".to_owned(),
                        })?;
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn apply_ollama_output_format(
    request: &mut RigCompletionRequest,
    output_format: Option<OutputFormat>,
) -> Result<(), BridgeError> {
    let schema = match output_format {
        None | Some(OutputFormat::Text) => return Ok(()),
        Some(OutputFormat::JsonObject) => json!({ "type": "object" }),
        Some(OutputFormat::JsonSchema {
            name,
            schema,
            strict,
        }) => {
            if name.trim().is_empty() {
                return invalid_request("JSON Schema output name must not be empty");
            }
            if !schema.is_object() {
                return invalid_request("Ollama output schema must be a JSON object");
            }
            if !strict {
                return Err(BridgeError::UnsupportedCapability {
                    capability: "output_format.json_schema.non_strict".to_owned(),
                });
            }
            schema
        }
        Some(_) => {
            return Err(BridgeError::UnsupportedCapability {
                capability: "output_format.unknown".to_owned(),
            });
        }
    };

    request.output_schema =
        Some(
            serde_json::from_value(schema).map_err(|_| BridgeError::InvalidRequest {
                message: "Ollama output schema is not a valid JSON Schema".to_owned(),
            })?,
        );
    Ok(())
}

#[derive(Clone, Debug)]
pub(crate) struct OpenAiRequestMapper {
    namespace: &'static str,
    provider_label: &'static str,
    allow_reasoning_effort: bool,
    provider_options: Map<String, Value>,
}

impl OpenAiRequestMapper {
    pub(crate) fn new(provider_options: Value) -> Result<Self, BridgeError> {
        Self::build("openai", "OpenAI", true, provider_options)
    }

    pub(crate) fn for_named_provider(
        namespace: &'static str,
        provider_label: &'static str,
        provider_options: Value,
    ) -> Result<Self, BridgeError> {
        Self::build(namespace, provider_label, false, provider_options)
    }

    fn build(
        namespace: &'static str,
        provider_label: &'static str,
        allow_reasoning_effort: bool,
        provider_options: Value,
    ) -> Result<Self, BridgeError> {
        let Value::Object(provider_options) = provider_options else {
            return invalid_configuration(format!(
                "{provider_label} provider_options must be a JSON object"
            ));
        };
        validate_provider_options(&provider_options, provider_label, allow_reasoning_effort)?;
        Ok(Self {
            namespace,
            provider_label,
            allow_reasoning_effort,
            provider_options,
        })
    }
}

impl Default for OpenAiRequestMapper {
    fn default() -> Self {
        Self {
            namespace: "openai",
            provider_label: "OpenAI",
            allow_reasoning_effort: true,
            provider_options: Map::new(),
        }
    }
}

impl RigRequestMapper for OpenAiRequestMapper {
    fn map_request(
        &self,
        request: ArmillaeCompletionRequest,
        defaults: &GenerationOptions,
    ) -> Result<RigCompletionRequest, BridgeError> {
        let parts = convert::request_parts(request, defaults)?;
        let mut additional_params = self.provider_options.clone();
        merge_request_extensions(
            &mut additional_params,
            parts.extensions.values,
            self.namespace,
            self.provider_label,
            self.allow_reasoning_effort,
        )?;

        if !parts.generation.stop.is_empty() {
            additional_params.insert("stop".to_owned(), json!(parts.generation.stop));
        }
        if let Some(seed) = parts.generation.seed {
            additional_params.insert("seed".to_owned(), json!(seed));
        }
        apply_output_format(&mut additional_params, parts.output_format)?;

        Ok(RigCompletionRequest {
            model: None,
            preamble: None,
            chat_history: parts.chat_history,
            documents: Vec::new(),
            tools: parts.tools,
            temperature: parts.generation.temperature,
            max_tokens: parts.generation.max_output_tokens,
            tool_choice: parts.tool_choice,
            additional_params: (!additional_params.is_empty())
                .then_some(Value::Object(additional_params)),
            output_schema: None,
            record_telemetry_content: false,
        })
    }
}

fn validate_provider_options(
    options: &Map<String, Value>,
    provider_label: &str,
    allow_reasoning_effort: bool,
) -> Result<(), BridgeError> {
    for (name, value) in options {
        if is_standard_field(name) {
            return invalid_configuration(format!(
                "{provider_label} provider_options cannot override standard field: {name}"
            ));
        }
        validate_provider_option(name, value, true, provider_label, allow_reasoning_effort)?;
    }
    Ok(())
}

fn merge_request_extensions(
    target: &mut Map<String, Value>,
    extensions: impl IntoIterator<Item = (String, Value)>,
    namespace: &str,
    provider_label: &str,
    allow_reasoning_effort: bool,
) -> Result<(), BridgeError> {
    for (key, value) in extensions {
        let expected_prefix = format!("{namespace}.");
        let Some(name) = key.strip_prefix(&expected_prefix) else {
            return invalid_request(format!(
                "{provider_label} Adapter does not accept extension namespace: {key}"
            ));
        };
        if is_standard_field(name) {
            return invalid_request(format!(
                "{provider_label} extension cannot override standard field: {name}"
            ));
        }
        validate_provider_option(name, &value, false, provider_label, allow_reasoning_effort)?;
        target.insert(name.to_owned(), value);
    }
    Ok(())
}

fn validate_provider_option(
    name: &str,
    value: &Value,
    configuration: bool,
    provider_label: &str,
    allow_reasoning_effort: bool,
) -> Result<(), BridgeError> {
    match name {
        "reasoning_effort"
            if allow_reasoning_effort && value.as_str().is_some_and(|value| !value.is_empty()) =>
        {
            Ok(())
        }
        "reasoning_effort" if allow_reasoning_effort => option_error(
            configuration,
            "OpenAI reasoning_effort must be a non-empty string",
        ),
        _ => option_error(
            configuration,
            format!("unknown {provider_label} option: {name}"),
        ),
    }
}

fn apply_output_format(
    additional_params: &mut Map<String, Value>,
    output_format: Option<OutputFormat>,
) -> Result<(), BridgeError> {
    match output_format {
        None | Some(OutputFormat::Text) => Ok(()),
        Some(OutputFormat::JsonObject) => {
            additional_params.insert(
                "response_format".to_owned(),
                json!({ "type": "json_object" }),
            );
            Ok(())
        }
        Some(OutputFormat::JsonSchema {
            name,
            schema,
            strict,
        }) => {
            if name.trim().is_empty() {
                return invalid_request("JSON Schema output name must not be empty");
            }
            if !schema.is_object() {
                return invalid_request("JSON Schema output schema must be a JSON object");
            }
            additional_params.insert(
                "response_format".to_owned(),
                json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": name,
                        "strict": strict,
                        "schema": schema,
                    }
                }),
            );
            Ok(())
        }
        Some(_) => Err(BridgeError::UnsupportedCapability {
            capability: "output_format.unknown".to_owned(),
        }),
    }
}

fn is_standard_field(name: &str) -> bool {
    matches!(
        name,
        "temperature"
            | "max_tokens"
            | "max_output_tokens"
            | "stop"
            | "seed"
            | "response_format"
            | "tool_choice"
            | "tools"
    )
}

fn option_error<T>(configuration: bool, message: impl Into<String>) -> Result<T, BridgeError> {
    if configuration {
        invalid_configuration(message)
    } else {
        invalid_request(message)
    }
}

fn invalid_configuration<T>(message: impl Into<String>) -> Result<T, BridgeError> {
    Err(BridgeError::InvalidConfiguration {
        message: message.into(),
    })
}

fn invalid_request<T>(message: impl Into<String>) -> Result<T, BridgeError> {
    Err(BridgeError::InvalidRequest {
        message: message.into(),
    })
}

#[cfg(test)]
mod tests {
    use armillae_core::{
        CompletionRequest, ContentPart, GenerationOptions, Message, OutputFormat,
        ProviderExtensions, Role, ToolCallId, ToolResult, ToolResultContent,
    };
    use armillae_llm::BridgeError;
    use rig_core::{message::Message as RigMessage, providers::openai};
    use serde_json::{Value, json};

    use super::{OpenAiRequestMapper, RigRequestMapper};

    fn tool_call_id(value: &str) -> ToolCallId {
        ToolCallId::new(value).expect("fixture ToolCall IDs are non-empty")
    }

    #[test]
    fn openai_mapper_preserves_generation_and_json_object_format() {
        let mapper = OpenAiRequestMapper::default();
        let request = CompletionRequest {
            messages: vec![Message::user("hello")],
            output_format: Some(OutputFormat::JsonObject),
            generation: GenerationOptions {
                temperature: Some(0.7),
                max_output_tokens: Some(512),
                stop: vec!["END".to_owned()],
                seed: Some(42),
            },
            ..CompletionRequest::default()
        };

        let mapped = mapper
            .map_request(request, &GenerationOptions::default())
            .expect("valid OpenAI generation options must map");
        let additional = mapped
            .additional_params
            .expect("provider wire fields must be present");

        assert_eq!(mapped.temperature, Some(0.7));
        assert_eq!(mapped.max_tokens, Some(512));
        assert_eq!(additional["stop"], json!(["END"]));
        assert_eq!(additional["seed"], 42);
        assert_eq!(additional["response_format"]["type"], "json_object");
        assert!(mapped.output_schema.is_none());
    }

    #[test]
    fn openai_mapper_preserves_schema_name_and_strict_value() {
        let mapper = OpenAiRequestMapper::default();
        let schema = json!({
            "type": "object",
            "properties": { "answer": { "type": "string" } },
            "required": ["answer"],
            "additionalProperties": false
        });
        let request = CompletionRequest {
            messages: vec![Message::user("hello")],
            output_format: Some(OutputFormat::JsonSchema {
                name: "answer_payload".to_owned(),
                schema: schema.clone(),
                strict: false,
            }),
            ..CompletionRequest::default()
        };

        let mapped = mapper
            .map_request(request, &GenerationOptions::default())
            .expect("valid JSON Schema output must map");
        let format = &mapped
            .additional_params
            .expect("response_format must be present")["response_format"];

        assert_eq!(format["type"], "json_schema");
        assert_eq!(format["json_schema"]["name"], "answer_payload");
        assert_eq!(format["json_schema"]["strict"], false);
        assert_eq!(format["json_schema"]["schema"], schema);
        assert!(mapped.output_schema.is_none());
    }

    #[test]
    fn request_extension_overrides_provider_specific_default() {
        let mapper = OpenAiRequestMapper::new(json!({ "reasoning_effort": "low" }))
            .expect("known provider default must validate");
        let request = CompletionRequest {
            messages: vec![Message::user("hello")],
            extensions: ProviderExtensions {
                values: [("openai.reasoning_effort".to_owned(), json!("high"))]
                    .into_iter()
                    .collect(),
            },
            ..CompletionRequest::default()
        };

        let mapped = mapper
            .map_request(request, &GenerationOptions::default())
            .expect("known request extension must map");

        assert_eq!(
            mapped
                .additional_params
                .expect("reasoning_effort must be present")["reasoning_effort"],
            "high"
        );
    }

    #[test]
    fn unknown_namespace_and_standard_field_override_are_rejected() {
        let mapper = OpenAiRequestMapper::default();
        let request_with_foreign_namespace = CompletionRequest {
            messages: vec![Message::user("hello")],
            extensions: ProviderExtensions {
                values: [("anthropic.thinking".to_owned(), json!(true))]
                    .into_iter()
                    .collect(),
            },
            ..CompletionRequest::default()
        };
        let request_overriding_seed = CompletionRequest {
            messages: vec![Message::user("hello")],
            extensions: ProviderExtensions {
                values: [("openai.seed".to_owned(), json!(99))]
                    .into_iter()
                    .collect(),
            },
            ..CompletionRequest::default()
        };

        assert!(matches!(
            mapper.map_request(
                request_with_foreign_namespace,
                &GenerationOptions::default()
            ),
            Err(BridgeError::InvalidRequest { .. })
        ));
        assert!(matches!(
            mapper.map_request(request_overriding_seed, &GenerationOptions::default()),
            Err(BridgeError::InvalidRequest { .. })
        ));
    }

    #[test]
    fn openai_tool_result_omits_is_error_without_rewriting_content() {
        let mapper = OpenAiRequestMapper::default();
        let request = CompletionRequest {
            messages: vec![Message::new(
                Role::Tool,
                vec![ContentPart::ToolResult(ToolResult {
                    call_id: tool_call_id("call-1"),
                    content: vec![ToolResultContent::Text {
                        text: "lookup failed".to_owned(),
                    }],
                    is_error: true,
                })],
            )],
            ..CompletionRequest::default()
        };

        let mapped = mapper
            .map_request(request, &GenerationOptions::default())
            .expect("error ToolResult must remain sendable");
        let native = mapped
            .chat_history
            .into_iter()
            .map(Vec::<openai::completion::Message>::try_from)
            .collect::<Result<Vec<_>, _>>()
            .expect("Rig history must convert to OpenAI")
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        let wire = serde_json::to_value(native).expect("native OpenAI messages must serialize");
        let tool_message = wire
            .as_array()
            .and_then(|messages| messages.first())
            .expect("one tool message must be serialized");

        assert_eq!(tool_message["role"], "tool");
        assert_eq!(tool_message["tool_call_id"], "call-1");
        assert_eq!(tool_message["content"], "lookup failed");
        assert_eq!(tool_message.get("is_error"), None);
    }

    #[test]
    fn provider_options_cannot_override_standard_fields() {
        assert!(matches!(
            OpenAiRequestMapper::new(json!({ "response_format": { "type": "json_object" } })),
            Err(BridgeError::InvalidConfiguration { .. })
        ));
        assert!(matches!(
            OpenAiRequestMapper::new(Value::String("invalid".to_owned())),
            Err(BridgeError::InvalidConfiguration { .. })
        ));
    }

    #[test]
    fn named_provider_mapper_rejects_options_and_foreign_namespaces() {
        assert!(matches!(
            OpenAiRequestMapper::for_named_provider(
                "deepseek",
                "DeepSeek",
                json!({ "thinking": "disabled" })
            ),
            Err(BridgeError::InvalidConfiguration { .. })
        ));

        let mapper = OpenAiRequestMapper::for_named_provider("deepseek", "DeepSeek", json!({}))
            .expect("empty named Provider options must validate");
        for key in ["openai.reasoning_effort", "deepseek.future"] {
            let request = CompletionRequest {
                messages: vec![Message::user("hello")],
                extensions: ProviderExtensions {
                    values: [(key.to_owned(), json!(true))].into_iter().collect(),
                },
                ..CompletionRequest::default()
            };

            assert!(matches!(
                mapper.map_request(request, &GenerationOptions::default()),
                Err(BridgeError::InvalidRequest { .. })
            ));
        }
    }

    #[test]
    fn system_messages_remain_in_history_instead_of_becoming_preamble() {
        let mapper = OpenAiRequestMapper::default();
        let request = CompletionRequest {
            messages: vec![Message::new(Role::System, vec![ContentPart::text("rules")])],
            ..CompletionRequest::default()
        };

        let mapped = mapper
            .map_request(request, &GenerationOptions::default())
            .expect("system message must map");

        assert!(mapped.preamble.is_none());
        assert!(matches!(
            mapped.chat_history.into_iter().next(),
            Some(RigMessage::System { content }) if content == "rules"
        ));
    }
}
