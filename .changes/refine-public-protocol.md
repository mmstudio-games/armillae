---
armillae-core: "minor:feat"
armillae-llm: "patch:feat"
armillae-llm-rig: "patch:feat"
armillae-tools: "patch:feat"
---

Represent missing completion finish reasons without inference and distinguish them from explicit unknown Provider values.

Add a non-empty, transparently serialized ToolCallId shared by tool calls, tool results, and streaming events while preserving Provider-issued IDs.
