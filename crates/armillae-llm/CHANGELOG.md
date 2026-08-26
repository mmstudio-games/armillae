# Changelog

<!-- semifold:release version=0.1.0-alpha.1 -->
## v0.1.0-alpha.1

### Chores

- [`1d9a64c`](https://github.com/mmstudio-games/armillae/commit/1d9a64c5d5721d7230c0ef2d96e902e5f2afcce6): Keep all four foundational crates on the alpha release channel, withdraw the pending direct stable promotion, and define evidence-based beta and stable entry gates.
- [`ad8913d`](https://github.com/mmstudio-games/armillae/commit/ad8913ddbdd907493d85adb411f051de937b2928): Upgrade the TOML parser dependency to 1.1.4, retaining the existing configuration API while picking up upstream parser fixes and removing obsolete duplicate parser dependencies from the lockfile.

### New Features

- [`9cda45d`](https://github.com/mmstudio-games/armillae/commit/9cda45d98db9a94d5364994ea45f59bc82e6df11): Add side-effect-free target Provider projection reports, same-Provider reasoning and ToolCall metadata replay, explicit cross-Provider not-forwarded facts, and structured projection failures across every supported Rig adapter.

### Refactors

- [`a19c5ef`](https://github.com/mmstudio-games/armillae/commit/a19c5ef547afcca6824c4354f06b70e6162cccbd): Remove the redundant driver field from BridgeConfig and make the builder accept only provider and model, leaving runtime Factory selection to the host while retaining BridgeFactory::driver as Factory identity; migrate Rig routing, tests, examples, and documentation, and explicitly reject legacy serialized driver fields.
- [`9eee1c2`](https://github.com/mmstudio-games/armillae/commit/9eee1c2618f864daf2953fad6f901da1a19b9b32): Replace positional optional arguments on BridgeConfig::resolve with a zero-argument common path and a private-field BridgeResolveContext for composing host SecretResolver and EndpointPolicy hooks; migrate adapters, tests, examples, and documentation to the clearer API.
<!-- semifold:release:end -->

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
