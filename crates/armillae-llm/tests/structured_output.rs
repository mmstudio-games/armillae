use std::{
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use armillae_core::{
    AssistantContent, CompletionEvent, CompletionResponse, FinishReason, OutputFormat,
    ProviderData, StructuredOutputMode, TextContent, ToolCall, ToolCallId,
};
use armillae_llm::{
    BridgeError, CompletionStream, OutputValidation, StructuredOutputErrorKind as Kind,
};
use futures_util::{Stream, StreamExt, stream};
use serde_json::{Value, json};

fn format(schema: Value) -> OutputFormat {
    OutputFormat::Structured {
        name: "person".into(),
        schema,
        mode: StructuredOutputMode::JsonObjectValidated,
    }
}

fn schema() -> Value {
    json!({"type":"object", "properties":{
        "name":{"type":"string", "pattern":"^[A-Z]", "minLength":2},
        "age":{"type":"integer", "minimum":0, "maximum":150}
    },"required":["name","age"],"additionalProperties":false})
}

fn response(text: &str) -> CompletionResponse {
    CompletionResponse {
        id: Some("id".into()),
        model: Some("model".into()),
        content: vec![AssistantContent::Text(TextContent::new(text))],
        finish_reason: Some(FinishReason::Stop),
        usage: None,
        provider_metadata: json!({}),
    }
}

#[test]
fn validates_actual_schema_constraints_without_exposing_values() {
    let validation = OutputValidation::prepare(Some(&format(schema()))).unwrap();
    validation
        .validate(&response(r#"{"name":"Alice","age":18}"#))
        .unwrap();
    for (text, kind) in [
        (r#"{"name":"Alice","age":-1}"#, Kind::SchemaMismatch),
        (
            r#"{"name":"Alice","age":"secret-output"}"#,
            Kind::SchemaMismatch,
        ),
        (r#"{"name":"alice","age":18}"#, Kind::SchemaMismatch),
        (r#"{"name":"A","age":18}"#, Kind::SchemaMismatch),
        (r#"{"name":"Alice","age":151}"#, Kind::SchemaMismatch),
        (r#"{"name":"Alice"}"#, Kind::SchemaMismatch),
        (
            r#"{"name":"Alice","age":18,"extra":true}"#,
            Kind::SchemaMismatch,
        ),
        (r#"{"name":"Alice","age":1.5}"#, Kind::SchemaMismatch),
        ("```json\n{}\n```", Kind::InvalidJson),
        ("{} {}", Kind::InvalidJson),
        ("[]", Kind::NotObject),
        (" ", Kind::MissingText),
    ] {
        let error = validation.validate(&response(text)).unwrap_err();
        assert_eq!(error, BridgeError::StructuredOutput { kind });
        assert!(!format!("{error:?} {error}").contains("secret-output"));
    }
    assert!(!format!("{validation:?}").contains("minimum"));
}

#[test]
fn invalid_schemas_and_external_references_fail_without_io() {
    for schema in [
        json!([]),
        json!({"type":"secret-schema"}),
        json!({"$schema":"https://invalid.test/custom", "type":"object"}),
        json!({"$ref":"https://127.0.0.1/private"}),
        json!({"$ref":"file:///etc/passwd"}),
        json!({"$ref":"#/$defs/missing"}),
        json!({"type":"object","properties":{"age":{"minimum":"secret-schema"}}}),
    ] {
        let error = OutputValidation::prepare(Some(&format(schema))).unwrap_err();
        assert_eq!(error, BridgeError::InvalidOutputSchema);
        assert!(!format!("{error:?} {error}").contains("secret-schema"));
    }
    let mut empty_name = format(schema());
    if let OutputFormat::Structured { name, .. } = &mut empty_name {
        name.clear();
    }
    assert!(matches!(
        OutputValidation::prepare(Some(&empty_name)),
        Err(BridgeError::InvalidOutputSchema)
    ));
}

#[test]
fn supports_local_refs_combinators_and_explicit_dialects() {
    for dialect in [
        "http://json-schema.org/draft-04/schema#",
        "http://json-schema.org/draft-06/schema#",
        "http://json-schema.org/draft-07/schema#",
        "https://json-schema.org/draft/2019-09/schema",
        "https://json-schema.org/draft/2020-12/schema",
    ] {
        let validation = OutputValidation::prepare(Some(&format(json!({
            "$schema":dialect, "type":"object", "definitions":{"age":{"type":"integer","minimum":0}},
            "properties":{"age":{"$ref":"#/definitions/age"}}, "required":["age"],
            "allOf":[{"properties":{"age":{"maximum":150}}}]
        })))).unwrap();
        validation.validate(&response(r#"{"age":18}"#)).unwrap();
        assert!(validation.validate(&response(r#"{"age":-1}"#)).is_err());
    }
}

#[test]
fn preserves_content_and_rejects_incomplete_or_tool_results() {
    let validation = OutputValidation::prepare(Some(&format(schema()))).unwrap();
    let mut value = response(r#"{"name":"Alice","age":18}"#);
    value.content = vec![
        AssistantContent::Text(TextContent::new(r#"{"name":"Al"#)),
        AssistantContent::ProviderData(ProviderData {
            provider: "test".into(),
            kind: "reasoning".into(),
            value: json!({"id":"opaque"}),
        }),
        AssistantContent::Text(TextContent::new(r#"ice","age":18}"#)),
    ];
    let original = value.clone();
    validation.validate(&value).unwrap();
    assert_eq!(value, original);
    for reason in [
        FinishReason::Length,
        FinishReason::ContentFilter,
        FinishReason::Unknown("future".into()),
    ] {
        value.finish_reason = Some(reason);
        assert_eq!(
            validation.validate(&value),
            Err(BridgeError::StructuredOutput {
                kind: Kind::Incomplete
            })
        );
    }
    value.finish_reason = None;
    validation.validate(&value).unwrap();
    value.content.push(AssistantContent::ToolCall(ToolCall {
        id: ToolCallId::new("call").unwrap(),
        name: "lookup".into(),
        arguments: json!({}),
    }));
    assert_eq!(
        validation.validate(&value),
        Err(BridgeError::StructuredOutput {
            kind: Kind::ToolCall
        })
    );
}

#[test]
fn streaming_preserves_preview_and_has_one_terminal_outcome() {
    futures_executor::block_on(async {
        for (text, succeeds) in [
            (r#"{"name":"Alice","age":18}"#, true),
            (r#"{"name":"Alice","age":-1}"#, false),
        ] {
            let preview = CompletionEvent::TextDelta {
                index: 3,
                text: text.into(),
            };
            let terminal = CompletionEvent::ResponseCompleted {
                response: response(text),
            };
            let input = vec![
                Ok(preview.clone()),
                Ok(terminal.clone()),
                Ok(terminal.clone()),
            ];
            let validation = OutputValidation::prepare(Some(&format(schema()))).unwrap();
            let events = validation
                .stream(Box::pin(stream::iter(input)), "test")
                .collect::<Vec<_>>()
                .await;
            assert_eq!(events.len(), 2);
            assert_eq!(events[0], Ok(preview));
            if succeeds {
                assert_eq!(events[1], Ok(terminal));
            } else {
                assert_eq!(
                    events[1],
                    Err(BridgeError::StructuredOutput {
                        kind: Kind::SchemaMismatch
                    })
                );
            }
        }
        let validation = OutputValidation::prepare(Some(&format(schema()))).unwrap();
        let events = validation
            .stream(Box::pin(stream::empty()), "test")
            .collect::<Vec<_>>()
            .await;
        assert!(matches!(
            &events[..],
            [Err(BridgeError::StreamInterrupted { .. })]
        ));
        let validation = OutputValidation::prepare(Some(&format(schema()))).unwrap();
        let events = validation
            .stream(
                Box::pin(stream::iter([Err(BridgeError::Cancelled)])),
                "test",
            )
            .collect::<Vec<_>>()
            .await;
        assert_eq!(events, vec![Err(BridgeError::Cancelled)]);
    });
}

struct DropProbe(Arc<AtomicBool>);
impl Stream for DropProbe {
    type Item = Result<CompletionEvent, BridgeError>;
    fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Poll::Pending
    }
}
impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[test]
fn dropping_structured_stream_drops_transport_without_polling_or_drain() {
    let dropped = Arc::new(AtomicBool::new(false));
    let inner: CompletionStream = Box::pin(DropProbe(dropped.clone()));
    let validation = OutputValidation::prepare(Some(&format(schema()))).unwrap();
    drop(validation.stream(inner, "test"));
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
fn wire_support_does_not_imply_either_validation_mode() {
    use armillae_llm::{BridgeCapabilities, OutputFormatCapabilities};
    let mut capabilities = BridgeCapabilities {
        reasoning: armillae_llm::ReasoningCapabilities::NONE,
        streaming: true,
        output_format: OutputFormatCapabilities {
            json_object: true,
            json_schema: true,
            ..Default::default()
        },
        ..Default::default()
    };
    for mode in [
        StructuredOutputMode::NativeStrict,
        StructuredOutputMode::JsonObjectValidated,
    ] {
        let request = armillae_core::CompletionRequest {
            output_format: Some(OutputFormat::Structured {
                name: "person".into(),
                schema: schema(),
                mode,
            }),
            ..Default::default()
        };
        assert!(matches!(
            capabilities.validate_request(&request),
            Err(BridgeError::UnsupportedCapability { .. })
        ));
        assert!(matches!(
            capabilities.validate_streaming_request(&request),
            Err(BridgeError::UnsupportedCapability { .. })
        ));
    }
    capabilities.output_format.native_strict_schema = true;
    capabilities.output_format.json_schema = false;
    assert!(matches!(
        capabilities.validate(),
        Err(BridgeError::InvalidConfiguration { .. })
    ));
    capabilities.output_format = OutputFormatCapabilities {
        json_object_schema_validation: true,
        ..Default::default()
    };
    assert!(matches!(
        capabilities.validate(),
        Err(BridgeError::InvalidConfiguration { .. })
    ));
}

#[cfg(feature = "mock")]
#[test]
fn mock_uses_same_complete_and_stream_validation() {
    use armillae_llm::{LlmBridge, MockBridge, MockResponse};
    futures_executor::block_on(async {
        let request = armillae_core::CompletionRequest {
            output_format: Some(format(schema())),
            ..Default::default()
        };
        let invalid = response(r#"{"age":-1}"#);
        let bridge = MockBridge::scripted([
            MockResponse::Completion(invalid.clone()),
            MockResponse::stream([Ok(CompletionEvent::ResponseCompleted { response: invalid })]),
        ]);
        assert!(matches!(
            bridge.complete(request.clone()).await,
            Err(BridgeError::StructuredOutput { .. })
        ));
        let events = bridge
            .stream(request)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await;
        assert!(matches!(
            &events[..],
            [Err(BridgeError::StructuredOutput { .. })]
        ));
    });
}
