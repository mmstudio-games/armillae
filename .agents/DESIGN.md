# Armillae 设计索引

> 状态：Active
> 更新日期：2026-08-27
> 作用：Armillae 生态的权威工程设计入口，不在本文件重复各子系统规范或 RFC

本目录服务于项目设计、实施和 Agent 协作；工程事实按成熟度分为已生效的 `specs/`、尚在决策
中的 `rfcs/`、实现差异 `todos/` 和技术验证证据 `spikes/`。根目录 `docs/` 保留给面向使用者的
安装、概念、指南与 API 文档，但当前公共接口尚未冻结且项目未进入稳定版，不创建或维护独立
用户指南。只有公共接口冻结且稳定版推进获得明确授权后，才能开始编写 `docs/` 用户文档。

## 1. 项目方向

Armillae 面向 Agentic 叙事、TRPG 运行时和大世界游戏引擎提供分层基础设施。项目不把模型
调用、Tool 执行、Agent 调度、叙事状态和世界状态压入同一个运行时；各层拥有独立协议、错误
语义和演进节奏，通过明确的单向依赖组合。

模拟推进、Clock、可替换 ECS 后端和 Module 边界已经由 RFC 0002 收敛，并转入
`armillae-simulate` Active Spec；具体持久化模型及其 RFC 暂缓，不属于 `armillae-simulate` 的
实现责任。LLM Bridge 第一阶段离线基线已经完成；真实 DeepSeek 多轮验证暴露出 ProviderData
只有响应保留、没有请求回放的非对称边界。用户于 2026-08-27 接受 RFC 0003，要求 Armillae
继续拥有 canonical LLM 协议，由 Adapter 对目标 Provider 做双向投影，并由独立 LLM Router
在显式策略下提供模型 fallback。所有已支持 Adapter 的直接 Bridge projection 已完成离线实现
与 Mock HTTP 合约；Router 和授权 Live 场景矩阵仍待完成，因此仍不宣称“全量兼容所有模型”。
真实 Provider 验证继续暴露此前离线门禁未覆盖的重大设计与链路问题，基础 crate 因此保持
`alpha` 发布通道。进入 `beta` 的依据是 0.1 范围和公共协议基本冻结、重大链路缺陷清零并获得
代表性 Live 与真实下游证据，而不是“离线实现完成”；稳定版必须经过至少一个 beta 稳定周期，
不得从当前 alpha 直接晋级。

## 2. 分层与依赖方向

```text
叙事应用 / TRPG / 世界引擎
              │
              ▼
Agentic 叙事运行时                 Discovery
  ├── 生命周期与执行推进
  ├── Agent 行为与上下文组织
  ├── 组合：模拟基础设施            Active Spec；未实现
  │     ├── armillae-simulate       后端中立的执行、Clock 与 Module 契约
  │     └── armillae-simulate-bevy  首个 ECS 后端适配
  ├── 组合：状态与持久化            RFC 暂缓；独立于 simulate
  ├── 可选：LlmRouter              RFC 0003 Accepted；组合多个 LlmBridge
  │     └── LlmBridge              一次 Provider Model Call；canonical 协议投影
├── 可选：直接使用 LlmBridge      Provider projection 离线合约已完成
  ├── 可选：ToolExecutor           单次执行边界已实现
  │     └── armillae-tools-macros  函数式 Tool 声明宏；仅生成现有 Tool 契约
  └── 副作用治理
```

依赖只能从上层指向下层。`LlmBridge` 仍只执行一次 Provider Model Call，`ToolExecutor` 仍只
执行一次 `ToolCall -> ToolResult`；它们不持有 Agentic 运行时，也不负责推进叙事、Turn 或
世界状态。`LlmRouter` 可以按宿主显式提供的候选顺序和 fallback 策略执行一个或多个 Bridge
attempt，但不执行 Tool、不维护 Conversation Memory，也不改变 canonical request。运行时可以
直接使用 Bridge 或组合 Router；是否以及何时发起一次逻辑 LLM 请求仍由运行时或应用决定。

## 3. 权威工程文档

| 文档 | 类型与状态 | 权威范围 |
|---|---|---|
| [LLM Bridge、Router 与 Tool Executor](specs/llm-bridge.md) | Active Spec | Canonical 消息与 Completion 协议、Bridge、Router、Tool、Provider projection、Streaming、安全与合约测试 |
| [Simulate](specs/simulate.md) | Active Spec | 公共 API 与 JSON 协议、显式执行与推进、Clock、Module、后端契约、Bevy-native API 和验收门禁 |
| [RFC 0001：Agentic 叙事运行时](rfcs/0001-agentic-runtime.md) | Draft RFC | 运行时目标、分层边界、待冻结的领域模型与设计工作流 |
| [RFC 0002：Simulate 与可替换 ECS 后端](rfcs/0002-simulate.md) | Accepted RFC | Simulate 命名、责任、推进所有权、Clock、Module 与可替换后端决策 |
| [RFC 0003：LLM canonical 投影与模型 fallback](rfcs/0003-llm-projection-fallback.md) | Accepted RFC | Provider 双向投影、兼容性事实、候选路由与 fallback 边界 |
| [rig-core 0.41.0 Spike](spikes/rig-core-0.41.0.md) | Completed Spike | Rig 低层可行性证据、限制与锁定版本依据 |

实现前必须先在本索引中找到对应的 Active Spec 或已接受 RFC，再从 [TODO 索引](TODO.md)
定位实施清单。跨子系统变更先更新本入口中的依赖和责任边界，再更新相关 Spec 或 RFC，最后
更新实现差异清单与代码。Draft RFC 不构成实现授权。

## 4. 当前工作顺序

1. 按 RFC 0003 和 LLM Bridge Active Spec 先完成所有已支持 Adapter 的直接 Bridge Provider
   projection：同 Provider 回放已知私有数据，跨 Provider 只生成目标 wire projection，不修改
   canonical 数据，并让调用方取得结构化 compatibility facts。Rig 响应中不含 ID、签名、
   密文、redacted data、summary 或非空文本的纯空 reasoning，必须在共享 canonical 响应边界
   归一化为缺席，不能进入 history 并阻断后续请求；未知、带状态或其他有语义的私有数据不得
   使用该规则。
2. 通过直接 Bridge 合约与 Live 回归证明 projection 闭环后，再实现 host-owned fallback Router；
   Router 只复用 Adapter projection，不成为单 Provider 正常工作的前置条件。
3. 重新执行 DeepSeek 多轮与 Tool continuation Live 验证，再完成默认 ignored 的 OpenAI 协议
   Live 场景门禁。没有真实凭证时只交付可复现门禁，不伪造 Live 通过证据。
4. 所有基础 crate 保持 alpha；冻结 0.1 范围（包括 Router 是否纳入）、清零已知重大链路问题、
   完成安全/发布审计、代表性 Live 和至少一个真实下游验证后，才单独决策是否进入 beta。
   稳定版必须在 beta 中证明兼容性承诺可执行，不能仅凭离线测试或功能清单完成度晋级。
5. 按已冻结的 Simulate 公共契约完成 Bevy P0 Spike，再实现共享后端合约测试；在 Spike 编译
   验证精确 Bevy 版本、Features 和错误边界前不创建产品 crate。
6. 使用 Simulate 的已确认边界继续完成 Agentic 叙事运行时 RFC，不让运行时替用户决定 Agent、
   Tool 或 Simulation Driver 的调度策略。
7. 状态与持久化继续作为独立子系统保留，但在用户重新启动该方向前不创建 RFC、Spec、crate
   或持久化 Schema。
8. 冻结运行时与 LLM/Tool 等可选能力的依赖边界及端到端验收标准。

Anthropic 与 Ollama 继续使用精确锁定的 Rig 0.41.0 和既有 Bridge 合约，不为 Rig 已过滤的原始
未知 SSE/NDJSON 数据引入自有传输层；Driver 未暴露的事实作为显式兼容限制记录。

## 5. 变更规则

- 已生效子系统协议变化：先更新对应 Active Spec，再更新 `todos/` 中对应清单和实现。
- 尚未确认的架构提案：先进入 `rfcs/` 并按 [RFC 工作流](rfcs/README.md) 推进；Draft RFC
  不得直接产生实施任务。
- 跨层责任、依赖方向或生态范围变化：先更新本索引，再更新所有受影响的 Spec 或 RFC。
- `TODO.md` 只做全项目索引；`todos/*.md` 分别记录已确认设计与当前实现之间的差异。
- 未冻结的运行时问题必须显式保留在 Draft RFC 中，不得通过代码、测试或实施清单偷渡为
  既定设计。
- LLM Bridge 暂停扩展不等于废弃；运行时不得复制或绕过已经稳定的公共协议。
