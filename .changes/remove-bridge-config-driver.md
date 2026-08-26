---
armillae-llm: "minor:refactor"
armillae-llm-rig: "patch:refactor"
---

Remove the redundant driver field from BridgeConfig and make the builder accept only provider and model, leaving runtime Factory selection to the host while retaining BridgeFactory::driver as Factory identity; migrate Rig routing, tests, examples, and documentation, and explicitly reject legacy serialized driver fields.
