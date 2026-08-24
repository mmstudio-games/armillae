# Changelog

<!-- semifold:release version=0.1.0-alpha.0 -->
## v0.1.0-alpha.0

### New Features

- [`81817a2`](https://github.com/mmstudio-games/armillae/commit/81817a2b16d1c02b510f037cab449452ce0cf612): Add type-safe Tool authoring, dynamic dispatch, runtime-independent execution, and normalized JSON or multi-content outputs.

    Provide a mutable ToolRegistry with stable definitions, structured errors, host-only typed context, and complete offline contract tests.

- [`b8455dc`](https://github.com/mmstudio-games/armillae/commit/b8455dc05073ff790b8fdf876f1bdc6ff90fd494): Represent missing completion finish reasons without inference and distinguish them from explicit unknown Provider values.

    Add a non-empty, transparently serialized ToolCallId shared by tool calls, tool results, and streaming events while preserving Provider-issued IDs.


### Dependencies

- Update armillae-core to 0.1.0-alpha.0.
<!-- semifold:release:end -->
