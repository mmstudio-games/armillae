---
armillae-llm-rig: "patch:refactor"
---

Isolate OpenAI and OpenAI-compatible validation, client construction, capabilities, and contract tests behind a private Provider module so additional Rig Providers can be added without expanding the Bridge factory.
