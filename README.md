# Armillae

[简体中文](https://github.com/mmstudio-games/armillae/blob/main/README.zh.md)

Armillae is a Rust foundation for provider-independent LLM calls and type-safe Tool execution.
It is being built for agentic narrative systems, TRPG runtimes, and world simulation engines,
while keeping its first phase deliberately small and composable.

## Status

Armillae is alpha software. The first phase currently provides:

- provider-independent message, completion, Tool, usage, and streaming protocols;
- a runtime-independent `LlmBridge` for exactly one model call;
- type-safe Tool authoring, registration, and exactly one ToolCall execution;
- deterministic mocks and shared Bridge contract tests;
- Rig adapters for OpenAI, generic OpenAI-compatible endpoints, DeepSeek, MiniMax, and Moonshot;
- non-streaming and streaming text and ToolCall support for the implemented providers.

Anthropic and Ollama adapters, observability, examples, and the remaining release documentation
are still planned for the first phase. Turn runners, automatic Tool loops, agents, memory,
embeddings, vector stores, and RAG are intentionally outside the current scope.

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
before making changes and [docs/DESIGN.md](https://github.com/mmstudio-games/armillae/blob/main/docs/DESIGN.md)
for the authoritative first-phase design.

## License

Armillae is licensed under the
[GNU Affero General Public License v3.0 only](https://github.com/mmstudio-games/armillae/blob/main/LICENSE),
identified by SPDX as `AGPL-3.0-only`.
