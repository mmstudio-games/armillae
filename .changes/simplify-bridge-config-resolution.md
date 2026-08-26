---
armillae-llm: "minor:refactor"
armillae-llm-rig: "patch:refactor"
---

Replace positional optional arguments on BridgeConfig::resolve with a zero-argument common path and a private-field BridgeResolveContext for composing host SecretResolver and EndpointPolicy hooks; migrate adapters, tests, examples, and documentation to the clearer API.
