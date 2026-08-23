#![cfg(feature = "mock")]

use std::{sync::Arc, thread};

use armillae_core::{
    AssistantContent, CompletionEvent, CompletionRequest, CompletionResponse, ContentKind,
    ContentPart, FinishReason, Message, TextContent, ToolCall, ToolCallId, ToolDefinition,
};
use armillae_llm::{
    BridgeCapabilities, BridgeError, ErrorMetadata, LlmBridge, MockBridge, MockResponse,
    mock::contract::{
        BridgeContractError, validate_stream_events, verify_completion, verify_stream,
    },
};
use futures_executor::block_on;
use futures_util::StreamExt;
use serde_json::json;

#[test]
fn fixed_completion_repeats_and_records_requests() {
    let bridge = MockBridge::fixed(MockResponse::text("fixed response"));
    let first_request = CompletionRequest {
        messages: vec![Message::user("first request")],
        ..CompletionRequest::default()
    };
    let second_request = CompletionRequest {
        messages: vec![Message::user("second request")],
        ..CompletionRequest::default()
    };

    let first = block_on(bridge.complete(first_request.clone()))
        .expect("the fixed completion is available on the first call");
    let second = block_on(bridge.complete(second_request.clone()))
        .expect("the fixed completion is repeated on the second call");

    assert_eq!(first, second);
    assert_eq!(response_text(&first), Some("fixed response"));
    assert_eq!(
        bridge.requests().expect("the request lock is available"),
        vec![first_request, second_request]
    );
    assert_eq!(
        bridge
            .remaining_scripted_responses()
            .expect("the plan lock is available"),
        None
    );
}

#[test]
fn scripted_completions_are_consumed_in_order_and_exhaust() {
    let bridge = MockBridge::scripted([
        MockResponse::text("first"),
        MockResponse::tool_call(
            tool_call_id("call-1"),
            "lookup",
            json!({"query": "armillae"}),
        ),
    ]);

    let first = block_on(bridge.complete(CompletionRequest::default()))
        .expect("the first scripted response is available");
    assert_eq!(response_text(&first), Some("first"));
    assert_eq!(
        bridge
            .remaining_scripted_responses()
            .expect("the script lock is available"),
        Some(1)
    );

    let second = block_on(bridge.complete(CompletionRequest::default()))
        .expect("the second scripted response is available");
    assert!(matches!(
        second.content.as_slice(),
        [AssistantContent::ToolCall(ToolCall { id, name, arguments })]
            if id.as_str() == "call-1"
                && name == "lookup"
                && arguments == &json!({"query": "armillae"})
    ));

    assert!(matches!(
        block_on(bridge.complete(CompletionRequest::default())),
        Err(BridgeError::InvalidRequest { message }) if message == "MockBridge script is exhausted"
    ));
}

#[test]
fn capability_rejection_records_request_without_consuming_script() {
    let bridge = MockBridge::scripted([MockResponse::text("still available")])
        .with_capabilities(BridgeCapabilities::default())
        .expect("the default capability set is internally valid");
    let rejected = CompletionRequest {
        tools: vec![ToolDefinition {
            name: "lookup".to_owned(),
            description: "Look up a value".to_owned(),
            input_schema: json!({"type": "object"}),
        }],
        ..CompletionRequest::default()
    };

    assert!(matches!(
        block_on(bridge.complete(rejected.clone())),
        Err(BridgeError::UnsupportedCapability { capability }) if capability == "tool_calling"
    ));
    assert_eq!(
        bridge
            .remaining_scripted_responses()
            .expect("the script lock is available"),
        Some(1)
    );

    let response = block_on(bridge.complete(CompletionRequest::default()))
        .expect("the rejected request did not consume the scripted response");
    assert_eq!(response_text(&response), Some("still available"));
    assert_eq!(
        bridge.requests().expect("the request lock is available"),
        vec![rejected, CompletionRequest::default()]
    );
}

#[test]
fn text_stream_preserves_chunks_and_has_one_terminal_response() {
    let bridge = MockBridge::fixed(MockResponse::text_stream(["你", "好", "，Armillae"]));
    let events = collect_stream(&bridge);

    let deltas: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            CompletionEvent::TextDelta { index: 0, text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(deltas, ["你", "好", "，Armillae"]);

    let completions: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            CompletionEvent::ResponseCompleted { response } => Some(response),
            _ => None,
        })
        .collect();
    assert_eq!(completions.len(), 1);
    assert_eq!(response_text(completions[0]), Some("你好，Armillae"));
    assert!(matches!(
        events.last(),
        Some(CompletionEvent::ResponseCompleted { .. })
    ));
}

#[test]
fn tool_call_stream_preserves_arbitrary_argument_fragments() {
    let response = MockResponse::tool_call_stream(
        tool_call_id("call-1"),
        "lookup",
        ["{\"query", "\":\"Ar", "millae\"}"],
    )
    .expect("the argument fragments form valid JSON");
    let bridge = MockBridge::fixed(response);
    let events = collect_stream(&bridge);

    let fragments: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            CompletionEvent::ToolCallArgumentsDelta { index: 0, fragment } => {
                Some(fragment.as_str())
            }
            _ => None,
        })
        .collect();
    assert_eq!(fragments, ["{\"query", "\":\"Ar", "millae\"}"]);
    assert!(events.iter().any(|event| matches!(
        event,
        CompletionEvent::ToolCallCompleted {
            index: 0,
            call: ToolCall { id, name, arguments },
        } if id.as_str() == "call-1"
            && name == "lookup"
            && arguments == &json!({"query": "Armillae"})
    )));
}

#[test]
fn invalid_tool_call_fragments_are_configuration_errors() {
    assert!(matches!(
        MockResponse::tool_call_stream(tool_call_id("call-1"), "lookup", ["{invalid"]),
        Err(BridgeError::InvalidConfiguration { .. })
    ));
}

#[test]
fn provider_errors_and_mid_stream_interruptions_can_be_injected() {
    let metadata = ErrorMetadata::new("mock-provider").with_request_id("request-1");
    let error_bridge = MockBridge::fixed(MockResponse::error(BridgeError::RateLimited {
        retry_after: None,
        metadata: metadata.clone(),
    }));
    assert!(matches!(
        block_on(error_bridge.complete(CompletionRequest::default())),
        Err(BridgeError::RateLimited { .. })
    ));

    let stream_bridge = MockBridge::fixed(MockResponse::interrupted_stream(
        [CompletionEvent::ResponseStarted {
            id: None,
            model: None,
        }],
        metadata,
    ));
    let items = block_on(async {
        stream_bridge
            .stream(CompletionRequest::default())
            .await
            .expect("the stream starts successfully")
            .collect::<Vec<_>>()
            .await
    });
    assert!(matches!(
        items.as_slice(),
        [
            Ok(CompletionEvent::ResponseStarted { .. }),
            Err(BridgeError::StreamInterrupted { .. })
        ]
    ));
}

#[test]
fn scripted_queue_is_safe_for_concurrent_callers() {
    let bridge = Arc::new(MockBridge::scripted([
        MockResponse::text("one"),
        MockResponse::text("two"),
        MockResponse::text("three"),
        MockResponse::text("four"),
    ]));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let bridge = Arc::clone(&bridge);
            thread::spawn(move || {
                block_on(bridge.complete(CompletionRequest::default()))
                    .expect("each concurrent caller receives one scripted response")
            })
        })
        .collect();
    let mut texts: Vec<String> = handles
        .into_iter()
        .map(|handle| {
            let response = handle
                .join()
                .expect("the concurrent MockBridge caller does not panic");
            response_text(&response)
                .expect("each scripted response contains text")
                .to_owned()
        })
        .collect();
    texts.sort();

    assert_eq!(texts, ["four", "one", "three", "two"]);
    assert_eq!(
        bridge
            .remaining_scripted_responses()
            .expect("the script lock is available"),
        Some(0)
    );
}

#[test]
fn debug_output_omits_request_response_and_error_content() {
    let bridge = MockBridge::scripted([MockResponse::text("response-secret-marker")]);
    block_on(bridge.complete(CompletionRequest {
        messages: vec![Message::user("request-secret-marker")],
        ..CompletionRequest::default()
    }))
    .expect("the scripted response is available");

    let bridge_debug = format!("{bridge:?}");
    assert!(!bridge_debug.contains("request-secret-marker"));
    assert!(!bridge_debug.contains("response-secret-marker"));

    let response_debug = format!(
        "{:?}",
        MockResponse::error(BridgeError::InvalidRequest {
            message: "error-secret-marker".to_owned(),
        })
    );
    assert!(!response_debug.contains("error-secret-marker"));
}

#[test]
fn complete_and_stream_reject_mismatched_script_items() {
    let complete_bridge = MockBridge::fixed(MockResponse::text_stream(["chunk"]));
    assert!(matches!(
        block_on(complete_bridge.complete(CompletionRequest::default())),
        Err(BridgeError::InvalidRequest { .. })
    ));

    let stream_bridge = MockBridge::fixed(MockResponse::text("completion"));
    assert!(matches!(
        block_on(stream_bridge.stream(CompletionRequest::default())),
        Err(BridgeError::InvalidRequest { .. })
    ));
}

#[test]
fn take_requests_clears_the_recording() {
    let bridge = MockBridge::fixed(MockResponse::text("response"));
    block_on(bridge.complete(CompletionRequest {
        messages: vec![Message::new(
            armillae_core::Role::User,
            vec![ContentPart::text("request")],
        )],
        ..CompletionRequest::default()
    }))
    .expect("the fixed response is available");

    assert_eq!(
        bridge
            .take_requests()
            .expect("the request lock is available")
            .len(),
        1
    );
    assert!(
        bridge
            .requests()
            .expect("the request lock is available")
            .is_empty()
    );
}

#[test]
fn shared_contract_verifies_mock_completion_and_stream_fixtures() {
    let expected = text_completion("contract response");
    let completion_bridge = MockBridge::fixed(MockResponse::completion(expected.clone()));
    block_on(verify_completion(
        &completion_bridge,
        CompletionRequest::default(),
        &expected,
    ))
    .expect("the deterministic completion satisfies the shared contract");

    let stream_bridge = MockBridge::fixed(MockResponse::text_stream(["contract ", "response"]));
    let events = block_on(verify_stream(
        &stream_bridge,
        CompletionRequest::default(),
        &expected,
    ))
    .expect("the deterministic stream satisfies the shared contract");
    assert!(matches!(
        events.last(),
        Some(CompletionEvent::ResponseCompleted { .. })
    ));
}

#[test]
fn shared_contract_rejects_invalid_event_order_and_content() {
    let missing_start = [CompletionEvent::ResponseCompleted {
        response: text_completion("response"),
    }];
    assert!(matches!(
        validate_stream_events(&missing_start),
        Err(BridgeContractError::EventSequence { .. })
    ));

    let mismatched_text = [
        CompletionEvent::ResponseStarted {
            id: None,
            model: None,
        },
        CompletionEvent::ContentStarted {
            index: 0,
            kind: ContentKind::Text,
        },
        CompletionEvent::TextDelta {
            index: 0,
            text: "delta".to_owned(),
        },
        CompletionEvent::ContentCompleted { index: 0 },
        CompletionEvent::ResponseCompleted {
            response: text_completion("different final text"),
        },
    ];
    assert!(matches!(
        validate_stream_events(&mismatched_text),
        Err(BridgeContractError::EventSequence { .. })
    ));
}

#[test]
fn shared_contract_reassembles_tool_call_fragments() {
    let expected_call = ToolCall {
        id: tool_call_id("call-1"),
        name: "lookup".to_owned(),
        arguments: json!({"query": "Armillae"}),
    };
    let expected = CompletionResponse {
        id: None,
        model: None,
        content: vec![AssistantContent::ToolCall(expected_call)],
        finish_reason: Some(FinishReason::ToolCall),
        usage: None,
        provider_metadata: serde_json::Value::Null,
    };
    let bridge = MockBridge::fixed(
        MockResponse::tool_call_stream(
            tool_call_id("call-1"),
            "lookup",
            ["{\"query\":", "\"Ar", "millae\"}"],
        )
        .expect("the fragments form valid JSON"),
    );

    block_on(verify_stream(
        &bridge,
        CompletionRequest::default(),
        &expected,
    ))
    .expect("the shared contract reassembles the ToolCall arguments");
}

#[test]
fn shared_contract_preserves_injected_stream_failure_classification() {
    let bridge = MockBridge::fixed(MockResponse::interrupted_stream(
        [CompletionEvent::ResponseStarted {
            id: None,
            model: None,
        }],
        ErrorMetadata::new("mock-provider"),
    ));
    let expected = text_completion("unreachable");

    assert!(matches!(
        block_on(verify_stream(
            &bridge,
            CompletionRequest::default(),
            &expected
        )),
        Err(BridgeContractError::BridgeFailure {
            operation: "stream item",
            error: BridgeError::StreamInterrupted { .. },
        })
    ));
}

fn collect_stream(bridge: &MockBridge) -> Vec<CompletionEvent> {
    block_on(async {
        bridge
            .stream(CompletionRequest::default())
            .await
            .expect("the Mock stream starts successfully")
            .map(|item| item.expect("the convenience stream contains no injected error"))
            .collect()
            .await
    })
}

fn response_text(response: &armillae_core::CompletionResponse) -> Option<&str> {
    match response.content.as_slice() {
        [AssistantContent::Text(text)] => Some(text.text.as_str()),
        _ => None,
    }
}

fn text_completion(text: &str) -> CompletionResponse {
    CompletionResponse {
        id: None,
        model: None,
        content: vec![AssistantContent::Text(TextContent::new(text))],
        finish_reason: Some(FinishReason::Stop),
        usage: None,
        provider_metadata: serde_json::Value::Null,
    }
}

fn tool_call_id(value: &str) -> ToolCallId {
    ToolCallId::new(value).expect("fixture ToolCall IDs are non-empty")
}
