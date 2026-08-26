---
armillae-llm-rig: "minor:feat"
---

Add the native Anthropic Messages Provider through Rig 0.41 with conservative capability preflight, non-streaming and streaming text, ToolCall, ToolResult, reasoning, usage, finish-reason, and error normalization. The adapter rejects ToolResult error flags and any structured-output schema Rig would rewrite semantically, while documenting Rig-filtered unknown SSE events as a driver boundary.
