# Armillae 设计索引

> 状态：Active
> 更新日期：2026-08-26
> 作用：Armillae 生态的权威工程设计入口，不在本文件重复各子系统规范或 RFC

本目录服务于项目设计、实施和 Agent 协作；面向使用者的安装、概念、指南与 API 文档统一放在
根目录 `docs/`。工程事实按成熟度分为已生效的 `specs/`、尚在决策中的 `rfcs/`、实现差异
`todos/` 和技术验证证据 `spikes/`。

## 1. 项目方向

Armillae 面向 Agentic 叙事、TRPG 运行时和大世界游戏引擎提供分层基础设施。项目不把模型
调用、Tool 执行、Agent 调度、叙事状态和世界状态压入同一个运行时；各层拥有独立协议、错误
语义和演进节奏，通过明确的单向依赖组合。

当前主仓开发重心从继续扩展 LLM Provider 转向 Agentic 叙事基础设施。模拟推进、Clock、可替换
ECS 后端和 Module 边界已经由 RFC 0002 收敛，并转入 `armillae-simulate` Active Spec；具体
持久化模型及其 RFC 暂缓，不属于 `armillae-simulate` 的实现责任。LLM Bridge 已完成 OpenAI
协议主流 Provider 的公共协议、非流式、流式和 Tool Calling 基线，但在完成端到端场景矩阵前，
不宣称“全量支持所有 OpenAI 协议主流模型”。Anthropic P6 已在隔离分支完成，不改变主仓优先
推进 Simulate 的顺序，也不阻塞运行时设计；Ollama 及其它 Bridge 完善项继续暂停。

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
  ├── 可选：LlmBridge              OpenAI 协议基线维护中；Anthropic P6 已完成
  ├── 可选：ToolExecutor           单次执行边界已实现
  └── 副作用治理
```

依赖只能从上层指向下层。`LlmBridge` 仍只执行一次 Model Call，`ToolExecutor` 仍只执行一次
`ToolCall -> ToolResult`；它们不持有 Agentic 运行时，也不负责推进叙事、Turn 或世界状态。
运行时是否以及何时调用模型，是运行时设计问题，不是 Bridge 协议的一部分。

## 3. 权威工程文档

| 文档 | 类型与状态 | 权威范围 |
|---|---|---|
| [LLM Bridge 与 Tool Executor](specs/llm-bridge.md) | Active Spec | 公共消息与 Completion 协议、Bridge、Tool、Provider Adapter、Streaming、安全与合约测试 |
| [Simulate](specs/simulate.md) | Active Spec | 公共 API 与 JSON 协议、显式执行与推进、Clock、Module、后端契约、Bevy-native API 和验收门禁 |
| [RFC 0001：Agentic 叙事运行时](rfcs/0001-agentic-runtime.md) | Draft RFC | 运行时目标、分层边界、待冻结的领域模型与设计工作流 |
| [RFC 0002：Simulate 与可替换 ECS 后端](rfcs/0002-simulate.md) | Accepted RFC | Simulate 命名、责任、推进所有权、Clock、Module 与可替换后端决策 |
| [rig-core 0.41.0 Spike](spikes/rig-core-0.41.0.md) | Completed Spike | Rig 低层可行性证据、限制与锁定版本依据 |

实现前必须先在本索引中找到对应的 Active Spec 或已接受 RFC，再从 [TODO 索引](TODO.md)
定位实施清单。跨子系统变更先更新本入口中的依赖和责任边界，再更新相关 Spec 或 RFC，最后
更新实现差异清单与代码。Draft RFC 不构成实现授权。

## 4. 当前工作顺序

1. 按已冻结的 Simulate 公共契约完成 Bevy P0 Spike，再实现共享后端合约测试；在 Spike 编译
   验证精确 Bevy 版本、Features 和错误边界前不创建产品 crate。
2. 为 OpenAI 协议支持定义并执行端到端场景矩阵，形成可审计的支持声明；该工作只验证既有
   Bridge，不继续扩大 OpenAI-compatible Provider 范围。
3. 使用 Simulate 的已确认边界继续完成 Agentic 叙事运行时 RFC，不让运行时替用户决定 Agent、
   Tool 或 Simulation Driver 的调度策略。
4. 状态与持久化继续作为独立子系统保留，但在用户重新启动该方向前不创建 RFC、Spec、crate
   或持久化 Schema。
5. 冻结运行时与 LLM/Tool 等可选能力的依赖边界及端到端验收标准。

Anthropic P6 继续使用精确锁定的 Rig 0.41.0 和既有 Bridge 合约，不为 Rig 已过滤的原始未知
Anthropic SSE 事件引入自有传输层；该已完成增量不占用上述主仓工作顺序。

## 5. 变更规则

- 已生效子系统协议变化：先更新对应 Active Spec，再更新 `todos/` 中对应清单和实现。
- 尚未确认的架构提案：先进入 `rfcs/` 并按 [RFC 工作流](rfcs/README.md) 推进；Draft RFC
  不得直接产生实施任务。
- 跨层责任、依赖方向或生态范围变化：先更新本索引，再更新所有受影响的 Spec 或 RFC。
- `TODO.md` 只做全项目索引；`todos/*.md` 分别记录已确认设计与当前实现之间的差异。
- 未冻结的运行时问题必须显式保留在 Draft RFC 中，不得通过代码、测试或实施清单偷渡为
  既定设计。
- LLM Bridge 暂停扩展不等于废弃；运行时不得复制或绕过已经稳定的公共协议。
