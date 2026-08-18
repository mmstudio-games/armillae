use armillae_core::{
    CompletionRequest as ArmillaeCompletionRequest, GenerationOptions, OutputFormat,
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

#[derive(Clone, Debug, Default)]
pub(crate) struct OpenAiRequestMapper {
    provider_options: Map<String, Value>,
}

impl OpenAiRequestMapper {
    pub(crate) fn new(provider_options: Value) -> Result<Self, BridgeError> {
        let Value::Object(provider_options) = provider_options else {
            return invalid_configuration("OpenAI provider_options must be a JSON object");
        };
        validate_provider_options(&provider_options)?;
        Ok(Self { provider_options })
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
        merge_request_extensions(&mut additional_params, parts.extensions.values)?;

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

fn validate_provider_options(options: &Map<String, Value>) -> Result<(), BridgeError> {
    for (name, value) in options {
        if is_standard_field(name) {
            return invalid_configuration(format!(
                "OpenAI provider_options cannot override standard field: {name}"
            ));
        }
        validate_openai_option(name, value, true)?;
    }
    Ok(())
}

fn merge_request_extensions(
    target: &mut Map<String, Value>,
    extensions: impl IntoIterator<Item = (String, Value)>,
) -> Result<(), BridgeError> {
    for (key, value) in extensions {
        let Some(name) = key.strip_prefix("openai.") else {
            return invalid_request(format!(
                "OpenAI Adapter does not accept extension namespace: {key}"
            ));
        };
        if is_standard_field(name) {
            return invalid_request(format!(
                "OpenAI extension cannot override standard field: {name}"
            ));
        }
        validate_openai_option(name, &value, false)?;
        target.insert(name.to_owned(), value);
    }
    Ok(())
}

fn validate_openai_option(
    name: &str,
    value: &Value,
    configuration: bool,
) -> Result<(), BridgeError> {
    match name {
        "reasoning_effort" if value.as_str().is_some_and(|value| !value.is_empty()) => Ok(()),
        "reasoning_effort" => option_error(
            configuration,
            "OpenAI reasoning_effort must be a non-empty string",
        ),
        _ => option_error(configuration, format!("unknown OpenAI option: {name}")),
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
        ProviderExtensions, Role, ToolResult, ToolResultContent,
    };
    use armillae_llm::BridgeError;
    use rig_core::{message::Message as RigMessage, providers::openai};
    use serde_json::{Value, json};

    use super::{OpenAiRequestMapper, RigRequestMapper};

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
                    call_id: "call-1".to_owned(),
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
