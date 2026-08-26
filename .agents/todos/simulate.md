# Armillae Simulate 实施清单

> 状态：Planned
> 最后核对：2026-08-26
> 需求来源：[Armillae Simulate Spec](../specs/simulate.md)
> 决策来源：[RFC 0002](../rfcs/0002-simulate.md)

本清单只记录 Active Spec 与当前实现之间的差异，不是独立需求来源。当前仓库尚无 Simulate
crate；用户本轮只授权文档工作，已勾选项只表示规范设计完成，产品实现均未开始。持久化不属于
本清单。

## P0：实施门禁

- [x] 在 Active Spec 中冻结首批 Rust 公共类型、JSON wire shape 和精确签名。
- [x] 定义同类型批量 Advance 的输入、无固定公共上限、顺序和失败边界。
- [x] 定义 Clock 预计算错误可恢复、System/Backend 错误使 Simulation Faulted 的精确契约。
- [ ] 完成 Bevy ECS Spike，记录精确版本、MSRV、最小 Features、Schedule、变更检测、显式
  Context、`Local<T>::FromWorld` 初始化时序、`SystemExecutionResult` pipe、redacting fallback、
  `World: Send`、`NonSend` 线程亲和性和故障边界证据。
- [x] 定义与 Bevy 无关的 `ScriptedSimulation` 和共享 `BackendContractFactory` 测试入口。

## P1：后端中立核心

- [ ] 使用 Cargo CLI 创建 `armillae-simulate` crate 并检查发布元数据。
- [ ] 实现 Backend Builder，以及 Active、Stopped 和 Faulted Simulation 生命周期。
- [ ] 实现 Module 描述、完整校验、原子注册、激活与冻结。
- [ ] 实现 ID/版本、Capabilities、Execute、Clock 与 Advance 的 Serde/Schema 协议。
- [ ] 实现 object-safe 同步 `Simulation`、结构化 Runtime 错误和状态转换。
- [ ] 实现用户 `Clock` Trait、同类型多 Clock Instance 和显式批量 Advance 输入。
- [ ] 实现 `testing` feature 下的 Scripted Test Double 与共享合约入口。
- [ ] 实现 Backend 能力预检与共享合约测试。
- [ ] 覆盖无隐式推进、无递归推进、唯一终止结果和故障状态测试。

## P2：Bevy Backend

- [ ] 使用 Cargo CLI 创建 `armillae-simulate-bevy` crate，并精确添加 Spike 锁定的
  `bevy_ecs` 版本。
- [ ] 实现 Bevy World、Module 注册和冻结 Schedule 图。
- [ ] 实现 Execute 与按 Clock Type 注册的 Advance 响应图。
- [ ] 实现同类型多实例目标传递，不为每个实例复制静态 Schedule。
- [ ] 实现 `ExecuteContext`、`AdvanceContext<C>` 和 `ClockComponent<C>`。
- [ ] 实现 output sink 对未声明写入、编码失败和重复写入的永久记录及稳定错误优先级。
- [ ] 实现 JSON 与 typed Clock 共用的索引和执行路径。
- [ ] 提供 closure-scoped、不可跨 `await` 的 Bevy-native inspect/write 入口。
- [ ] 实现结构化 `SystemExecutionResult` pipe、redacting fallback marker、panic 捕获和
  Faulted 转移。
- [ ] 明确并测试 `Simulation: Send` 与 Bevy `NonSend` 插入线程亲和性的组合边界。
- [ ] 保持第一阶段无自有 `unsafe`、无动态 Component ABI；若实现需要改变必须先更新 Spec。
- [ ] 通过全部共享 Backend 合约和 Bevy 专项测试。

## P3：集成

- [ ] 提供纯行动驱动、单 Clock、同类型多 Clock 和混合推进示例。
- [ ] 提供应用自行把 Simulation 写入句柄注入 `ToolContext` 的示例，不增加 crate 互相依赖。
- [ ] 在实际端到端场景通过前保持 README 的“尚未实现”陈述。

## 后续范围

Hosted Loader、热重载、持久化和 Agent Runtime 需要各自的已接受设计或 Active Spec，不得从本
清单推导实现。安装、核心概念、Native Module、Clock 和 Bevy-native 用户指南仅在公共接口
冻结且稳定版推进获得明确授权后进入 `docs/`，不属于当前 Simulate 实施清单。
