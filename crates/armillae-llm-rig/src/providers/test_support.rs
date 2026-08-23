use armillae_llm::{
    BoxFuture, BridgeConfig, BridgeError, CredentialRef, SecretResolver, SecretString,
};
use rig_core::test_utils::SequencedStreamingHttpClient;
use serde_json::Value;

struct StaticSecretResolver;

impl SecretResolver for StaticSecretResolver {
    fn resolve<'a>(&'a self, _key: &'a str) -> BoxFuture<'a, Result<SecretString, BridgeError>> {
        Box::pin(async { Ok(SecretString::from("named-provider-test-secret")) })
    }
}

pub(super) fn resolved_config(
    provider: &str,
    endpoint: Option<&str>,
    with_credential: bool,
    provider_options: Value,
) -> (BridgeConfig, Option<SecretString>) {
    let mut builder =
        BridgeConfig::builder("rig", provider, "test-model").provider_options(provider_options);
    if let Some(endpoint) = endpoint {
        builder = builder.endpoint(
            endpoint
                .parse()
                .expect("provider test endpoint must be a valid URL"),
        );
    }
    if with_credential {
        builder = builder.credential(CredentialRef::Resolver {
            key: "named-provider-test".to_owned(),
        });
    }
    let config = builder
        .build()
        .expect("provider test config must pass common validation");
    let resolved = futures::executor::block_on(config.resolve(
        with_credential.then_some(&StaticSecretResolver as &dyn SecretResolver),
        None,
    ))
    .expect("provider test config must resolve");

    resolved.into_parts()
}

pub(super) fn text_stream_client() -> SequencedStreamingHttpClient {
    let events = vec![
        serde_json::json!({
            "id": "stream-test",
            "model": "provider-model",
            "choices": [{
                "delta": { "content": "你" },
                "finish_reason": null
            }],
            "usage": null
        }),
        serde_json::json!({
            "id": "stream-test",
            "model": "provider-model",
            "choices": [{
                "delta": { "content": "好" },
                "finish_reason": null
            }],
            "usage": null
        }),
        serde_json::json!({
            "id": "stream-test",
            "model": "provider-model",
            "choices": [{
                "delta": {},
                "finish_reason": "stop"
            }],
            "usage": null
        }),
        serde_json::json!({
            "id": "stream-test",
            "model": "provider-model",
            "choices": [],
            "usage": {
                "prompt_tokens": 3,
                "completion_tokens": 2,
                "total_tokens": 5,
                "prompt_cache_hit_tokens": 0,
                "prompt_cache_miss_tokens": 3,
                "prompt_tokens_details": { "cached_tokens": 0 }
            }
        }),
    ];
    streaming_client(events)
}

pub(super) fn streaming_client(events: Vec<Value>) -> SequencedStreamingHttpClient {
    let mut sse = events
        .into_iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect::<String>();
    sse.push_str("data: [DONE]\n\n");
    let chunks = sse
        .as_bytes()
        .chunks(5)
        .map(|chunk| Ok::<_, rig_core::http_client::Error>(chunk.to_vec().into()))
        .collect();
    SequencedStreamingHttpClient::new(chunks)
}

pub(super) fn expected_text_stream() -> armillae_core::CompletionResponse {
    armillae_core::CompletionResponse {
        id: None,
        model: None,
        content: vec![armillae_core::AssistantContent::Text(
            armillae_core::TextContent::new("你好"),
        )],
        finish_reason: None,
        usage: Some(armillae_core::TokenUsage {
            input_tokens: Some(3),
            output_tokens: Some(2),
            total_tokens: Some(5),
            cached_input_tokens: Some(0),
        }),
        provider_metadata: serde_json::json!({}),
    }
}
