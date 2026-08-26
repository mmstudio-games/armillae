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
driver = "rig"
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
  "driver": "rig",
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

let config = BridgeConfig::builder("rig", "openai", "gpt-4.1-mini")
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

Resolve the credential, then construct the Driver:

```rust
use armillae_llm::{BridgeFactory, LlmBridge};
use armillae_llm_rig::RigBridgeFactory;

# async fn create(config: armillae_llm::BridgeConfig)
#     -> Result<std::sync::Arc<dyn LlmBridge>, armillae_llm::BridgeError> {
let resolved = config.resolve(None, None).await?;
let bridge = RigBridgeFactory.create(resolved).await?;
# Ok(bridge)
# }
```

For a local Ollama daemon, use provider `ollama`, a model such as `qwen3:8b`, no credential, and no
endpoint. Configure a credential only for a protected proxy. All explicit endpoints pass structural
URL validation; applications accepting untrusted configuration should also supply an
`EndpointPolicy` that restricts schemes, hosts, and resolved network ranges.

## Examples

The examples compile as `armillae-llm-rig` example targets:

```sh
cargo run -p armillae-llm-rig --example simple_completion
cargo run -p armillae-llm-rig --example streaming
cargo run -p armillae-llm-rig --example manual_tool_flow
```

`manual_tool_flow` shows the ownership boundary directly: the first model call returns ToolCalls,
the application executes each one through `ToolRegistry`, appends ToolResults to history, and makes
the second model call itself.

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

The release gate is frozen to `openai/gpt-4.1-mini`, `deepseek/deepseek-chat`,
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
