# Armillae

[English](https://github.com/mmstudio-games/armillae/blob/main/README.md)

Armillae 是一个面向 Agentic 叙事系统、TRPG 运行时和大世界模拟引擎的分层 Rust 生态。当前已
实现的底层能力提供 Provider 无关 LLM 调用与类型安全 Tool 执行，下一阶段的设计重心是位于其上
且独立演进的 Agentic 叙事运行时。

## 当前状态

Armillae 目前处于 alpha 阶段，四个基础 crate 继续使用 Semifold `alpha` 发布通道，以便公共协议
边界、Provider 兼容性和发布证据继续收敛。只有冻结 0.1 范围、清零已知重大链路缺陷、获得代表性
Live 与真实下游证据并经明确授权后才进入 beta；不会从 alpha 直接晋级 stable。已经实现的 LLM
基础设施提供：

- Provider 无关的消息、Completion、Tool、Usage 与 Streaming 协议；
- 一次只执行一个 Model Call、且不依赖具体异步运行时的 `LlmBridge`；
- 类型安全的 Tool 定义、注册与单次 ToolCall 执行；
- 确定性的 Mock 与共享 Bridge 合约测试；
- OpenAI、通用 OpenAI-compatible、DeepSeek、MiniMax、Moonshot、Anthropic 和 Ollama 的 Rig
  Adapter；
- 上述已实现 Provider 的非流式及流式文本和 ToolCall 支持。

OpenAI 协议基线在正式宣称全量支持前，仍需通过明确的 Provider/模型端到端场景矩阵。
Anthropic 原生 Messages Adapter 使用保守能力配置：请求必须提供 `max_output_tokens`；Rig 0.41
无法保留 `ToolResult.is_error = true`，因此 Adapter 会显式拒绝；Rig 已过滤的 Anthropic 原始
未知 SSE 事件不会暴露。Ollama 使用保守的原生能力配置；由于其线协议没有调用 ID，Adapter 会
生成 Armillae ToolCall ID，并在后续 ToolResult 请求中映射回工具名。配置、能力矩阵、示例、安全
边界和默认 ignored 的 Live 支持门禁见 [LLM Bridge 使用指南](docs/llm-bridge.md)。

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
[CONTRIBUTING.md](https://github.com/mmstudio-games/armillae/blob/main/CONTRIBUTING.md)，工程规范和
RFC 由贡献指南统一引导。

## 许可证

Armillae 仅采用
[GNU Affero General Public License v3.0](https://github.com/mmstudio-games/armillae/blob/main/LICENSE)，
SPDX 标识为 `AGPL-3.0-only`。
