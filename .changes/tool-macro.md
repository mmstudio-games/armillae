---
armillae-tools-macros: "patch:feat"
---

Add a #[tool] attribute macro that turns synchronous and asynchronous functions into Armillae Tool implementations with generated argument schemas, typed errors, explicit ToolContext injection, and either centralized or parameter-local argument descriptions.

Keep the macro as a separate proc-macro crate that depends on armillae-tools without changing ToolExecutor or Bridge execution boundaries.
