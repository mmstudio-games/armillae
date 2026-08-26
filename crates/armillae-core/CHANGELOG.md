# Changelog

<!-- semifold:release version=0.1.0-alpha.1 -->
## v0.1.0-alpha.1

### Chores

- [`1d9a64c`](https://github.com/mmstudio-games/armillae/commit/1d9a64c5d5721d7230c0ef2d96e902e5f2afcce6): Keep all four foundational crates on the alpha release channel, withdraw the pending direct stable promotion, and define evidence-based beta and stable entry gates.
<!-- semifold:release:end -->

<!-- semifold:release version=0.1.0-alpha.0 -->
## v0.1.0-alpha.0

### New Features

- [`3c57f59`](https://github.com/mmstudio-games/armillae/commit/3c57f59625a25f25b17a0f3685200db1f990de11): Define the provider-independent LLM and tool protocol in armillae-core, including stable JSON wire formats and schema snapshots.

    Validate rig-core 0.41.0 low-level completion, tool calling, streaming assembly, and cancellation behavior with offline OpenAI and Anthropic fixtures.

- [`b8455dc`](https://github.com/mmstudio-games/armillae/commit/b8455dc05073ff790b8fdf876f1bdc6ff90fd494): Represent missing completion finish reasons without inference and distinguish them from explicit unknown Provider values.

    Add a non-empty, transparently serialized ToolCallId shared by tool calls, tool results, and streaming events while preserving Provider-issued IDs.
<!-- semifold:release:end -->
