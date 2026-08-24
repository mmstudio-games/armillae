# Armillae 设计索引

> 状态：Active
> 更新日期：2026-08-24
> 作用：Armillae 生态的权威设计入口，不在本文件重复各子系统协议

## 1. 项目方向

Armillae 面向 Agentic 叙事、TRPG 运行时和大世界游戏引擎提供分层基础设施。项目不把模型
调用、Tool 执行、Agent 调度、叙事状态和世界状态压入同一个运行时；各层拥有独立协议、错误
语义和演进节奏，通过明确的单向依赖组合。

当前开发重心从继续扩展 LLM Provider 转向 Agentic 叙事运行时的设计。LLM Bridge 已完成
OpenAI 协议主流 Provider 的公共协议、非流式、流式和 Tool Calling 基线，但在完成端到端场景
矩阵前，不宣称“全量支持所有 OpenAI 协议主流模型”。Anthropic、Ollama 及其它 Bridge 完善项
进入暂停队列，不阻塞运行时设计。

## 2. 分层与依赖方向

```text
叙事应用 / TRPG / 世界引擎
              │
              ▼
Agentic 叙事运行时                 设计中
  ├── 生命周期与执行推进
  ├── 叙事状态与世界状态边界
  ├── 持久化、回放与副作用治理
  └── Agent 行为与上下文组织
              │ 可选能力依赖
              ▼
能力与基础设施层
  ├── LlmBridge                    OpenAI 协议基线维护中
  └── ToolExecutor                 单次执行边界已实现
```

依赖只能从上层指向下层。`LlmBridge` 仍只执行一次 Model Call，`ToolExecutor` 仍只执行一次
`ToolCall -> ToolResult`；它们不持有 Agentic 运行时，也不负责推进叙事、Turn 或世界状态。
运行时是否以及何时调用模型，是运行时设计问题，不是 Bridge 协议的一部分。

## 3. 权威设计文档

| 文档 | 状态 | 权威范围 |
|---|---|---|
| [LLM Bridge 与 Tool Executor](LLM_BRIDGE.md) | OpenAI 协议基线维护中 | 公共消息与 Completion 协议、Bridge、Tool、Provider Adapter、Streaming、安全与合约测试 |
| [Agentic 叙事运行时](AGENTIC_RUNTIME.md) | Discovery | 运行时目标、分层边界、待冻结的领域模型与设计工作流 |
| [rig-core 0.41.0 Spike](spikes/rig-core-0.41.0.md) | 已完成 | Rig 低层可行性证据、限制与锁定版本依据 |

实现前必须先在本索引中找到对应权威设计，再从根 [TODO 索引](../TODO.md) 定位实施清单。
跨子系统变更先更新本入口中的依赖和责任边界，再更新相关子系统设计，最后更新实现差异清单
与代码。

## 4. 当前工作顺序

1. 为 OpenAI 协议支持定义并执行端到端场景矩阵，形成可审计的支持声明；该工作只验证既有
   Bridge，不继续扩大 Provider 范围。
2. 完成 Agentic 叙事运行时的场景、术语、状态所有权和生命周期设计。
3. 冻结运行时与 LLM/Tool 等可选能力的依赖边界及端到端验收标准。
4. 设计确认后再建立运行时实施清单和 crate，不从待决问题直接推导代码。

## 5. 变更规则

- 子系统内部协议变化：先更新对应子系统设计，再更新 `todos/` 中对应清单和实现。
- 跨层责任、依赖方向或生态范围变化：先更新本索引，再更新所有受影响的子系统设计。
- 根 `TODO.md` 只做全项目索引；`todos/*.md` 分别记录已确认设计与当前实现之间的差异。
- 未冻结的运行时问题必须显式保留为待决策项，不得通过代码、测试或实施清单偷渡为既定设计。
- LLM Bridge 暂停扩展不等于废弃；运行时不得复制或绕过已经稳定的公共协议。
