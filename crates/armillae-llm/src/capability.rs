use armillae_core::{CompletionRequest, ContentPart, OutputFormat, Role, ToolChoice};

use crate::BridgeError;

/// Tool-choice modes supported by a Bridge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToolChoiceCapabilities {
    pub auto: bool,
    pub none: bool,
    pub required: bool,
    pub specific: bool,
}

impl ToolChoiceCapabilities {
    pub const fn all() -> Self {
        Self {
            auto: true,
            none: true,
            required: true,
            specific: true,
        }
    }

    const fn any(self) -> bool {
        self.auto || self.none || self.required || self.specific
    }
}

/// Structured-output modes supported by a Bridge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutputFormatCapabilities {
    pub json_object: bool,
    pub json_schema: bool,
}

impl OutputFormatCapabilities {
    pub const fn all() -> Self {
        Self {
            json_object: true,
            json_schema: true,
        }
    }
}

/// Provider and model capabilities used for local request preflight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BridgeCapabilities {
    pub streaming: bool,
    pub tool_calling: bool,
    pub parallel_tool_calls: bool,
    pub tool_choice: ToolChoiceCapabilities,
    pub output_format: OutputFormatCapabilities,
    pub system_message: bool,
    pub developer_message: bool,
}

impl BridgeCapabilities {
    pub const fn all() -> Self {
        Self {
            streaming: true,
            tool_calling: true,
            parallel_tool_calls: true,
            tool_choice: ToolChoiceCapabilities::all(),
            output_format: OutputFormatCapabilities::all(),
            system_message: true,
            developer_message: true,
        }
    }

    pub fn validate(&self) -> Result<(), BridgeError> {
        if !self.tool_calling && (self.parallel_tool_calls || self.tool_choice.any()) {
            return Err(BridgeError::InvalidConfiguration {
                message: "tool choice and parallel ToolCall capabilities require tool_calling"
                    .to_owned(),
            });
        }
        Ok(())
    }

    pub fn validate_request(&self, request: &CompletionRequest) -> Result<(), BridgeError> {
        self.validate()?;

        let history_uses_tools = request.messages.iter().any(|message| {
            message.role == Role::Tool
                || message.content.iter().any(|content| {
                    matches!(
                        content,
                        ContentPart::ToolCall(_) | ContentPart::ToolResult(_)
                    )
                })
        });
        if (!request.tools.is_empty() || history_uses_tools || request.tool_choice.is_some())
            && !self.tool_calling
        {
            return unsupported("tool_calling");
        }

        if let Some(choice) = &request.tool_choice {
            if request.tools.is_empty() {
                return Err(BridgeError::InvalidRequest {
                    message: "tool_choice requires at least one Tool Definition".to_owned(),
                });
            }
            match choice {
                ToolChoice::Auto => {
                    if !self.tool_choice.auto {
                        return unsupported("tool_choice.auto");
                    }
                }
                ToolChoice::None => {
                    if !self.tool_choice.none {
                        return unsupported("tool_choice.none");
                    }
                }
                ToolChoice::Required => {
                    if !self.tool_choice.required {
                        return unsupported("tool_choice.required");
                    }
                }
                ToolChoice::Specific { name } => {
                    if !request.tools.iter().any(|tool| tool.name == *name) {
                        return Err(BridgeError::InvalidRequest {
                            message: format!(
                                "specific ToolChoice references undeclared tool: {name}"
                            ),
                        });
                    }
                    if !self.tool_choice.specific {
                        return unsupported("tool_choice.specific");
                    }
                }
                _ => return unsupported("tool_choice.unknown"),
            }
        }

        if let Some(output_format) = &request.output_format {
            match output_format {
                OutputFormat::Text => {}
                OutputFormat::JsonObject => {
                    if !self.output_format.json_object {
                        return unsupported("output_format.json_object");
                    }
                }
                OutputFormat::JsonSchema { .. } => {
                    if !self.output_format.json_schema {
                        return unsupported("output_format.json_schema");
                    }
                }
                _ => return unsupported("output_format.unknown"),
            }
        }

        for message in &request.messages {
            match message.role {
                Role::System if !self.system_message => return unsupported("role.system"),
                Role::Developer if !self.developer_message => {
                    return unsupported("role.developer");
                }
                Role::System | Role::Developer | Role::User | Role::Assistant | Role::Tool => {}
                _ => return unsupported("role.unknown"),
            }
        }

        Ok(())
    }

    pub fn validate_streaming_request(
        &self,
        request: &CompletionRequest,
    ) -> Result<(), BridgeError> {
        if !self.streaming {
            return unsupported("streaming");
        }
        self.validate_request(request)
    }
}

fn unsupported<T>(capability: &str) -> Result<T, BridgeError> {
    Err(BridgeError::UnsupportedCapability {
        capability: capability.to_owned(),
    })
}
