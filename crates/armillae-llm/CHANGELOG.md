# Changelog

<!-- semifold:release version=0.1.0-alpha.0 -->
## v0.1.0-alpha.0

### New Features

- [`75ac414`](https://github.com/mmstudio-games/armillae/commit/75ac414011b60fbc632cf19b8f1f68aabe77d616): Add the runtime-independent LlmBridge contract, granular capability preflight, and normalized Bridge error classification.

    Provide validated TOML, JSON, and builder configuration, safe Secret resolution, optional endpoint policy, and an object-safe Bridge factory.

- [`93e4679`](https://github.com/mmstudio-games/armillae/commit/93e4679564555c68ba8f7794feca83811d270480): Add the opt-in MockBridge with fixed and scripted responses, semantic streaming helpers, deterministic error injection, and safe request recording.

    Provide runtime-independent Bridge contract verification that can be reused by Mock and real adapters.

- [`b8455dc`](https://github.com/mmstudio-games/armillae/commit/b8455dc05073ff790b8fdf876f1bdc6ff90fd494): Represent missing completion finish reasons without inference and distinguish them from explicit unknown Provider values.

    Add a non-empty, transparently serialized ToolCallId shared by tool calls, tool results, and streaming events while preserving Provider-issued IDs.


### Dependencies

- Update armillae-core to 0.1.0-alpha.0.
<!-- semifold:release:end -->
