# Changelog

<!-- semifold:release version=0.1.0-alpha.1 -->
## v0.1.0-alpha.1

### Bug Fixes

- [`e2ddb63`](https://github.com/mmstudio-games/armillae/commit/e2ddb63c72c77af252b5bf7fb9e89c0b3da359d8): Normalize stateless empty reasoning at the shared Rig canonical boundary so provider responses cannot poison later projections while signed, identified, encrypted, redacted, summarized, and non-empty reasoning remains replayable.
- [`484a0b8`](https://github.com/mmstudio-games/armillae/commit/484a0b8efe230a24d330fa9bc9349f1ee262f819): Replace the deprecated deepseek-chat compatibility alias with the official deepseek-v4-flash model ID across the frozen Live matrix, interactive example, Live harness, offline fixtures, and user documentation.

### Chores

- [`ba93d85`](https://github.com/mmstudio-games/armillae/commit/ba93d85a938838fc2bd4d56e72b42086f657c18a): Add an interactive DeepSeek conversation example that resolves DEEPSEEK_API_KEY, preserves System/User/Assistant history, and performs one explicit Bridge call per user turn.
- [`1e02fe4`](https://github.com/mmstudio-games/armillae/commit/1e02fe4ba29503276f3e7046eb05bbb8433b615c): Extend the interactive DeepSeek conversation example with a deterministic typed Tool, ToolRegistry execution, and one explicit ToolResult continuation call, while preserving the ordinary single-call text path and documenting the host-owned dispatch flow.
- [`e33e47e`](https://github.com/mmstudio-games/armillae/commit/e33e47e3e7a0941ad7f13609274d4b0db932a429): Add runnable structured-output and local Ollama examples, and expand the LLM Bridge guide with non-streaming, streaming, strict JSON Schema, manual ToolCall continuation, and local-provider usage patterns.
- [`1d9a64c`](https://github.com/mmstudio-games/armillae/commit/1d9a64c5d5721d7230c0ef2d96e902e5f2afcce6): Keep all four foundational crates on the alpha release channel, withdraw the pending direct stable promotion, and define evidence-based beta and stable entry gates.

### New Features

- [`85648aa`](https://github.com/mmstudio-games/armillae/commit/85648aa8d34f553d43bbcb16f67a8300002be4da): Finish the first-phase Rig Bridge with native Ollama completion and NDJSON streaming, safe structured tracing, examples, documentation, and an ignored OpenAI-protocol Live support gate.
- [`da50071`](https://github.com/mmstudio-games/armillae/commit/da500715b77c0804eb3595f600ab7a35e3fa5aee): Add the native Anthropic Messages Provider through Rig 0.41 with conservative capability preflight, non-streaming and streaming text, ToolCall, ToolResult, reasoning, usage, finish-reason, and error normalization. The adapter rejects ToolResult error flags and any structured-output schema Rig would rewrite semantically, while documenting Rig-filtered unknown SSE events as a driver boundary. ([#3](https://github.com/mmstudio-games/armillae/pull/3) by @fu050409)
- [`9cda45d`](https://github.com/mmstudio-games/armillae/commit/9cda45d98db9a94d5364994ea45f59bc82e6df11): Add side-effect-free target Provider projection reports, same-Provider reasoning and ToolCall metadata replay, explicit cross-Provider not-forwarded facts, and structured projection failures across every supported Rig adapter.

### Refactors

- [`a19c5ef`](https://github.com/mmstudio-games/armillae/commit/a19c5ef547afcca6824c4354f06b70e6162cccbd): Remove the redundant driver field from BridgeConfig and make the builder accept only provider and model, leaving runtime Factory selection to the host while retaining BridgeFactory::driver as Factory identity; migrate Rig routing, tests, examples, and documentation, and explicitly reject legacy serialized driver fields.
- [`9eee1c2`](https://github.com/mmstudio-games/armillae/commit/9eee1c2618f864daf2953fad6f901da1a19b9b32): Replace positional optional arguments on BridgeConfig::resolve with a zero-argument common path and a private-field BridgeResolveContext for composing host SecretResolver and EndpointPolicy hooks; migrate adapters, tests, examples, and documentation to the clearer API.
<!-- semifold:release:end -->

<!-- semifold:release version=0.1.0-alpha.0 -->
## v0.1.0-alpha.0

### New Features

- [`79ae302`](https://github.com/mmstudio-games/armillae/commit/79ae30285598dd11528f4d20afd65d6791ef3286): Add first-class non-streaming OpenAI-compatible completion Providers for DeepSeek, MiniMax, and Moonshot with native Rig clients, conservative capability profiles, Provider-specific response normalization, and offline wire contract coverage.
- [`3c57f59`](https://github.com/mmstudio-games/armillae/commit/3c57f59625a25f25b17a0f3685200db1f990de11): Define the provider-independent LLM and tool protocol in armillae-core, including stable JSON wire formats and schema snapshots.

    Validate rig-core 0.41.0 low-level completion, tool calling, streaming assembly, and cancellation behavior with offline OpenAI and Anthropic fixtures.

- [`33d3433`](https://github.com/mmstudio-games/armillae/commit/33d3433a333a58beb1296f4e3f0e760c69e29673): Add unified semantic streaming for OpenAI, OpenAI-compatible, DeepSeek, MiniMax, and Moonshot, including text, reasoning, interleaved tool calls, usage, interruption handling, and drop cancellation.
- [`4990699`](https://github.com/mmstudio-games/armillae/commit/4990699c8c2d59be897aeab92a17af8bdc2d8ae4): Add the non-streaming generic RigBridge with explicit request mapping, response normalization, capability preflight, and safe Provider error conversion.

    Provide the OpenAI and OpenAI-compatible RigBridgeFactory with validated credentials, custom endpoints, generation defaults, structured output, ToolCall round trips, and reusable Bridge contract coverage.

- [`b8455dc`](https://github.com/mmstudio-games/armillae/commit/b8455dc05073ff790b8fdf876f1bdc6ff90fd494): Represent missing completion finish reasons without inference and distinguish them from explicit unknown Provider values.

    Add a non-empty, transparently serialized ToolCallId shared by tool calls, tool results, and streaming events while preserving Provider-issued IDs.


### Refactors

- [`381ba41`](https://github.com/mmstudio-games/armillae/commit/381ba41cb788242ac821696eb3d0cf856abc5bf5): Isolate OpenAI and OpenAI-compatible validation, client construction, capabilities, and contract tests behind a private Provider module so additional Rig Providers can be added without expanding the Bridge factory.

### Dependencies

- Update armillae-core to 0.1.0-alpha.0.
- Update armillae-llm to 0.1.0-alpha.0.
<!-- semifold:release:end -->
