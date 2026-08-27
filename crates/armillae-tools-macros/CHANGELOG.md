# Changelog

<!-- semifold:release version=0.1.0-alpha.1 -->
## v0.1.0-alpha.1

### New Features

- [`193ae09`](https://github.com/mmstudio-games/armillae/commit/193ae099185a13cda0bdc09435219f5c3b0e5e5a): Add a #[tool] attribute macro that turns synchronous and asynchronous functions into Armillae Tool implementations with generated argument schemas, typed errors, explicit ToolContext injection, and either centralized or parameter-local argument descriptions.

    Keep the macro as a separate proc-macro crate that depends on armillae-tools without changing ToolExecutor or Bridge execution boundaries.
<!-- semifold:release:end -->
