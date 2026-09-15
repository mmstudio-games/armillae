---
armillae-core: "minor:feat"
armillae-llm: "minor:feat"
armillae-llm-rig: "minor:feat"
---

Add explicit Thinking and ReasoningEffort generation controls with Bridge defaults, per-call overrides and ProviderDefault reset. Publish Adapter reasoning capabilities and reject unsupported modes, effort levels and conflicting settings before HTTP calls. All seven Provider entries share the complete, stream and projection path; Anthropic schema and effort share a single output_config.

Remove the legacy OpenAI provider_options and request-extension reasoning_effort entry points. Rust callers must update exhaustive GenerationOptions and BridgeCapabilities literals. The alpha API does not retain a compatibility layer. Live gates remain opt-in and ignored by default.
