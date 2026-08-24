# Changelog

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
