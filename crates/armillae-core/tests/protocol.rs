use std::collections::BTreeMap;

use armillae_core::{
    AssistantContent, CompletionEvent, CompletionRequest, CompletionResponse, ContentKind,
    ContentPart, FinishReason, GenerationOptions, Message, OutputFormat, ProviderData,
    ProviderExtensions, Role, TextContent, TokenUsage, ToolCall, ToolCallId, ToolChoice,
    ToolDefinition, ToolResult, ToolResultContent,
};
use schemars::JsonSchema;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn round_trip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let encoded = serde_json::to_value(value).expect("protocol fixtures must serialize");
    let decoded: T = serde_json::from_value(encoded).expect("protocol fixtures must deserialize");
    assert_eq!(&decoded, value);
}

fn sample_tool_call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: ToolCallId::new(id).expect("sample ToolCall IDs are non-empty"),
        name: name.to_owned(),
        arguments,
    }
}

fn sample_response() -> CompletionResponse {
    CompletionResponse {
        id: Some("response-1".to_owned()),
        model: Some("example-model".to_owned()),
        content: vec![
            AssistantContent::Text(TextContent::new("Checking two tools.")),
            AssistantContent::ToolCall(sample_tool_call(
                "call-weather",
                "get_weather",
                json!({ "city": "上海" }),
            )),
            AssistantContent::ToolCall(sample_tool_call(
                "call-dice",
                "roll_dice",
                json!({ "sides": 20 }),
            )),
            AssistantContent::ProviderData(ProviderData {
                provider: "example".to_owned(),
                kind: "hosted_tool_status".to_owned(),
                value: json!({ "status": "complete", "future_field": [1, 2] }),
            }),
        ],
        finish_reason: Some(FinishReason::ToolCall),
        usage: Some(TokenUsage {
            input_tokens: Some(10),
            output_tokens: Some(5),
            total_tokens: Some(15),
            cached_input_tokens: Some(2),
        }),
        provider_metadata: json!({ "request_id": "request-1" }),
    }
}

#[test]
fn public_protocol_types_round_trip() {
    let response = sample_response();
    let tool_result = ToolResult {
        call_id: ToolCallId::new("call-weather").expect("fixture ToolCall ID is non-empty"),
        content: vec![
            ToolResultContent::Text {
                text: "rain".to_owned(),
            },
            ToolResultContent::Json {
                value: json!({ "temperature": 19 }),
            },
        ],
        is_error: false,
    };
    let mut extension_values = BTreeMap::new();
    extension_values.insert(
        "openai.reasoning_effort".to_owned(),
        Value::String("medium".to_owned()),
    );
    let request = CompletionRequest {
        messages: vec![
            Message::user("Use both tools."),
            response.as_assistant_message(),
            Message::tool_result(tool_result.clone()),
        ],
        tools: vec![ToolDefinition {
            name: "get_weather".to_owned(),
            description: "Get weather".to_owned(),
            input_schema: json!({ "type": "object" }),
        }],
        tool_choice: Some(ToolChoice::Specific {
            name: "get_weather".to_owned(),
        }),
        output_format: Some(OutputFormat::JsonSchema {
            name: "weather".to_owned(),
            schema: json!({ "type": "object" }),
            strict: true,
        }),
        generation: GenerationOptions {
            temperature: Some(0.2),
            max_output_tokens: Some(256),
            stop: vec!["END".to_owned()],
            seed: Some(7),
        },
        extensions: ProviderExtensions {
            values: extension_values,
        },
    };

    round_trip(&request);
    round_trip(&response);
    round_trip(&tool_result);
    round_trip(&CompletionEvent::ResponseCompleted { response });
    round_trip(&Role::Developer);
    round_trip(&ToolChoice::Auto);
    round_trip(&OutputFormat::Text);
}

#[test]
fn wire_format_is_stable_and_content_order_and_ids_are_preserved() {
    let response = sample_response();
    let encoded = serde_json::to_value(&response).expect("response fixture must serialize");

    assert_eq!(encoded["finish_reason"], "tool_call");
    assert_eq!(encoded["content"][0]["type"], "text");
    assert_eq!(encoded["content"][1]["type"], "tool_call");
    assert_eq!(encoded["content"][1]["id"], "call-weather");
    assert_eq!(encoded["content"][2]["id"], "call-dice");
    assert_eq!(encoded["content"][3]["type"], "provider_data");

    let message = response.as_assistant_message();
    assert_eq!(message.role, Role::Assistant);
    assert!(matches!(message.content[0], ContentPart::Text(_)));
    assert!(matches!(message.content[1], ContentPart::ToolCall(_)));
    assert!(matches!(message.content[2], ContentPart::ToolCall(_)));
    assert!(matches!(message.content[3], ContentPart::ProviderData(_)));
    assert_eq!(
        response
            .tool_calls()
            .map(|call| call.id.as_str())
            .collect::<Vec<_>>(),
        ["call-weather", "call-dice"]
    );
}

#[test]
fn unknown_finish_reason_and_provider_data_survive_round_trip() {
    let reason: FinishReason =
        serde_json::from_value(json!("provider_future_reason")).expect("reason must deserialize");
    assert_eq!(
        reason,
        FinishReason::Unknown("provider_future_reason".to_owned())
    );
    assert_eq!(
        serde_json::to_value(reason).expect("reason must serialize"),
        json!("provider_future_reason")
    );

    let data = ProviderData {
        provider: "future-provider".to_owned(),
        kind: "future-event".to_owned(),
        value: json!({
            "unknown_object": { "enabled": true },
            "unknown_array": [1, "two", null]
        }),
    };
    round_trip(&data);
}

#[test]
fn streaming_events_round_trip_with_stable_indices_and_one_completion() {
    let call = sample_tool_call("call-weather", "get_weather", json!({ "city": "上海" }));
    let events = vec![
        CompletionEvent::ResponseStarted {
            id: Some("response-1".to_owned()),
            model: Some("example-model".to_owned()),
        },
        CompletionEvent::ContentStarted {
            index: 0,
            kind: ContentKind::Text,
        },
        CompletionEvent::TextDelta {
            index: 0,
            text: "Checking tools.".to_owned(),
        },
        CompletionEvent::ContentCompleted { index: 0 },
        CompletionEvent::ContentStarted {
            index: 1,
            kind: ContentKind::ToolCall,
        },
        CompletionEvent::ToolCallStarted {
            index: 1,
            id: call.id.clone(),
            name: Some(call.name.clone()),
        },
        CompletionEvent::ToolCallArgumentsDelta {
            index: 1,
            fragment: "{\"city\":\"上海\"}".to_owned(),
        },
        CompletionEvent::ToolCallCompleted {
            index: 1,
            call: call.clone(),
        },
        CompletionEvent::ContentCompleted { index: 1 },
        CompletionEvent::Usage {
            usage: TokenUsage {
                total_tokens: Some(15),
                ..TokenUsage::default()
            },
        },
        CompletionEvent::ProviderEvent {
            data: ProviderData {
                provider: "example".to_owned(),
                kind: "future-event".to_owned(),
                value: json!({ "preserved": true }),
            },
        },
        CompletionEvent::ResponseCompleted {
            response: sample_response(),
        },
    ];

    for event in &events {
        round_trip(event);
    }

    let encoded = serde_json::to_value(&events).expect("event fixtures must serialize");
    assert_eq!(encoded[1]["index"], 0);
    assert_eq!(encoded[2]["index"], 0);
    assert_eq!(encoded[3]["index"], 0);
    assert_eq!(encoded[4]["index"], 1);
    assert_eq!(encoded[5]["id"], call.id.as_str());
    assert_eq!(encoded[7]["call"]["arguments"], call.arguments);
    assert_eq!(
        encoded
            .as_array()
            .expect("events serialize as an array")
            .iter()
            .filter(|event| event["type"] == "response_completed")
            .count(),
        1
    );
}

#[derive(JsonSchema)]
#[allow(dead_code)]
struct ProtocolSchema {
    message: Message,
    request: CompletionRequest,
    response: CompletionResponse,
    event: CompletionEvent,
}

#[test]
fn protocol_schema_is_valid_json_and_matches_snapshot() {
    let schema = schemars::schema_for!(ProtocolSchema);
    let actual = serde_json::to_value(schema).expect("generated schema must be valid JSON");
    let expected: Value = serde_json::from_str(include_str!("snapshots/protocol-schema.json"))
        .expect("checked-in protocol schema snapshot must be valid JSON");
    assert_eq!(actual, expected);
}

#[test]
fn tool_call_id_is_a_transparent_non_empty_string() {
    let id = ToolCallId::new("call-1").expect("fixture ToolCall ID is non-empty");
    assert_eq!(
        serde_json::to_value(&id).expect("ID must serialize"),
        json!("call-1")
    );
    assert_eq!(
        serde_json::from_value::<ToolCallId>(json!("call-1"))
            .expect("non-empty ID must deserialize"),
        id
    );
    assert!(ToolCallId::new("").is_err());
    assert!(serde_json::from_value::<ToolCallId>(json!("")).is_err());
}

#[test]
fn missing_and_null_finish_reason_are_distinct_from_unknown_values() {
    let mut missing = serde_json::to_value(sample_response()).expect("response must serialize");
    missing
        .as_object_mut()
        .expect("response serializes as an object")
        .remove("finish_reason");
    let missing: CompletionResponse =
        serde_json::from_value(missing).expect("missing finish reason must deserialize");
    assert_eq!(missing.finish_reason, None);

    let mut null = serde_json::to_value(sample_response()).expect("response must serialize");
    null["finish_reason"] = Value::Null;
    let null: CompletionResponse =
        serde_json::from_value(null).expect("null finish reason must deserialize");
    assert_eq!(null.finish_reason, None);
    assert_eq!(
        serde_json::to_value(null).expect("response must serialize")["finish_reason"],
        Value::Null
    );

    let mut unknown = sample_response();
    unknown.finish_reason = Some(FinishReason::Unknown("future_reason".to_owned()));
    assert_eq!(
        serde_json::to_value(unknown).expect("response must serialize")["finish_reason"],
        "future_reason"
    );
}
