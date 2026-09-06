---
armillae-llm: "patch:fix"
armillae-llm-rig: "patch:fix"
---

Preserve HTTP status from typed client error chains and expose safe transport failure categories and OS error codes. Distinguish authentication, permissions, rate limits, HTTP timeouts, connection failures, and client timeouts without retaining raw error text or URLs.
