use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
};

use armillae_core::{
    AssistantContent, CompletionEvent, CompletionRequest, Message, OutputFormat,
    StructuredOutputMode,
};
use armillae_llm::{BridgeError, LlmBridge, StructuredOutputErrorKind};
use bytes::Bytes;
use futures::{Stream, StreamExt};
use rig_core::{
    http_client::{
        self, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
    },
    test_utils::RecordingHttpClient,
    wasm_compat::WasmCompatSend,
};
use serde_json::{Value, json};

const PROVIDERS: [&str; 7] = [
    "openai",
    "openai-compatible",
    "deepseek",
    "minimax",
    "moonshot",
    "anthropic",
    "ollama",
];
const MODES: [StructuredOutputMode; 2] = [
    StructuredOutputMode::NativeStrict,
    StructuredOutputMode::JsonObjectValidated,
];
const VALID: &str = r#"{"answer":"你好"}"#;

#[derive(Clone, Debug, Default)]
pub(super) struct Client {
    unary: RecordingHttpClient,
    bytes: Vec<u8>,
    requests: Arc<Mutex<Vec<Value>>>,
    raw_requests: Arc<Mutex<Vec<Bytes>>>,
    dropped: Arc<AtomicBool>,
    pending: bool,
    transport_error: bool,
}

struct TrackedStream {
    inner: http_client::sse::BoxedStream,
    dropped: Arc<AtomicBool>,
}
impl Stream for TrackedStream {
    type Item = http_client::Result<Bytes>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}
impl Drop for TrackedStream {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
    }
}

impl HttpClientExt for Client {
    fn send<T, U>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        let (parts, body) = req.into_parts();
        let body = body.into();
        self.raw_requests.lock().unwrap().push(body.clone());
        self.requests
            .lock()
            .unwrap()
            .push(serde_json::from_slice(&body).unwrap());
        self.unary.send(Request::from_parts(parts, body))
    }
    fn send_multipart<U>(
        &self,
        req: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        self.unary.send_multipart(req)
    }
    fn send_streaming<T>(
        &self,
        req: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        let body: Bytes = req.into_body().into();
        self.raw_requests.lock().unwrap().push(body.clone());
        self.requests
            .lock()
            .unwrap()
            .push(serde_json::from_slice(&body).unwrap());
        // One-byte transport chunks split every UTF-8 code point and SSE/NDJSON delimiter.
        let mut chunks = self
            .bytes
            .iter()
            .map(|byte| Ok(Bytes::from(vec![*byte])))
            .collect::<Vec<_>>();
        if self.transport_error {
            chunks.push(Err(http_client::Error::StreamEnded));
        }
        let mut inner: http_client::sse::BoxedStream = Box::pin(futures::stream::iter(chunks));
        if self.pending {
            inner = Box::pin(inner.chain(futures::stream::pending()));
        }
        let tracked: http_client::sse::BoxedStream = Box::pin(TrackedStream {
            inner,
            dropped: self.dropped.clone(),
        });
        async move {
            Response::builder()
                .status(200)
                .header("content-type", "text/event-stream")
                .body(tracked)
                .map_err(http_client::Error::Protocol)
        }
    }
}

fn schema() -> Value {
    json!({"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"],"additionalProperties":false})
}
fn request(mode: StructuredOutputMode) -> CompletionRequest {
    CompletionRequest {
        messages: vec![Message::user("Return JSON with the string answer 你好.")],
        output_format: Some(OutputFormat::Structured {
            name: "answer".into(),
            schema: schema(),
            mode,
        }),
        generation: armillae_core::GenerationOptions {
            max_output_tokens: Some(128),
            ..Default::default()
        },
        ..Default::default()
    }
}
fn supported(provider: &str, mode: StructuredOutputMode) -> bool {
    match mode {
        StructuredOutputMode::NativeStrict => matches!(
            provider,
            "openai" | "openai-compatible" | "anthropic" | "ollama"
        ),
        StructuredOutputMode::JsonObjectValidated => provider != "anthropic",
        _ => false,
    }
}
fn bridge(provider: &str, client: Client) -> Arc<dyn LlmBridge> {
    let (config, credential) = super::test_support::resolved_config(
        provider,
        Some("http://provider.test"),
        provider != "ollama",
        json!({}),
    );
    bridge_config(provider, client, config, credential).unwrap()
}

fn bridge_config(
    provider: &str,
    client: Client,
    config: armillae_llm::BridgeConfig,
    credential: Option<armillae_llm::SecretString>,
) -> Result<Arc<dyn LlmBridge>, BridgeError> {
    match provider {
        "openai" | "openai-compatible" => {
            super::openai::structured_test_bridge(config, credential, client)
        }
        "anthropic" => super::anthropic::structured_test_bridge(config, credential, client),
        "ollama" => super::ollama::structured_test_bridge(config, credential, client),
        "deepseek" => super::deepseek::structured_test_bridge(config, credential, client),
        "moonshot" => super::moonshot::structured_test_bridge(config, credential, client),
        "minimax" => super::minimax::structured_test_bridge(config, credential, client),
        _ => unreachable!(),
    }
}

fn client(provider: &str, text: &str, terminal: bool) -> Client {
    client_with_reason(provider, text, terminal, "stop")
}

fn client_with_reason(provider: &str, text: &str, terminal: bool, reason: &str) -> Client {
    let usage = json!({"prompt_tokens":3,"completion_tokens":2,"total_tokens":5,
        "prompt_cache_hit_tokens":0,"prompt_cache_miss_tokens":3,"prompt_tokens_details":{"cached_tokens":0}});
    let mut unary = match provider {
        "anthropic" => json!({"id":"response","type":"message","role":"assistant","model":"model",
            "content":[{"type":"text","text":text}],"stop_reason":"end_turn","stop_sequence":null,
            "usage":{"input_tokens":3,"output_tokens":2}}),
        "ollama" => json!({"model":"model","created_at":"2026-09-10T00:00:00Z",
            "message":{"role":"assistant","content":text},"done":true,"done_reason":"stop","prompt_eval_count":3,"eval_count":2}),
        _ => json!({"id":"response","object":"chat.completion","created":0,"model":"model",
            "choices":[{"index":0,"message":{"role":"assistant","content":text},"finish_reason":"stop"}],"usage":usage}),
    };
    let native_reason = match (provider, reason) {
        ("anthropic", "stop") => "end_turn",
        ("anthropic", "length") => "max_tokens",
        _ => reason,
    };
    match provider {
        "anthropic" => unary["stop_reason"] = json!(native_reason),
        "ollama" => unary["done_reason"] = json!(native_reason),
        _ => unary["choices"][0]["finish_reason"] = json!(native_reason),
    };
    let mut events = Vec::new();
    if provider == "anthropic" {
        events.push(json!({"type":"message_start","message":{"id":"response","type":"message","role":"assistant","model":"model","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":0}}}));
        events.push(json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}));
    }
    for ch in text.chars() {
        events.push(match provider {
            "anthropic" => json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":ch.to_string()}}),
            "ollama" => json!({"model":"model","created_at":"2026-09-10T00:00:00Z","message":{"role":"assistant","content":ch.to_string()},"done":false}),
            _ => json!({"id":"response","model":"model","choices":[{"index":0,"delta":{"content":ch.to_string()},"finish_reason":null}]}),
        });
    }
    if terminal {
        match provider {
            "anthropic" => {
                events.push(json!({"type":"content_block_stop","index":0}));
                events.push(json!({"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":2}}));
                events.push(json!({"type":"message_stop"}));
            }
            "ollama" => events.push(json!({"model":"model","created_at":"2026-09-10T00:00:00Z","message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":3,"eval_count":2})),
            _ => events.push(json!({"id":"response","model":"model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":usage})),
        }
    }
    let mut wire = events
        .into_iter()
        .map(|event| match provider {
            "ollama" => format!("{event}\n"),
            "anthropic" => format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            ),
            _ => format!("data: {event}\n\n"),
        })
        .collect::<String>();
    if terminal && !matches!(provider, "anthropic" | "ollama") {
        wire.push_str("data: [DONE]\n\n");
    }
    Client {
        unary: RecordingHttpClient::new(unary.to_string()),
        bytes: wire.into_bytes(),
        ..Default::default()
    }
}

fn assert_wire(provider: &str, mode: StructuredOutputMode, wire: &Value, streaming: bool) {
    match mode {
        StructuredOutputMode::NativeStrict => match provider {
            "anthropic" => assert_eq!(wire["output_config"]["format"]["schema"], schema()),
            "ollama" => assert_eq!(wire["format"], schema()),
            _ => {
                assert_eq!(wire["response_format"]["type"], "json_schema");
                assert_eq!(wire["response_format"]["json_schema"]["strict"], true);
                assert_eq!(wire["response_format"]["json_schema"]["schema"], schema());
                assert_eq!(wire["response_format"]["json_schema"]["name"], "answer");
            }
        },
        StructuredOutputMode::JsonObjectValidated => {
            if provider == "ollama" {
                assert_eq!(wire["format"], json!({"type":"object"}));
            } else {
                assert_eq!(wire["response_format"], json!({"type":"json_object"}));
            }
        }
        _ => unreachable!(),
    }
    if streaming {
        assert_eq!(wire["stream"], true);
    }
    assert_eq!(
        wire["messages"].as_array().unwrap().len(),
        1,
        "must not inject a prompt"
    );
}

#[test]
fn all_provider_modes_complete_and_stream_wire_validation_matrix() {
    for provider in PROVIDERS {
        for mode in MODES {
            for streaming in [false, true] {
                let client = client(provider, VALID, true);
                let bridge = bridge(provider, client.clone());
                let input = request(mode);
                let original = input.clone();
                let projected = bridge.project(&input);
                assert_eq!(input, original);
                assert!(client.requests.lock().unwrap().is_empty());
                futures::executor::block_on(async {
                    let response = if streaming {
                        match bridge.stream(input).await {
                            Err(error) => Err(error),
                            Ok(stream) => {
                                let events = stream
                                    .collect::<Vec<_>>()
                                    .await
                                    .into_iter()
                                    .collect::<Result<Vec<_>, _>>()
                                    .unwrap_or_else(|error| panic!("{provider} {mode:?}: {error}"));
                                let response =
                                    armillae_llm::mock::contract::validate_stream_events(&events);
                                assert!(response.is_ok(), "{provider} {mode:?}: {events:?}");
                                assert!(client.dropped.load(Ordering::SeqCst));
                                response
                                    .cloned()
                                    .map_err(|_| BridgeError::InvalidOutputSchema)
                            }
                        }
                    } else {
                        bridge.complete(input).await
                    };
                    if supported(provider, mode) {
                        projected.unwrap();
                        let response = response.unwrap_or_else(|error| {
                            panic!("{provider} {mode:?} streaming={streaming}: {error}")
                        });
                        let text = response
                            .content
                            .iter()
                            .filter_map(|part| match part {
                                AssistantContent::Text(text) => Some(text.text.as_str()),
                                _ => None,
                            })
                            .collect::<String>();
                        assert_eq!(text, VALID);
                        assert!(response.usage.is_some(), "{provider}");
                        let requests = client.requests.lock().unwrap();
                        assert_eq!(requests.len(), 1);
                        assert_wire(provider, mode, &requests[0], streaming);
                    } else {
                        assert!(matches!(
                            projected,
                            Err(BridgeError::UnsupportedCapability { .. })
                        ));
                        assert!(
                            matches!(response, Err(BridgeError::UnsupportedCapability { .. })),
                            "{provider} {mode:?}"
                        );
                        assert!(
                            client.requests.lock().unwrap().is_empty(),
                            "must reject before HTTP"
                        );
                    }
                });
            }
        }
    }
}

#[test]
fn all_supported_modes_reject_bad_json_and_schema_results_on_both_paths() {
    for provider in PROVIDERS {
        for mode in MODES {
            if !supported(provider, mode) {
                continue;
            }
            for (text, kind) in [
                ("not-json", StructuredOutputErrorKind::InvalidJson),
                (
                    r#"{"answer":123}"#,
                    StructuredOutputErrorKind::SchemaMismatch,
                ),
            ] {
                for streaming in [false, true] {
                    let client = client(provider, text, true);
                    let bridge = bridge(provider, client.clone());
                    futures::executor::block_on(async {
                        let error = if streaming {
                            let events = bridge
                                .stream(request(mode))
                                .await
                                .unwrap()
                                .collect::<Vec<_>>()
                                .await;
                            assert_eq!(
                                events.iter().filter(|event| event.is_err()).count(),
                                1,
                                "{provider}"
                            );
                            assert!(!events.iter().any(|event| matches!(
                                event,
                                Ok(CompletionEvent::ResponseCompleted { .. })
                            )));
                            events.into_iter().find_map(Result::err).unwrap()
                        } else {
                            bridge.complete(request(mode)).await.unwrap_err()
                        };
                        assert_eq!(
                            error,
                            BridgeError::StructuredOutput { kind },
                            "{provider} {mode:?} streaming={streaming}"
                        );
                        assert_eq!(client.requests.lock().unwrap().len(), 1, "no retries");
                    });
                }
            }
        }
    }
}

#[test]
fn all_supported_modes_preflight_schema_errors_on_complete_stream_and_project() {
    for provider in PROVIDERS {
        for mode in MODES {
            if !supported(provider, mode) {
                continue;
            }
            let client = client(provider, VALID, true);
            let bridge = bridge(provider, client.clone());
            let mut input = request(mode);
            if let Some(OutputFormat::Structured { schema, .. }) = &mut input.output_format {
                *schema = json!({"$ref":"https://127.0.0.1/secret"});
            }
            assert_eq!(
                bridge.project(&input),
                Err(BridgeError::InvalidOutputSchema)
            );
            futures::executor::block_on(async {
                assert_eq!(
                    bridge.complete(input.clone()).await,
                    Err(BridgeError::InvalidOutputSchema)
                );
                assert!(matches!(
                    bridge.stream(input).await,
                    Err(BridgeError::InvalidOutputSchema)
                ));
            });
            assert!(client.requests.lock().unwrap().is_empty());
        }
    }
}

#[test]
fn all_supported_streams_fail_on_interruption_and_drop_without_drain() {
    let mut false_successes = Vec::new();
    for provider in PROVIDERS {
        for mode in MODES {
            if !supported(provider, mode) {
                continue;
            }
            for pending in [false, true] {
                let mut client = client(provider, if pending { "{" } else { VALID }, false);
                client.pending = pending;
                let bridge = bridge(provider, client.clone());
                futures::executor::block_on(async {
                    let mut stream = bridge.stream(request(mode)).await.unwrap();
                    if pending {
                        loop {
                            match stream.next().await {
                                Some(Ok(CompletionEvent::TextDelta { .. })) => break,
                                Some(Ok(_)) => {}
                                other => panic!(
                                    "{provider}: expected preview before cancellation: {other:?}"
                                ),
                            }
                        }
                        drop(stream);
                    } else {
                        let events = stream.collect::<Vec<_>>().await;
                        if events.iter().filter(|event| event.is_err()).count() != 1
                            || events.iter().any(|event| {
                                matches!(event, Ok(CompletionEvent::ResponseCompleted { .. }))
                            })
                        {
                            false_successes.push(format!("{provider}/{mode:?}"));
                        }
                    }
                    assert!(client.dropped.load(Ordering::SeqCst), "{provider}");
                });
            }
        }
    }
    assert!(
        false_successes.is_empty(),
        "EOF without a Provider terminal marker was accepted: {false_successes:?}"
    );
}

#[test]
fn native_subset_never_silently_drops_constraints() {
    for provider in PROVIDERS {
        if !supported(provider, StructuredOutputMode::NativeStrict) {
            continue;
        }
        let client = client(provider, VALID, true);
        let bridge = bridge(provider, client.clone());
        let mut input = request(StructuredOutputMode::NativeStrict);
        if let Some(OutputFormat::Structured { schema, .. }) = &mut input.output_format {
            schema["properties"]["answer"]["minLength"] = json!(2);
        }
        assert!(matches!(
            bridge.project(&input),
            Err(BridgeError::UnsupportedCapability { .. })
        ));
        futures::executor::block_on(async {
            assert!(matches!(
                bridge.complete(input.clone()).await,
                Err(BridgeError::UnsupportedCapability { .. })
            ));
            assert!(matches!(
                bridge.stream(input).await,
                Err(BridgeError::UnsupportedCapability { .. })
            ));
        });
        assert!(client.requests.lock().unwrap().is_empty());
    }
}

#[test]
fn all_supported_streams_preserve_non_success_reasons_and_never_retry_errors() {
    for provider in PROVIDERS {
        for mode in MODES {
            if !supported(provider, mode) {
                continue;
            }
            for case in [
                "length",
                "refusal",
                "future_reason",
                "malformed",
                "transport",
            ] {
                let mut client = client(provider, VALID, case != "transport");
                let mut wire = String::from_utf8(client.bytes.clone()).unwrap();
                if case == "malformed" {
                    let bad = if provider == "ollama" {
                        "{bad-json}\n"
                    } else {
                        "data: {bad-json}\n\n"
                    };
                    wire = format!("{bad}{wire}");
                } else if case == "transport" {
                    client.transport_error = true;
                } else {
                    let reason = match (provider, case) {
                        ("anthropic", "length") => "max_tokens",
                        (_, "refusal") => "refusal",
                        _ => case,
                    };
                    wire = wire
                        .replace(
                            "\"finish_reason\":\"stop\"",
                            &format!("\"finish_reason\":\"{reason}\""),
                        )
                        .replace(
                            "\"stop_reason\":\"end_turn\"",
                            &format!("\"stop_reason\":\"{reason}\""),
                        )
                        .replace(
                            "\"done_reason\":\"stop\"",
                            &format!("\"done_reason\":\"{reason}\""),
                        );
                }
                client.bytes = wire.into_bytes();
                let bridge = bridge(provider, client.clone());
                futures::executor::block_on(async {
                    let events = bridge
                        .stream(request(mode))
                        .await
                        .unwrap()
                        .collect::<Vec<_>>()
                        .await;
                    let errors = events
                        .iter()
                        .filter_map(|e| e.as_ref().err())
                        .collect::<Vec<_>>();
                    assert_eq!(errors.len(), 1, "{provider} {mode:?} {case}");
                    assert!(
                        !events
                            .iter()
                            .any(|e| matches!(e, Ok(CompletionEvent::ResponseCompleted { .. }))),
                        "{provider} {case}"
                    );
                    if matches!(case, "length" | "refusal" | "future_reason") {
                        assert_eq!(
                            *errors[0],
                            BridgeError::StructuredOutput {
                                kind: StructuredOutputErrorKind::Incomplete
                            }
                        );
                    } else {
                        assert!(matches!(errors[0], BridgeError::StreamInterrupted { .. }));
                    }
                    assert_eq!(
                        client.requests.lock().unwrap().len(),
                        1,
                        "{provider} {case}: no retry"
                    );
                    assert!(
                        client.dropped.load(Ordering::SeqCst),
                        "{provider} {case}: release body"
                    );
                });
            }
        }
    }
}

#[test]
fn all_supported_unary_modes_reject_non_success_finish_reasons() {
    for provider in PROVIDERS {
        for mode in MODES {
            if !supported(provider, mode) {
                continue;
            }
            for reason in ["length", "refusal", "future_reason"] {
                let client = client_with_reason(provider, VALID, true, reason);
                let bridge = bridge(provider, client.clone());
                let error =
                    futures::executor::block_on(bridge.complete(request(mode))).unwrap_err();
                assert_eq!(
                    error,
                    BridgeError::StructuredOutput {
                        kind: StructuredOutputErrorKind::Incomplete
                    },
                    "{provider} {mode:?} {reason}"
                );
                assert_eq!(client.requests.lock().unwrap().len(), 1);
            }
        }
    }
}

#[tokio::test]
async fn explicit_reasoning_controls_reach_all_seven_provider_wires() {
    use armillae_core::{GenerationOptions, ReasoningEffort as E, Thinking};
    for (provider, model, thinking, effort, expected) in [
        (
            "openai",
            "test-model",
            Some(Thinking::Enabled),
            Some(E::High),
            json!({"reasoning_effort":"high"}),
        ),
        (
            "openai-compatible",
            "test-model",
            Some(Thinking::Disabled),
            None,
            json!({"reasoning_effort":"none"}),
        ),
        (
            "deepseek",
            "deepseek-v4-pro",
            Some(Thinking::Enabled),
            Some(E::High),
            json!({"thinking":{"type":"enabled"},"reasoning_effort":"high"}),
        ),
        (
            "moonshot",
            "kimi-k2.6",
            Some(Thinking::Disabled),
            None,
            json!({"thinking":{"type":"disabled"}}),
        ),
        (
            "moonshot",
            "kimi-k3",
            None,
            Some(E::Max),
            json!({"reasoning_effort":"max"}),
        ),
        (
            "minimax",
            "MiniMax-M3",
            Some(Thinking::Disabled),
            None,
            json!({"thinking":{"type":"disabled"}}),
        ),
        (
            "minimax",
            "MiniMax-M3",
            Some(Thinking::Enabled),
            None,
            json!({"thinking":{"type":"adaptive"}}),
        ),
        (
            "anthropic",
            "claude-sonnet-4-6",
            Some(Thinking::Adaptive),
            Some(E::High),
            json!({"thinking":{"type":"adaptive"},"output_config":{"effort":"high"}}),
        ),
        (
            "anthropic",
            "claude-sonnet-4-5",
            Some(Thinking::Budget { tokens: 1024 }),
            None,
            json!({"thinking":{"type":"enabled","budget_tokens":1024}}),
        ),
        (
            "ollama",
            "qwen3",
            Some(Thinking::Disabled),
            None,
            json!({"think":false}),
        ),
        (
            "ollama",
            "gpt-oss:20b",
            None,
            Some(E::Low),
            json!({"think":"low"}),
        ),
    ] {
        for streaming in [false, true] {
            let client = client(provider, "hello", true);
            let (mut config, credential) = super::test_support::resolved_config(
                provider,
                Some("http://provider.test"),
                provider != "ollama",
                json!({}),
            );
            config.model = model.into();
            config.defaults = GenerationOptions {
                thinking,
                reasoning_effort: effort,
                max_output_tokens: Some(4096),
                ..Default::default()
            };
            let bridge = bridge_config(provider, client.clone(), config, credential).unwrap();
            let req = CompletionRequest {
                messages: vec![Message::user("hello")],
                ..Default::default()
            };
            bridge.project(&req).unwrap();
            assert!(client.requests.lock().unwrap().is_empty());
            if streaming {
                let events = bridge.stream(req).await.unwrap().collect::<Vec<_>>().await;
                assert!(events.iter().all(Result::is_ok), "{provider}: {events:?}");
            } else {
                bridge.complete(req).await.unwrap();
            }
            let requests = client.requests.lock().unwrap();
            let body = &requests[0];
            for (key, value) in expected.as_object().unwrap() {
                assert_eq!(
                    &body[key], value,
                    "{provider} {model} streaming={streaming}"
                );
            }
        }
    }
}

#[tokio::test]
async fn reasoning_defaults_can_be_overridden_and_cleared_without_mutating_history() {
    use armillae_core::{GenerationOptions, ReasoningEffort as E, Thinking};
    let client = client("deepseek", "hello", true);
    let (mut config, credential) = super::test_support::resolved_config(
        "deepseek",
        Some("http://provider.test"),
        true,
        json!({}),
    );
    config.defaults = GenerationOptions {
        thinking: Some(Thinking::Enabled),
        reasoning_effort: Some(E::High),
        ..Default::default()
    };
    let bridge = bridge_config("deepseek", client.clone(), config, credential).unwrap();
    for streaming in [false, true] {
        for (thinking, effort) in [
            (Thinking::Enabled, E::Low),
            (Thinking::ProviderDefault, E::ProviderDefault),
        ] {
            let req = CompletionRequest {
                messages: vec![Message::user("hello")],
                generation: GenerationOptions {
                    thinking: Some(thinking),
                    reasoning_effort: Some(effort),
                    ..Default::default()
                },
                ..Default::default()
            };
            let original = req.clone();
            bridge.project(&req).unwrap();
            assert_eq!(req, original);
            if streaming {
                assert!(
                    bridge
                        .stream(req)
                        .await
                        .unwrap()
                        .collect::<Vec<_>>()
                        .await
                        .iter()
                        .all(Result::is_ok)
                );
            } else {
                bridge.complete(req).await.unwrap();
            }
            let requests = client.requests.lock().unwrap();
            let body = requests.last().unwrap();
            if effort == E::Low {
                assert_eq!(body["reasoning_effort"], "low");
            } else {
                assert!(body.get("reasoning_effort").is_none());
                assert!(body.get("thinking").is_none());
            }
        }
    }
}

#[tokio::test]
async fn anthropic_effort_and_native_schema_share_one_output_config() {
    use armillae_core::{ReasoningEffort, Thinking};
    for streaming in [false, true] {
        let client = client("anthropic", VALID, true);
        let bridge = bridge("anthropic", client.clone());
        let mut req = request(StructuredOutputMode::NativeStrict);
        req.generation.thinking = Some(Thinking::Adaptive);
        req.generation.reasoning_effort = Some(ReasoningEffort::High);
        if streaming {
            assert!(
                bridge
                    .stream(req)
                    .await
                    .unwrap()
                    .collect::<Vec<_>>()
                    .await
                    .iter()
                    .all(Result::is_ok)
            );
        } else {
            bridge.complete(req).await.unwrap();
        }
        let requests = client.requests.lock().unwrap();
        let raw = client.raw_requests.lock().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&raw[0])
                .matches("\"output_config\"")
                .count(),
            1
        );
        assert_eq!(requests[0]["output_config"]["effort"], "high");
        assert_eq!(requests[0]["output_config"]["format"]["schema"], schema());
    }
}

#[tokio::test]
async fn invalid_reasoning_is_rejected_before_any_http_request() {
    use armillae_core::{GenerationOptions, ReasoningEffort as E, Thinking};
    for (provider, model, thinking, effort) in [
        ("openai", "test-model", Some(Thinking::Enabled), None),
        (
            "openai-compatible",
            "test-model",
            Some(Thinking::Disabled),
            Some(E::High),
        ),
        ("deepseek", "deepseek-v4-pro", None, Some(E::Medium)),
        ("moonshot", "kimi-k3", Some(Thinking::Disabled), None),
        ("moonshot", "kimi-k2.6", None, Some(E::High)),
        ("minimax", "MiniMax-M2.7", Some(Thinking::Disabled), None),
        ("minimax", "MiniMax-M3", None, Some(E::High)),
        (
            "anthropic",
            "claude-sonnet-4-6",
            Some(Thinking::Budget { tokens: 1 }),
            None,
        ),
        (
            "anthropic",
            "claude-sonnet-4-5",
            Some(Thinking::Adaptive),
            None,
        ),
        ("ollama", "gpt-oss:20b", Some(Thinking::Disabled), None),
        ("ollama", "qwen3", Some(Thinking::Disabled), Some(E::High)),
    ] {
        let client = client(provider, "hello", true);
        let (mut config, credential) = super::test_support::resolved_config(
            provider,
            Some("http://provider.test"),
            provider != "ollama",
            json!({}),
        );
        config.model = model.into();
        let bridge =
            bridge_config(provider, client.clone(), config.clone(), credential.clone()).unwrap();
        let req = CompletionRequest {
            messages: vec![Message::user("hello")],
            generation: GenerationOptions {
                thinking,
                reasoning_effort: effort,
                max_output_tokens: Some(4096),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(bridge.project(&req).is_err(), "{provider}/{model}");
        assert!(bridge.complete(req.clone()).await.is_err());
        assert!(bridge.stream(req.clone()).await.is_err());
        assert!(client.requests.lock().unwrap().is_empty());
        config.defaults = req.generation;
        assert!(matches!(
            bridge_config(provider, client.clone(), config, credential),
            Err(BridgeError::InvalidConfiguration { .. })
        ));
    }
}
