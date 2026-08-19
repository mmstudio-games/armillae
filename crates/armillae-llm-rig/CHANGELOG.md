# Changelog

<!-- semifold:release version=0.1.0-alpha.0 -->
## v0.1.0-alpha.0

### New Features

- [`3c57f59`](https://github.com/mmstudio-games/armillae/commit/3c57f59625a25f25b17a0f3685200db1f990de11): Define the provider-independent LLM and tool protocol in armillae-core, including stable JSON wire formats and schema snapshots.

    Validate rig-core 0.41.0 low-level completion, tool calling, streaming assembly, and cancellation behavior with offline OpenAI and Anthropic fixtures.

- [`4990699`](https://github.com/mmstudio-games/armillae/commit/4990699c8c2d59be897aeab92a17af8bdc2d8ae4): Add the non-streaming generic RigBridge with explicit request mapping, response normalization, capability preflight, and safe Provider error conversion.

    Provide the OpenAI and OpenAI-compatible RigBridgeFactory with validated credentials, custom endpoints, generation defaults, structured output, ToolCall round trips, and reusable Bridge contract coverage.


### Dependencies

- Update armillae-core to 0.1.0-alpha.0.
- Update armillae-llm to 0.1.0-alpha.0.
<!-- semifold:release:end -->
