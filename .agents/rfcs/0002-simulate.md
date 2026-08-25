# RFC 0002：Armillae Simulate 与可替换 ECS 后端

> 状态：Accepted
> 接受日期：2026-08-25
> 设计入口：[Armillae 设计索引](../DESIGN.md)
> 上位 RFC：[RFC 0001：Agentic 叙事运行时](0001-agentic-runtime.md)
> 落地规范：[Armillae Simulate Spec](../specs/simulate.md)

本 RFC 记录 Armillae 模拟基础设施的架构决定：使用 `armillae-simulate` 表达后端中立的执行与
推进能力，使用可替换 Backend 承载工作世界，首个 Backend 由 Bevy ECS 实现。它不设计 Agent
Harness、Tool 调度或持久化系统。

RFC 接受只表示本文件中的架构边界已经确认。公共行为、实现门禁、失败语义和合约测试以
[Simulate Active Spec](../specs/simulate.md) 为准；尚未完成的 Spike 仍会阻止对应代码路径实施。

## 1. 背景

Armillae 的下游可能是 Tick 驱动的大世界、行动驱动的卡牌或 TRPG、二者混合的叙事游戏，
也可能完全不包含 Agent。不同应用需要使用自己的数据、Clock、System 和推进方式，而不能被
固定的 `Character`、`Player`、`Turn`、`Scene` 或“当前主控角色”抽象限制。

单独暴露 ECS 只能解决内存数据和系统执行，不能自然形成稳定的跨语言协议；把所有规则固定为
编译期 Rust System，又无法满足 JavaScript、Python 和 Wasm 下游定义游戏规则的需要。因此，
Armillae 需要定义领域中立的模拟契约，并把具体 ECS 实现隔离为可替换 Backend。

## 2. 目标

1. 定义谁请求执行、谁选择执行计划、谁实际运行 System。
2. 同时支持 Tick、行动/事件驱动和混合推进，不内置唯一主循环。
3. 支持开发者自定义 Clock，同一种 Clock 类型允许存在多个独立实例。
4. 允许 System 对工作世界产生实际修改，包括直接修改组件、创建或删除实体。
5. 支持 Rust Native Module，并为 Hosted Module 保留不依赖 Rust ABI 的稳定方向。
6. 使用 Bevy ECS 作为首个 Backend，同时允许未来增加其它 ECS 或领域专用执行后端。
7. 隔离 Bevy 类型和版本，使 Bevy 升级不反向改变后端中立协议。
8. 为未来状态与持久化系统保留可重建边界，但不在本 RFC 中设计或实现持久化。

## 3. 非目标

- 定义角色、玩家、NPC、背包、地图、行动、回合或其它应用领域模型；
- 设计单 Agent、多 Agent、主 Agent、角色 Agent 或其 Harness；
- 决定 Tool 是否调用、按什么顺序调用、如何并发、重试或审批；
- 让 `LlmBridge`、`ToolExecutor`、ECS 或 Simulation 自动推进 Agent；
- 选择数据库、存档格式、StateStore、Revision、事件日志或 Schema 迁移协议；
- 序列化并长期保存 Bevy `World`、`Entity` 或内部调度状态；
- 设计渲染、物理、音频、输入、UI 或完整 Bevy App 生命周期；
- 第一阶段支持运行中热加载、卸载或替换 Module；
- 把进程内 JavaScript 或 Python 扩展描述为不受信任代码沙箱。

## 4. 已接受决策

### 4.1 命名与 crate 责任

采用以下名称：

| crate | 责任 |
|---|---|
| `armillae-simulate` | 后端中立的 Simulation 生命周期、显式执行与推进、Clock、Module 和 Backend 契约 |
| `armillae-simulate-bevy` | 使用 Bevy ECS 实现工作世界、Query、Schedule 和 Bevy-native 扩展面 |

`simulate` 比 `sim` 更容易被首次接触项目的开发者理解，也不暗示 Armillae 拥有用户的世界模型。
`armillae-world` 不作为基础 crate 名称；它只保留给未来可能出现的可选领域套件。

### 4.2 外部 Driver 拥有推进时机

Simulation 不拥有应用主循环，也不会自行调用 `advance`。开发者的 Driver 或 Harness 决定：

- 何时请求执行；
- 执行普通入口还是推进 Clock；
- 推进哪个 Clock 实例以及使用什么 Step；
- 是否在玩家行动、墙钟 Tick、网络事件、测试或 Tool 中发起请求；
- 多个请求之间的顺序、并发和重试策略。

即使未来提供墙钟 Driver，它也只能是开发者显式安装和启动的可选适配器，不能成为 Simulation
的隐藏行为。

### 4.3 Simulation 与 Backend 拥有单次执行边界

收到显式请求后，责任进一步划分为：

```text
开发者 Driver / Harness
        │ 选择何时、目标与输入
        ▼
armillae-simulate
        │ 校验生命周期、选择已注册执行入口、形成执行边界
        ▼
Simulation Backend
        │ 运行工作世界中的 Systems
        ▼
用户定义的状态修改与执行结果
```

这意味着“调度”存在两个不同层次：开发者拥有请求级调度；Simulation Backend 根据开发者已经
注册的执行图完成单次请求内部的 System 调度。后者不能反向决定下一次请求何时发生。

### 4.4 ECS 是工作世界与执行引擎

ECS 不只保存数据。首个 Bevy Backend 使用 `bevy_ecs` 提供：

- Entity、Component 和 Resource 构成的当前工作世界；
- Query 和受 Rust 借用规则约束的数据访问；
- System、Schedule、运行条件和显式排序；
- 根据读写集合识别冲突，并在允许时并行执行；
- 延迟结构修改和变更检测等运行期机制。

Bevy 不负责决定何时推进 Clock，也不提供 Armillae 的持久化事务、长期稳定身份或跨后端协议。
Bevy 的 `Changed<T>` 只能在 System 已经运行时筛选变化，不能替代显式 `advance` 请求。

### 4.5 Clock 由开发者定义并允许多实例

Armillae 不提供封闭的 `ClockDomain` 枚举，也不在 Clock API 中保留 `AdvanceCause`。开发者定义
Clock 的值、Step 和转移规则；Simulation 只拥有组合所需的不变量：

- Clock 类型具有稳定身份；
- 同一种 Clock 类型可以有多个独立实例；
- 每个实例具有自己的身份、当前值和生命周期；
- 每次推进显式指定目标实例和 Step；
- Clock 转移不能自行休眠、读取墙钟或调用 Agent；
- Clock 实例不属于玩家，控制角色热切换不改变 Clock 身份；
- 响应 Clock 的 System 按 Clock 类型注册，不为每个实例复制静态 Schedule；
- 本次目标实例作为显式执行输入交给响应 System。

Clock 值发生修改不会让 Bevy 自动运行 System。`armillae-simulate` 必须选择相应执行入口，
Backend 才会运行响应 Systems。任何后续 Clock 推进都是新的显式请求；当前执行不得通过隐藏
递归制造无限推进链。

### 4.6 执行与时间推进正交

Simulation 至少区分两类显式操作：

1. **Execute**：运行开发者注册的执行入口，可以修改工作世界但不要求推进 Clock；
2. **Advance**：转移一个或多个目标 Clock 实例，并运行该 Clock 类型的响应 Systems。

因此，一次 Tool 可以只设置 `Position`，玩家行动也可以只修改背包；它们不需要伪装为 Tick。
反过来，离线快进可以推进 Clock，而不需要制造玩家或 Agent Action。

Clock 转移与响应 Systems 属于同一个不可交错的 Simulation 执行边界。这里的“不可交错”只
描述活动工作世界的可观察性，不等同于持久化事务；持久化提交和失败恢复由未来独立状态设计
定义。

### 4.7 Module 是逻辑注册单元

Module 是安装 Schema、Clock 类型、执行入口、System 和能力声明的逻辑单元，不等同于一个
crate、Bevy `Plugin`、Rust Trait Object、动态库或脚本文件。第一阶段不提供能修改工作世界的
Module 生命周期钩子；应用状态在激活后、首次执行前显式初始化。

第一阶段生命周期为：

```text
describe -> validate -> register -> activate -> freeze
```

Module 集合在 Simulation 激活前注册；激活后冻结集合，但实体、Clock 实例和应用数据仍可动态
变化。运行中热替换需要独立设计，不进入首个实现范围。

执行面分为：

- **Native System**：通过 Backend-native API 使用高性能 Query；明确绑定 Backend 及版本；
- **Hosted System**：通过拥有所有权的批量输入和变更输出执行，不持有 ECS 引用或 Rust ABI。

Hosted Module 是已接受的架构方向，但具体 Loader、IDL、同步/异步 ABI 和安全模型需独立 Spec；
首个 Native 实现不得封死该扩展方向。

### 4.8 Agent、Tool 与 Simulation 相互独立

`armillae-simulate` 和 `armillae-simulate-bevy` 不依赖 `armillae-llm`、`armillae-tools` 或未来
Agent Harness。Simulation 不识别 Agent、ToolCall 或 ToolResult。

应用可以把自己拥有的 Simulation 写入句柄放入 `ToolContext`，让 Tool 直接调用公开的世界
修改或推进接口；具体句柄、锁、队列、顺序和权限仍由应用负责。Armillae 必须允许 Tool 对世界
产生真实副作用，但不能据此接管用户的 Tool 或 Agent 调度策略。

### 4.9 Backend 可替换，Bevy 版本由 Adapter 发布线隔离

后端中立 API 不暴露 Bevy `World`、`Entity`、`Query`、`ComponentId` 或 `Schedule`。这些类型
只能出现在 `armillae-simulate-bevy` 及明确标记为 Bevy-native 的扩展面。

`armillae-simulate-bevy` 的每条兼容发布线精确锁定一个经过 Spike 和合约测试验证的 Bevy
版本。升级 Bevy 需要新的 Adapter 发布与迁移验证，不在同一个 crate 中使用互斥
`bevy-0-x` Features 选择依赖版本。Cargo Features 会在依赖图中合并，应保持可叠加，不适合
承担互斥 ABI 选择。

如果未来必须同时维护多个不兼容 Bevy 版本，优先使用不同 Adapter 发布线；只有出现同一应用
必须并存多个版本的真实需求时，才评估独立版本化包名。未来自研 ECS 只有在替换存储、Query
或调度器时才属于新 Backend；普通游戏规则仍应实现为 Module/System。

### 4.10 持久化属于独立子系统

`armillae-simulate` 不实现数据库、文件存档、Snapshot、Journal、Revision、迁移或 StateStore。
未来状态与持久化能力属于独立 crate 与 RFC，本轮明确暂停。
Simulate 仍必须保持以下兼容约束：

- Bevy `Entity` 和内存布局不能成为稳定外部身份；
- 工作世界必须被视为可丢弃、可重新物化的执行视图；
- 后端中立 Module 描述不能依赖 Bevy 内存布局；
- 影响未来行为且希望恢复的数据不能只隐藏在不可导出的 Backend 临时状态中；
- 未来持久化集成不得要求 Agent Harness、ToolExecutor 或 LLM Bridge 进入 Simulation 底层。

上述约束只保留演进空间，不在当前 Spec 中引入持久化 API 或 Schema。

## 5. 依赖方向

```text
开发者应用 / 可选 Agentic Runtime
          │
          ├──────────────► armillae-llm / armillae-tools（可选、彼此独立）
          │
          ▼
 armillae-simulate
          ▲
          │ 实现后端契约
 armillae-simulate-bevy ─────────► 精确版本 bevy_ecs
```

约束如下：

- `armillae-simulate` 不依赖 Bevy、LLM、Tool、Agent SDK 或持久化实现；
- `armillae-simulate-bevy` 依赖 `armillae-simulate` 和精确版本的 `bevy_ecs`；
- 下游应用组合 Simulation 与 Tool/Agent，而不是让底层 crate 互相依赖；
- 未来状态 crate 与 Simulation 的依赖和提交协议必须由独立 RFC 决定。

## 6. 典型执行流程

### 6.1 无 Clock 的显式执行

```text
应用选择执行入口和输入
        -> Simulation 校验入口与生命周期
        -> Backend 运行已注册 Systems
        -> Systems 查询并修改工作世界
        -> Simulation 返回本次执行结果
```

该流程适用于玩家行动、Tool 直接写入、脚本事件或测试，不会隐式改变任何 Clock。

### 6.2 Clock 推进

```text
应用选择 Clock 实例和 Step
        -> Simulation 读取并验证目标实例
        -> 用户 Clock 计算 before -> after
        -> 形成显式 ClockAdvance 执行输入
        -> Backend 运行该 Clock 类型的响应 Systems
        -> 完成本次不可交错执行边界
        -> 返回推进结果
```

如果没有注册响应 System，推进只改变 Clock 值。Simulation 不会因为一个 Clock 变化而自行决定
推进另一个 Clock；应用可以在本次结果返回后显式发起下一次请求。

## 7. 主要取舍

### 7.1 收益

- 应用保留主循环、Agent 和 Tool 调度权；
- Tick、行动驱动和混合游戏共享同一基础设施；
- Bevy 提供成熟 ECS 性能，同时被隔离在 Adapter；
- 多 Clock 实例不依赖“当前玩家”或固定领域枚举；
- 未来可替换 Backend，持久化也不会被迫绑定 Bevy 内部格式；
- Native 与 Hosted 两种执行面可以按性能和可移植性选择。

### 7.2 成本与风险

- 后端中立层不能抽象所有 Native Query，Native Module 必然绑定后端版本；
- 多 Backend 需要共享合约测试，不能只靠相似 trait 名称宣称兼容；
- Bevy 默认并行顺序不提供业务确定性，显式排序和确定性等级仍需规范；
- Hosted Module 需要额外序列化、能力和超时协议；
- 持久化暂停期间只能冻结可重建约束，不能宣称已经支持存档或崩溃恢复。

## 8. 被拒绝的方案

| 方案 | 结论 | 原因 |
|---|---|---|
| `armillae-sim` | 不采用 | 对新用户含义不明确 |
| `armillae-world` 作为基础 crate | 不采用 | 暗示核心拥有用户世界与领域模型 |
| 直接把 Bevy 作为公共协议 | 不采用 | 版本、绑定、身份和兼容性泄漏 |
| 自研首个通用 ECS | 不采用 | 当前会重复成熟存储、Query 和调度能力 |
| 用 Feature 选择互斥 Bevy 版本 | 不采用 | Cargo Feature 合并语义与不兼容类型使组合脆弱 |
| 固定 `ClockDomain` | 不采用 | 无法覆盖用户自定义时间系统 |
| Clock 接收 `AdvanceCause` | 不采用 | 把调用来源耦合进时间语义 |
| Simulation 自动驱动 Agent 或 Tool | 不采用 | 侵犯下游 Harness 的策略所有权 |
| 在 Simulate 中实现持久化 | 不采用 | 状态存储与模拟执行是独立职责 |

## 9. 验收场景

Active Spec 和后续合约测试至少覆盖：

1. 普通 Execute 修改工作世界但不推进 Clock；
2. 单 Clock 推进只运行其注册的响应 Systems；
3. 同一种 Clock 的多个实例不会串写未选中的实例；
4. 应用不调用 `advance` 时，Simulation 不会自行推进；
5. Native System 可以直接查询和修改组件；
6. Tool 可经应用提供的写入句柄产生世界修改，底层 crate 不感知 Tool；
7. Module 激活后拒绝改变 Module 集合，但允许创建实体和 Clock 实例；
8. Bevy-native API 不泄漏到后端中立 crate；
9. Backend 版本变化不改变后端中立请求与结果语义；
10. 未实现状态子系统时，不宣称存档、持久化事务或无损恢复能力。

## 10. 实施门禁与后续工作

以下工作不会改变本 RFC 的已接受架构方向，但会限制对应实现范围：

1. 在 `.agents/spikes/` 完成 Bevy P0 Spike，验证精确版本、MSRV、最小 Features、Schedule、
   SystemParam/Local 初始化时序、显式执行 Context、结构化 fallible System pipe、`World: Send`、
   `NonSend` 线程亲和性、redacting fallback、panic 和故障边界；第一阶段不预造动态
   Component ABI；
2. Simulate Spec 已冻结 Rust 公共类型、JSON wire shape、object-safe `Simulation`、生命周期、
   Clock 批处理、Bevy-native API、结构化错误和共享合约测试入口；Spike 若发现不可实现之处必须
   先修订 Active Spec；
3. Hosted Module 的 IDL、Loader、同步/异步边界和能力模型进入后续独立 Spec 或 RFC；
4. 状态、持久化、Snapshot、Journal、Revision 和恢复语义保持暂停，用户重新启动该方向后再建
   独立 RFC；
5. 在 Active Spec 第 16 节剩余的 Bevy Spike 门禁完成前，不创建产品 crate 或代码。

## 11. 影响范围

### 11.1 直接影响

- 设计索引将模拟与持久化拆为两个责任；
- RFC 0001 只组合 Simulate，不再把持久化细节委托给本 RFC；
- 新增 Simulate Active Spec 和对应实施清单；
- 候选 crate 名从 `armillae-sim*` 改为 `armillae-simulate*`。

### 11.2 明确不受影响

- `LlmBridge` 仍只执行一次 Provider 无关 Model Call；
- `ToolExecutor` 仍只执行一次显式 `ToolCall -> ToolResult`；
- 不修改现有公共协议、Provider Adapter、crate、Cargo manifest 或版本；
- 不创建持久化 RFC、Schema、Store 或实现。

## 12. 决策记录与依据

本 RFC 的核心边界由项目方在 2026-08-25 的设计讨论中逐项确认：Bevy 是首个 ECS Backend；
Clock 由用户定义且允许同类型多实例；推进由用户 Driver 发起；Agent 和 Tool Harness 属于下游；
Tool 必须能够修改世界；crate 使用 `armillae-simulate` 命名；Bevy 版本由 Adapter 隔离；持久化
不在 Simulate 中实现且当前不创建其 RFC。

外部技术依据：

- [Bevy ECS 0.19.1](https://docs.rs/bevy_ecs/0.19.1/bevy_ecs/)：World、Query、System 与 Schedule
  能力；
- [Bevy Schedule](https://docs.rs/bevy_ecs/0.19.1/bevy_ecs/schedule/struct.Schedule.html)：显式
  运行、排序、条件和执行器边界；
- [Bevy World](https://docs.rs/bevy_ecs/0.19.1/bevy_ecs/world/struct.World.html)：工作世界、显式
  `Send` 实现与 `NonSend` 数据的插入线程访问约束；
- [Bevy Entity stability warning](https://docs.rs/bevy_ecs/0.19.1/bevy_ecs/entity/struct.Entity.html#stability-warning)：
  `Entity` 不提供长期序列化兼容保证；
- [Bevy migration guides](https://bevy.org/learn/migration-guides/introduction/)：快速演进及破坏性
  迁移风险；
- [Cargo Features](https://doc.rust-lang.org/cargo/reference/features.html)：Feature 合并和可叠加
  约束，不适合作为互斥依赖版本选择器。
