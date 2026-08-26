# RFC 0001：Armillae Agentic 叙事运行时

> 状态：Draft
> 更新日期：2026-08-25
> 设计入口：[Armillae 设计索引](../DESIGN.md)
> 下层能力边界：[LLM Bridge、Router 与 Tool Executor Spec](../specs/llm-bridge.md)
> LLM 路由边界：[RFC 0003：LLM canonical 投影与模型 fallback](0003-llm-projection-fallback.md)
> 模拟基础设施：[RFC 0002：Simulate 与可替换 ECS 后端](0002-simulate.md)
> 生效规范：[Armillae Simulate Spec](../specs/simulate.md)

本 RFC 处于 Discovery 阶段，尚未冻结实施方案。它记录待确认的问题、候选边界和设计工作流；
在状态变为 Accepted 或 Active 前，不构成实现授权。状态含义与推进规则见
[RFC 工作流](README.md)。

## 1. 已确认方向

Armillae 的长期目标是提供面向 Agentic 叙事的通用运行时，可用于叙事引擎、TRPG 运行时和
大世界游戏引擎。运行时关注上下文组织、叙事状态、世界状态、执行推进、工具调度、持久化、
回放和更高层 Agent 行为。

运行时属于 Armillae 生态，但不是 LLM Bridge 的延伸：

- 运行时可以把 `LlmBridge`、`LlmRouter` 和 `ToolExecutor` 作为可选能力使用；
- 运行时不能要求 Bridge 自动执行 Tool、维护 Memory 或推进 Turn；
- Bridge 和 Tool 层不能反向依赖运行时；
- 叙事与世界运行不应以某个 Provider、模型 SDK 或 Rig 类型作为持久化协议；
- 是否调用 LLM 以及如何处理多个 ToolCall，由运行时或其上层策略决定。
- Provider projection、Candidate attempt 和 model fallback 由 RFC 0003 的运行时无关 LLM
  基础设施承担，运行时不得复制 Provider codec 或把 Provider wire 数据作为状态协议。

当前阶段只整理场景和冻结设计，不授权创建运行时 crate 或实现功能。

Simulation、ECS 工作世界、Clock、Module 和 Backend 边界由已接受的
[RFC 0002](0002-simulate.md) 及其 [Active Spec](../specs/simulate.md) 承担。本 RFC 继续拥有
Agent 生命周期、上下文、可选 LLM/Tool 组合与跨子系统副作用边界，不得让 Agent Harness
反向决定 Simulate 的底层协议。权威状态与持久化属于未来独立子系统；当前不由 RFC 0001 或
RFC 0002 提前定义。

## 2. 与 LLM Bridge 的关系

LLM Bridge 提供一次无状态 Model Call；Tool Executor 提供一次显式 Tool Execution。运行时需要
在自己的领域模型中决定调用时机、状态推进、失败恢复和副作用边界。两者的关系必须保持为
单向组合：

```text
Agentic 叙事运行时
  ├── 可选：直接使用 LlmBridge
  ├── 可选：LlmRouter
  │     └── 一个或多个 LlmBridge
  └── 可选：ToolExecutor
```

这不预先决定运行时必须存在名为 `Turn` 的核心类型，也不预先决定自动 Tool Loop、Memory、
RAG 或工作流引擎的实现方式。运行时可以提供 Candidate 列表和 fallback policy，但 Router 的
canonical projection、错误分类、attempt report、流式切换和取消语义由 RFC 0003 与 LLM Spec
统一定义，不随叙事领域模型分叉。

## 3. 设计输入

后续设计必须从可执行的端到端叙事场景出发，而不是直接从 crate 或 trait 出发。每个场景至少
要说明：参与者、输入、状态所有者、允许的决策、外部副作用、失败点、持久化边界、恢复方式和
可观测结果。

已知需要覆盖的能力域来自项目长期目标：

- 叙事上下文的组织与裁剪；
- 叙事状态和世界状态的所有权及一致性；
- 一次执行推进的生命周期；
- Agent 行为、调度和人工控制边界；
- Tool 或其它外部副作用的授权与结果归档；
- 持久化、存档、回放和确定性边界；
- 长期运行中的错误恢复、取消和可观测性。

这些是待设计的问题域，不代表已经选择具体的数据结构、调度算法或存储方案。

## 4. 待冻结的核心决策

以下问题在进入实现前都必须得到明确答案：

1. 运行时最小执行单元是 Action、Step、Turn、Scene、Session 还是其它概念，各自生命周期如何
   组合？
2. 叙事状态、应用状态、Agent 私有状态和派生上下文分别由谁拥有；未来状态 RFC 应由哪个
   子系统承接，而不把持久化塞入 Simulate？
3. 未来状态子系统冻结提交、存档、分支和重放语义后，Agent 生命周期如何组合而不复制它们？
4. 多 Agent 的推进、暂停、取消、优先级和公平性由哪一层负责？
5. Tool 与外部系统副作用如何审批、幂等、重试、补偿和审计？
6. LLM Router 已按策略耗尽 Candidate、输出无效或执行被中断时，运行时如何保持可恢复状态？
7. Memory、Embedding、Vector Store 和 RAG 是运行时核心协议、可选能力还是应用层策略？
8. 端到端验收以哪些叙事场景、确定性要求、延迟与可观测事实为准？

在这些边界确认前，不定义公共 Rust API、持久化 Schema、自动 Tool Loop 或 crate 拆分。

## 5. 文档阶段里程碑

1. 场景目录：收集最小叙事循环、多人/多 Agent、世界状态变化、存档回放和失败恢复场景。
2. 领域词汇表：冻结 Action、Turn、Scene、Session、World、Agent、State、Event 等术语。
3. 状态与生命周期：明确权威状态、状态转换、提交点、取消和恢复。
4. 执行与副作用：明确调度、审批、Tool 使用、并发和错误处理责任。
5. 持久化与回放：明确 Schema、版本、事件顺序、快照和兼容策略。
6. 安全与可观测性：明确 Secret、内容、审计、指标和调试边界。
7. 验收与实施拆分：用端到端场景验证设计后，再创建对应实施清单、crate 计划和工程估算。

## 6. 当前非目标

- 在场景和领域模型冻结前实现运行时代码；
- 为了运行时方便而改变 `LlmBridge` 或 `ToolExecutor` 的单次调用边界；
- 把某个 LLM Provider、Rig Agent、向量数据库或工作流框架设为运行时核心协议；
- 在没有状态与副作用语义时先实现自动 Tool Loop；
- 将“Agentic”解释为模型可以不受约束地执行外部副作用。
