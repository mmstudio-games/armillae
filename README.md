# Armillae

[简体中文](https://github.com/mmstudio-games/armillae/blob/main/README.zh.md)

Armillae is a layered Rust ecosystem for agentic narrative systems, TRPG runtimes, and world
simulation engines. Its implemented foundation provides provider-independent LLM calls and
type-safe Tool execution; its next design focus is the Agentic narrative runtime above them.

## Status

Armillae is alpha software. The implemented LLM foundation currently provides:

- provider-independent message, completion, Tool, usage, and streaming protocols;
- a runtime-independent `LlmBridge` for exactly one model call;
- type-safe Tool authoring, registration, and exactly one ToolCall execution;
- deterministic mocks and shared Bridge contract tests;
- Rig adapters for OpenAI, generic OpenAI-compatible endpoints, DeepSeek, MiniMax, and Moonshot;
- non-streaming and streaming text and ToolCall support for the implemented providers.

This is the OpenAI-protocol baseline for mainstream compatible Providers. A formal full-support
claim remains gated on an explicit end-to-end Provider/model scenario matrix. Anthropic, Ollama,
and other Bridge expansion work is paused while the Agentic narrative runtime is designed as an
independent upper layer.

## Crates

| Crate | Responsibility |
|---|---|
| `armillae-core` | Provider-independent messages, completions, Tools, usage, and streaming events |
| `armillae-llm` | Bridge traits, capabilities, configuration, secrets, errors, factories, and mocks |
| `armillae-tools` | Type-safe Tools, context, registry, and single-call execution |
| `armillae-llm-rig` | Rig-backed Provider adapters isolated from the public protocol |

The central boundary is simple: a Bridge performs one model call, and a Tool Executor performs
one ToolCall. Whether another model call should happen is decided by downstream code.

## Development

The workspace uses stable Rust with the 2024 edition. Run the same offline checks used by CI:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features --locked
cargo test --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
```

Live Provider tests are ignored by default and must only be run with explicitly supplied test
credentials. See [CONTRIBUTING.md](https://github.com/mmstudio-games/armillae/blob/main/CONTRIBUTING.md)
before making changes. Engineering specifications and RFCs are linked from the contribution guide.

## License

Armillae is licensed under the
[GNU Affero General Public License v3.0 only](https://github.com/mmstudio-games/armillae/blob/main/LICENSE),
identified by SPDX as `AGPL-3.0-only`.
