# LLM Bridge guide

Armillae exposes one provider-independent `LlmBridge` call at a time. A Bridge can return text and
ToolCalls, but it never executes a Tool or automatically calls the model again. Applications own
that control flow and can combine the Bridge with `armillae-tools::ToolExecutor` explicitly.

## Supported Rig providers

The table describes the conservative Adapter profile, not every capability a remote model may
have. Unsupported requests fail local preflight; a remote model that is weaker than its profile
returns a normalized Provider error instead of triggering an automatic downgrade.

| Provider | Credential | Streaming | ToolChoice | Structured output | Roles and notable limits |
| --- | --- | --- | --- | --- | --- |
| `openai` | required | yes | auto, none, required, specific | JSON Object, JSON Schema | System; no Developer |
| `openai-compatible` | required; endpoint required | yes | auto, none, required, specific | JSON Object, JSON Schema | Caller asserts the endpoint follows the OpenAI profile |
| `deepseek` | required | yes | auto, none | JSON Object | System; no Developer |
| `minimax` | required | yes | auto, none, required, specific | JSON Object, JSON Schema | OpenAI-compatible path only |
| `moonshot` | required | yes | auto, none | JSON Object | OpenAI-compatible path only |
| `anthropic` | required | yes | auto, none, required, specific | strict JSON Schema | Requires max output tokens; leading System only |
| `ollama` | optional | yes | none declared | JSON Object, strict JSON Schema | Defaults to `http://localhost:11434`; no Developer |

All profiles support Tool Definitions, one or more ToolCalls, and a later ToolResult request.
OpenAI-family wire formats do not carry `ToolResult.is_error`; Armillae preserves the caller's
content and does not invent an error wrapper. Anthropic can carry the flag natively, but Rig 0.41
cannot preserve it, so its Adapter rejects `is_error = true`. Ollama has no native ToolCall ID;
Armillae generates unique IDs and maps them back to native tool names when the result is sent.

Rig 0.41 filters unknown raw Anthropic SSE and Ollama NDJSON fields before the Adapter sees them.
Armillae does not duplicate either transport to recover those events. Choose another Driver if raw
unknown events are an application requirement.

## Equivalent configuration forms

TOML, JSON, and the Rust builder produce the same validated `BridgeConfig`. Secret values are never
stored in the serializable configuration.

```toml
api_version = "armillae.llm/v1alpha1"
provider = "openai"
model = "gpt-4.1-mini"

[credential]
type = "environment"
name = "OPENAI_API_KEY"

[transport]
connect_timeout_ms = 5000
request_timeout_ms = 60000

[defaults]
temperature = 0.2
max_output_tokens = 512
stop = []
```

```json
{
  "api_version": "armillae.llm/v1alpha1",
  "provider": "openai",
  "model": "gpt-4.1-mini",
  "endpoint": null,
  "credential": { "type": "environment", "name": "OPENAI_API_KEY" },
  "transport": { "connect_timeout_ms": 5000, "request_timeout_ms": 60000 },
  "defaults": {
    "temperature": 0.2,
    "max_output_tokens": 512,
    "stop": [],
    "seed": null
  },
  "provider_options": {}
}
```

```rust
use armillae_core::GenerationOptions;
use armillae_llm::{BridgeConfig, CredentialRef, TransportConfig};

let config = BridgeConfig::builder("openai", "gpt-4.1-mini")
    .credential(CredentialRef::Environment {
        name: "OPENAI_API_KEY".to_owned(),
    })
    .transport(TransportConfig {
        connect_timeout_ms: 5_000,
        request_timeout_ms: 60_000,
    })
    .defaults(GenerationOptions {
        temperature: Some(0.2),
        max_output_tokens: Some(512),
        ..GenerationOptions::default()
    })
    .build()?;
# Ok::<(), armillae_llm::BridgeError>(())
```

`BridgeConfig` intentionally does not select an Adapter Driver. Applications may keep a `driver`
field in their own outer configuration and use it at runtime to choose `RigBridgeFactory` or a
future Factory. Armillae only receives the Provider configuration after that host-owned routing
decision, so it does not prescribe a configuration loader or require a compile-time Factory choice.

Resolve the credential, then construct the Driver:

```rust
use armillae_llm::{BridgeFactory, LlmBridge};
use armillae_llm_rig::RigBridgeFactory;

# async fn create(config: armillae_llm::BridgeConfig)
#     -> Result<std::sync::Arc<dyn LlmBridge>, armillae_llm::BridgeError> {
let resolved = config.resolve().await?;
let bridge = RigBridgeFactory.create(resolved).await?;
# Ok(bridge)
# }
```

## Canonical history and target projection

`CompletionRequest` and its messages remain Armillae-owned canonical data. Every Bridge projects
that request to its own Provider at the send boundary. Known private response data, such as
DeepSeek reasoning, Anthropic signed thinking, Ollama thinking, or Provider ToolCall metadata, is
replayed when the next request targets the same Provider.

An application can inspect the projection before sending:

```rust
use armillae_core::{CompletionRequest, Message};
use armillae_llm::LlmBridge;

# fn inspect(bridge: &dyn LlmBridge) -> Result<(), armillae_llm::BridgeError> {
let request = CompletionRequest {
    messages: vec![Message::user("Continue the conversation")],
    ..CompletionRequest::default()
};
let report = bridge.project(&request)?;
if !report.is_exact() {
    // Inspect content-free compatibility facts before deciding whether to send.
    let _facts = &report.facts;
}
# Ok(())
# }
```

`project` is synchronous, performs no network I/O, does not mutate the request, and does not consume
a Mock script item. An exact projection has no facts. Private data from another Provider, or an
unknown private kind, remains in the caller's canonical history but is not sent to the target;
the report contains a content-free `NotForwarded` fact. Malformed private data that the same
Provider claims to understand returns `BridgeError::ProjectionIncompatible` instead of being
dropped.

The shared Rig response boundary treats one unsigned empty reasoning text block with no Provider ID
as no reasoning for every Provider. DeepSeek `reasoning_content: ""`, for example, therefore never
enters canonical history. Reasoning with an ID, signature, encrypted/redacted payload, summary, or
non-empty text is preserved; unknown and malformed Provider data remains subject to the normal
preservation and projection checks.

`LlmRouter` is not required for this behavior. A host may manually call `project` and `complete` on
another Bridge using the same original request. The target Bridge performs its own projection;
the host remains responsible for choosing which errors permit another attempt and must not combine
two response streams after semantic output has started.

`resolve()` is the common path for Environment, File, or credential-free configurations. It
validates the configuration and turns the credential reference into a non-serializable, redacted
`ResolvedBridgeConfig`. A host only needs `resolve_with(...)` when it uses
`CredentialRef::Resolver` for an external Secret store or applies an `EndpointPolicy` to an
explicit endpoint:

```rust
use armillae_llm::BridgeResolveContext;

# async fn resolve_with_host_services(
#     config: &armillae_llm::BridgeConfig,
#     secret_resolver: &dyn armillae_llm::SecretResolver,
#     endpoint_policy: &dyn armillae_llm::EndpointPolicy,
# ) -> Result<(), armillae_llm::BridgeError> {
let context = BridgeResolveContext::new()
    .secret_resolver(secret_resolver)
    .endpoint_policy(endpoint_policy);
let resolved = config.resolve_with(context).await?;
# let _ = resolved;
# Ok(())
# }
```

The Secret Resolver is consulted only for `CredentialRef::Resolver { key }`; Environment and File
references remain built in. EndpointPolicy is a host allowlist or trust rule for custom endpoints,
not a Provider setting. Named Providers using their default endpoint normally need neither hook.

For a local Ollama daemon, use provider `ollama`, a model such as `qwen3:8b`, no credential, and no
endpoint. Configure a credential only for a protected proxy. All explicit endpoints pass structural
URL validation; applications accepting untrusted configuration should also supply an
`EndpointPolicy` that restricts schemes, hosts, and resolved network ranges.

## Examples

Every Bridge invocation below is exactly one model call. Only the manual Tool example performs a
second call, and the application does so explicitly after executing the returned ToolCall. The full
sources compile as `armillae-llm-rig` example targets.

### One non-streaming completion

Build and resolve a configuration, construct the Rig Driver, and submit a Provider-independent
request:

```rust
use armillae_core::{CompletionRequest, Message};
use armillae_llm::{BridgeConfig, BridgeFactory, CredentialRef};
use armillae_llm_rig::RigBridgeFactory;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let config = BridgeConfig::builder("openai", "gpt-4.1-mini")
    .credential(CredentialRef::Environment {
        name: "OPENAI_API_KEY".to_owned(),
    })
    .build()?;
let bridge = RigBridgeFactory
    .create(config.resolve().await?)
    .await?;
let response = bridge
    .complete(CompletionRequest {
        messages: vec![Message::user("Explain Armillae in one sentence.")],
        ..CompletionRequest::default()
    })
    .await?;

for content in response.content {
    println!("{content:?}");
}
# Ok(())
# }
```

The complete target is
[`simple_completion.rs`](../crates/armillae-llm-rig/examples/simple_completion.rs).

### Streaming text

`stream` returns semantic Armillae events rather than raw Provider SSE or NDJSON chunks. Consume
`TextDelta` for incremental output and use the unique `ResponseCompleted` event as the final
normalized response:

```rust
use armillae_core::CompletionEvent;
use futures::StreamExt;

# async fn example(
#     bridge: &dyn armillae_llm::LlmBridge,
#     request: armillae_core::CompletionRequest,
# ) -> Result<(), Box<dyn std::error::Error>> {
let mut stream = bridge.stream(request).await?;
while let Some(event) = stream.next().await {
    match event? {
        CompletionEvent::TextDelta { text, .. } => print!("{text}"),
        CompletionEvent::ResponseCompleted { response } => {
            println!("\nusage: {:?}", response.usage);
        }
        _ => {}
    }
}
# Ok(())
# }
```

The complete target, including stdout flushing, is
[`streaming.rs`](../crates/armillae-llm-rig/examples/streaming.rs).

### Strict structured output

Use `OutputFormat::JsonSchema` when the selected Provider supports it. Generate the schema from the
same Rust type used to deserialize the returned JSON; capability preflight rejects unsupported
Provider/profile combinations before making a request:

```rust
use armillae_core::{CompletionRequest, Message, OutputFormat};
use schemars::{JsonSchema, schema_for};
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReleaseSummary {
    title: String,
    highlights: Vec<String>,
}

# async fn example(
#     bridge: &dyn armillae_llm::LlmBridge,
# ) -> Result<(), Box<dyn std::error::Error>> {
let response = bridge
    .complete(CompletionRequest {
        messages: vec![Message::user("Return a short release summary as JSON.")],
        output_format: Some(OutputFormat::JsonSchema {
            name: "release_summary".to_owned(),
            schema: serde_json::to_value(schema_for!(ReleaseSummary))?,
            strict: true,
        }),
        ..CompletionRequest::default()
    })
    .await?;
# let _ = response;
# Ok(())
# }
```

[`structured_output.rs`](../crates/armillae-llm-rig/examples/structured_output.rs) also extracts the
text content and deserializes it back into `ReleaseSummary`.

### Manual ToolCall continuation

A Bridge returns ToolCalls but never executes them. The application owns execution, history, and
the decision to call the model again:

```rust
# async fn example(
#     bridge: &dyn armillae_llm::LlmBridge,
#     tools: &dyn armillae_tools::ToolExecutor,
#     first_request: armillae_core::CompletionRequest,
# ) -> Result<(), Box<dyn std::error::Error>> {
use armillae_core::{CompletionRequest, Message, ToolChoice};
use armillae_tools::ToolContext;

let definitions = tools.definitions();
let mut first_request = first_request;
first_request.tools = definitions.clone();
let mut history = first_request.messages.clone();
let first = bridge.complete(first_request).await?;
let calls = first.tool_calls().cloned().collect::<Vec<_>>();
history.push(first.as_assistant_message());

for call in calls {
    let result = tools.execute(ToolContext::default(), call).await?;
    history.push(Message::tool_result(result));
}

let final_response = bridge
    .complete(CompletionRequest {
        messages: history,
        tools: definitions,
        tool_choice: Some(ToolChoice::None),
        ..CompletionRequest::default()
    })
    .await?;
# let _ = final_response;
# Ok(())
# }
```

The full [`manual_tool_flow.rs`](../crates/armillae-llm-rig/examples/manual_tool_flow.rs) defines a
typed Tool and registers it in `ToolRegistry` before running this flow.

### Multi-turn DeepSeek conversation with local Tool dispatch

The DeepSeek example keeps System, User, Assistant, and ToolResult messages in application-owned
history. Each line entered by the user first creates a request with a small local project-fact Tool
and `ToolChoice::Auto`. If DeepSeek returns ToolCalls, the application executes them sequentially,
appends their ToolResults, and explicitly makes one final request with `ToolChoice::None`:

```rust
let mut history = vec![Message::new(
    Role::System,
    vec![ContentPart::text("Answer in the same language as the user.")],
)];

history.push(Message::user(prompt));
let first = bridge
    .complete(CompletionRequest {
        messages: history.clone(),
        tools: definitions.clone(),
        tool_choice: Some(ToolChoice::Auto),
        ..CompletionRequest::default()
    })
    .await?;
let calls = first.tool_calls().cloned().collect::<Vec<_>>();
let response = if calls.is_empty() {
    first
} else {
    history.push(first.as_assistant_message());
    for call in calls {
        let result = tools.execute(ToolContext::default(), call).await?;
        history.push(Message::tool_result(result));
    }
    bridge
        .complete(CompletionRequest {
            messages: history.clone(),
            tools: definitions.clone(),
            tool_choice: Some(ToolChoice::None),
            ..CompletionRequest::default()
        })
        .await?
};
history.push(response.as_assistant_message());
```

Export `DEEPSEEK_API_KEY`, then run
[`deepseek_conversation.rs`](../crates/armillae-llm-rig/examples/deepseek_conversation.rs). Enter
`/quit` to leave the conversation, or ask it to use the local lookup Tool for an Armillae fact. A
normal text response remains a single model call; a ToolCall response adds exactly one explicit
continuation call. The example uses Provider `deepseek` and the frozen baseline model
`deepseek-v4-flash`; no custom endpoint is required. The deprecated `deepseek-chat` and
`deepseek-reasoner` aliases are intentionally not used.

### Local Ollama without a credential

Ollama defaults to `http://localhost:11434`, so a local daemon only needs its Provider and model:

```rust
use armillae_llm::{BridgeConfig, BridgeFactory};
use armillae_llm_rig::RigBridgeFactory;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let config = BridgeConfig::builder("ollama", "qwen3:8b").build()?;
let bridge = RigBridgeFactory
    .create(config.resolve().await?)
    .await?;
# let _ = bridge;
# Ok(())
# }
```

Change `qwen3:8b` to a model installed by `ollama pull`. Add a credential only when the Ollama
endpoint is protected by a proxy. See
[`ollama_completion.rs`](../crates/armillae-llm-rig/examples/ollama_completion.rs) for the complete
request.

Run the examples from the workspace root:

```sh
cargo run -p armillae-llm-rig --example simple_completion
cargo run -p armillae-llm-rig --example streaming
cargo run -p armillae-llm-rig --example structured_output
cargo run -p armillae-llm-rig --example manual_tool_flow
cargo run -p armillae-llm-rig --example deepseek_conversation
cargo run -p armillae-llm-rig --example ollama_completion
```

The OpenAI examples require `OPENAI_API_KEY`, and the DeepSeek example requires
`DEEPSEEK_API_KEY`. The Ollama example requires a running daemon and the configured model, but no
API key. These commands make real Provider calls and may incur cost; use
`cargo check -p armillae-llm-rig --examples` to compile them without sending requests.

## Observability and security

The Rig Adapter emits `llm.bridge.call` spans on target `armillae::llm`. They include Adapter,
Provider, configured model, request ID when exposed, streaming mode, Tool Definition and ToolCall
counts, token usage, total latency, streaming first-output latency, and normalized error category.
They never include message content, Tool arguments, ToolResults, raw Provider bodies, credentials,
or Authorization headers.

Armillae always sends `record_telemetry_content = false` to Rig and does not expose a content-debug
switch. Rig 0.41's Ollama implementation can print raw NDJSON on its own `rig` DEBUG target, so do
not enable `rig` or `rig::completions` DEBUG/TRACE in production. This is separate from Armillae's
safe structured target.

Credential values must come from an environment variable, UTF-8 file, or host `SecretResolver`.
Resolved secrets and Provider options are redacted from `Debug`; Provider response bodies are not
copied into normalized errors. Live fixtures must never contain real headers, credentials, private
prompts, or unredacted responses.

## Live support gate

The release gate is frozen to `openai/gpt-4.1-mini`, `deepseek/deepseek-v4-flash`,
`minimax/MiniMax-M2`, and `moonshot/kimi-k2`. The ignored harness covers text, streaming, System and
multi-turn history, structured output, single and multiple ToolCalls, manual ToolResult continuation,
Usage/response facts, local capability rejection, and remote Provider rejection.

Run one Provider at a time on an authorized release workstation after exporting its credential:

```sh
ARMILLAE_LIVE_PROVIDER=openai \
  cargo test -p armillae-llm-rig --test openai_live -- --ignored --test-threads=1
```

Use `OPENAI_API_KEY`, `DEEPSEEK_API_KEY`, `MINIMAX_API_KEY`, or `MOONSHOT_API_KEY` for the selected
Provider. `ARMILLAE_LIVE_MODEL` and `ARMILLAE_LIVE_ENDPOINT` are exploratory overrides and do not
replace a run against the frozen matrix. As of 2026-08-26 no credential-backed run is recorded in
the repository, so Armillae does not make a full-support claim from offline tests alone.
