# Armillae

[English](https://github.com/mmstudio-games/armillae/blob/main/README.md)

Armillae 是一个使用 Rust 构建的 Provider 无关 LLM 调用与类型安全 Tool 执行基础设施。它面向
Agentic 叙事系统、TRPG 运行时和大世界模拟引擎，同时将第一阶段保持为小而清晰、可自由组合的
底层能力。

## 当前状态

Armillae 目前处于 alpha 阶段。第一阶段已经提供：

- Provider 无关的消息、Completion、Tool、Usage 与 Streaming 协议；
- 一次只执行一个 Model Call、且不依赖具体异步运行时的 `LlmBridge`；
- 类型安全的 Tool 定义、注册与单次 ToolCall 执行；
- 确定性的 Mock 与共享 Bridge 合约测试；
- OpenAI、通用 OpenAI-compatible、DeepSeek、MiniMax 和 Moonshot 的 Rig Adapter；
- 上述已实现 Provider 的非流式及流式文本和 ToolCall 支持。

Anthropic、Ollama Adapter、可观测性、示例和剩余发布文档仍属于第一阶段待完成工作。Turn
Runner、自动 Tool Loop、完整 Agent、Memory、Embedding、Vector Store 与 RAG 明确不属于当前
范围。

## Crate

| Crate | 职责 |
|---|---|
| `armillae-core` | Provider 无关的消息、Completion、Tool、Usage 与流式事件 |
| `armillae-llm` | Bridge trait、能力、配置、Secret、错误、Factory 与 Mock |
| `armillae-tools` | 类型安全 Tool、Context、Registry 与单次执行 |
| `armillae-llm-rig` | 与公共协议隔离的 Rig Provider Adapter |

核心边界很简单：Bridge 只完成一次模型调用，Tool Executor 只执行一次 ToolCall；是否继续调用
模型由下游代码决定。

## 开发

Workspace 使用 stable Rust 和 Rust 2024 edition。执行与 CI 相同的离线检查：

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --all-features --locked
cargo test --workspace --all-targets --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
```

Live Provider 测试默认 ignored，只有在明确提供测试凭证时才可运行。修改前请阅读
[CONTRIBUTING.md](https://github.com/mmstudio-games/armillae/blob/main/CONTRIBUTING.md)，第一阶段技术事实以
[docs/DESIGN.md](https://github.com/mmstudio-games/armillae/blob/main/docs/DESIGN.md) 为准。

## 许可证

Armillae 仅采用
[GNU Affero General Public License v3.0](https://github.com/mmstudio-games/armillae/blob/main/LICENSE)，
SPDX 标识为 `AGPL-3.0-only`。
