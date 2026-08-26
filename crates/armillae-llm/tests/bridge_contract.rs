use std::{sync::Arc, time::Duration};

use armillae_core::{
    CompletionRequest, CompletionResponse, ContentPart, FinishReason, Message, OutputFormat, Role,
    ToolCall, ToolCallId, ToolChoice, ToolDefinition, ToolResult,
};
use armillae_llm::{
    BoxFuture, BridgeCapabilities, BridgeError, CompletionStream, ErrorMetadata, LlmBridge,
    OutputFormatCapabilities, ProjectionReport, ToolChoiceCapabilities,
};
use futures_executor::block_on;
use futures_util::stream;
use serde_json::{Value, json};

struct ContractBridge;

impl LlmBridge for ContractBridge {
    fn capabilities(&self) -> BridgeCapabilities {
        BridgeCapabilities::all()
    }

    fn project(&self, request: &CompletionRequest) -> Result<ProjectionReport, BridgeError> {
        self.capabilities().validate_request(request)?;
        Ok(ProjectionReport::exact("contract"))
    }

    fn complete<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionResponse, BridgeError>> {
        Box::pin(async move {
            self.capabilities().validate_request(&request)?;
            Ok(empty_response())
        })
    }

    fn stream<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionStream, BridgeError>> {
        Box::pin(async move {
            self.capabilities().validate_streaming_request(&request)?;
            Ok(Box::pin(stream::empty()) as CompletionStream)
        })
    }
}

#[test]
fn llm_bridge_is_object_safe_and_runtime_independent() {
    let bridge: Arc<dyn LlmBridge> = Arc::new(ContractBridge);

    let response = block_on(bridge.complete(CompletionRequest::default()))
        .expect("the contract bridge accepts a default request");
    assert_eq!(response, empty_response());

    let stream = block_on(bridge.stream(CompletionRequest::default()))
        .expect("the contract bridge accepts a default streaming request");
    drop(stream);
}

#[test]
fn validates_each_tool_choice_capability_independently() {
    let cases = [
        (
            ToolChoice::Auto,
            ToolChoiceCapabilities {
                auto: false,
                ..ToolChoiceCapabilities::all()
            },
            "tool_choice.auto",
        ),
        (
            ToolChoice::None,
            ToolChoiceCapabilities {
                none: false,
                ..ToolChoiceCapabilities::all()
            },
            "tool_choice.none",
        ),
        (
            ToolChoice::Required,
            ToolChoiceCapabilities {
                required: false,
                ..ToolChoiceCapabilities::all()
            },
            "tool_choice.required",
        ),
        (
            ToolChoice::Specific {
                name: "lookup".to_owned(),
            },
            ToolChoiceCapabilities {
                specific: false,
                ..ToolChoiceCapabilities::all()
            },
            "tool_choice.specific",
        ),
    ];

    for (tool_choice, tool_choice_capabilities, expected_capability) in cases {
        let request = request_with_tool_choice(tool_choice);
        let capabilities = BridgeCapabilities {
            tool_calling: true,
            tool_choice: tool_choice_capabilities,
            ..BridgeCapabilities::default()
        };

        assert_unsupported(capabilities.validate_request(&request), expected_capability);
    }
}

#[test]
fn validates_tool_choice_references_before_provider_call() {
    let mut request = request_with_tool_choice(ToolChoice::Specific {
        name: "missing".to_owned(),
    });

    assert!(matches!(
        tool_capabilities().validate_request(&request),
        Err(BridgeError::InvalidRequest { message })
            if message == "specific ToolChoice references undeclared tool: missing"
    ));

    request.tools.clear();
    request.tool_choice = Some(ToolChoice::Auto);
    assert!(matches!(
        tool_capabilities().validate_request(&request),
        Err(BridgeError::InvalidRequest { message })
            if message == "tool_choice requires at least one Tool Definition"
    ));
}

#[test]
fn rejects_tools_and_tool_history_when_tool_calling_is_unsupported() {
    let capabilities = BridgeCapabilities::default();

    let mut with_definition = CompletionRequest::default();
    with_definition.tools.push(tool_definition());
    assert_unsupported(
        capabilities.validate_request(&with_definition),
        "tool_calling",
    );

    let with_tool_call = CompletionRequest {
        messages: vec![Message::assistant(vec![ContentPart::ToolCall(ToolCall {
            id: tool_call_id("call-1"),
            name: "lookup".to_owned(),
            arguments: json!({"query": "armillae"}),
        })])],
        ..CompletionRequest::default()
    };
    assert_unsupported(
        capabilities.validate_request(&with_tool_call),
        "tool_calling",
    );

    let with_tool_result = CompletionRequest {
        messages: vec![Message::tool_result(ToolResult {
            call_id: tool_call_id("call-1"),
            content: Vec::new(),
            is_error: false,
        })],
        ..CompletionRequest::default()
    };
    assert_unsupported(
        capabilities.validate_request(&with_tool_result),
        "tool_calling",
    );
}

#[test]
fn validates_structured_output_modes_independently() {
    let json_object_request = CompletionRequest {
        output_format: Some(OutputFormat::JsonObject),
        ..CompletionRequest::default()
    };
    let json_schema_request = CompletionRequest {
        output_format: Some(OutputFormat::JsonSchema {
            name: "answer".to_owned(),
            schema: json!({"type": "object"}),
            strict: true,
        }),
        ..CompletionRequest::default()
    };

    let only_json_schema = BridgeCapabilities {
        output_format: OutputFormatCapabilities {
            json_object: false,
            json_schema: true,
        },
        ..BridgeCapabilities::default()
    };
    assert_unsupported(
        only_json_schema.validate_request(&json_object_request),
        "output_format.json_object",
    );
    assert!(
        only_json_schema
            .validate_request(&json_schema_request)
            .is_ok()
    );

    let only_json_object = BridgeCapabilities {
        output_format: OutputFormatCapabilities {
            json_object: true,
            json_schema: false,
        },
        ..BridgeCapabilities::default()
    };
    assert!(
        only_json_object
            .validate_request(&json_object_request)
            .is_ok()
    );
    assert_unsupported(
        only_json_object.validate_request(&json_schema_request),
        "output_format.json_schema",
    );
}

#[test]
fn validates_system_and_developer_roles_independently() {
    let system_request = request_with_role(Role::System);
    let developer_request = request_with_role(Role::Developer);

    let only_developer = BridgeCapabilities {
        developer_message: true,
        ..BridgeCapabilities::default()
    };
    assert_unsupported(
        only_developer.validate_request(&system_request),
        "role.system",
    );
    assert!(only_developer.validate_request(&developer_request).is_ok());

    let only_system = BridgeCapabilities {
        system_message: true,
        ..BridgeCapabilities::default()
    };
    assert!(only_system.validate_request(&system_request).is_ok());
    assert_unsupported(
        only_system.validate_request(&developer_request),
        "role.developer",
    );
}

#[test]
fn streaming_is_rejected_locally_when_unsupported() {
    let capabilities = BridgeCapabilities::default();

    assert_unsupported(
        capabilities.validate_streaming_request(&CompletionRequest::default()),
        "streaming",
    );
}

#[test]
fn invalid_tool_capability_combinations_are_configuration_errors() {
    let parallel_without_tools = BridgeCapabilities {
        parallel_tool_calls: true,
        ..BridgeCapabilities::default()
    };
    assert!(matches!(
        parallel_without_tools.validate(),
        Err(BridgeError::InvalidConfiguration { .. })
    ));

    let choice_without_tools = BridgeCapabilities {
        tool_choice: ToolChoiceCapabilities {
            auto: true,
            ..ToolChoiceCapabilities::default()
        },
        ..BridgeCapabilities::default()
    };
    assert!(matches!(
        choice_without_tools.validate(),
        Err(BridgeError::InvalidConfiguration { .. })
    ));
}

#[test]
fn error_metadata_preserves_only_normalized_facts() {
    let metadata = ErrorMetadata::new("provider")
        .with_http_status(429)
        .with_request_id("request-1");

    assert_eq!(metadata.provider, "provider");
    assert_eq!(metadata.http_status, Some(429));
    assert_eq!(metadata.request_id.as_deref(), Some("request-1"));
}

#[test]
fn bridge_error_exposes_the_complete_normalized_classification() {
    let metadata = ErrorMetadata::new("provider-secret-marker")
        .with_http_status(503)
        .with_request_id("request-secret-marker");
    let errors = [
        BridgeError::InvalidConfiguration {
            message: "bad configuration".to_owned(),
        },
        BridgeError::UnsupportedCapability {
            capability: "streaming".to_owned(),
        },
        BridgeError::InvalidRequest {
            message: "bad request".to_owned(),
        },
        BridgeError::Authentication {
            metadata: metadata.clone(),
        },
        BridgeError::PermissionDenied {
            metadata: metadata.clone(),
        },
        BridgeError::RateLimited {
            retry_after: Some(Duration::from_secs(1)),
            metadata: metadata.clone(),
        },
        BridgeError::Timeout {
            metadata: metadata.clone(),
        },
        BridgeError::Cancelled,
        BridgeError::Transport {
            retryable: true,
            metadata: metadata.clone(),
        },
        BridgeError::ProviderRejected {
            code: Some("invalid_request".to_owned()),
            message: "sanitized rejection".to_owned(),
            metadata: metadata.clone(),
        },
        BridgeError::InvalidProviderResponse {
            message: "sanitized response error".to_owned(),
            metadata: metadata.clone(),
        },
        BridgeError::StreamInterrupted { metadata },
    ];

    assert_eq!(errors.len(), 12);
    for error in errors {
        assert!(error.to_string().is_ascii());
        assert!(!error.to_string().contains("request-secret-marker"));
        assert!(!error.to_string().contains("provider-secret-marker"));
    }
}

fn empty_response() -> CompletionResponse {
    CompletionResponse {
        id: None,
        model: None,
        content: Vec::new(),
        finish_reason: Some(FinishReason::Stop),
        usage: None,
        provider_metadata: Value::Null,
    }
}

fn tool_call_id(value: &str) -> ToolCallId {
    ToolCallId::new(value).expect("fixture ToolCall IDs are non-empty")
}

fn tool_capabilities() -> BridgeCapabilities {
    BridgeCapabilities {
        tool_calling: true,
        tool_choice: ToolChoiceCapabilities::all(),
        ..BridgeCapabilities::default()
    }
}

fn tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "lookup".to_owned(),
        description: "Look up a value".to_owned(),
        input_schema: json!({"type": "object"}),
    }
}

fn request_with_tool_choice(tool_choice: ToolChoice) -> CompletionRequest {
    CompletionRequest {
        tools: vec![tool_definition()],
        tool_choice: Some(tool_choice),
        ..CompletionRequest::default()
    }
}

fn request_with_role(role: Role) -> CompletionRequest {
    CompletionRequest {
        messages: vec![Message::new(role, vec![ContentPart::text("message")])],
        ..CompletionRequest::default()
    }
}

fn assert_unsupported(result: Result<(), BridgeError>, expected_capability: &str) {
    assert!(matches!(
        result,
        Err(BridgeError::UnsupportedCapability { capability })
            if capability == expected_capability
    ));
}
