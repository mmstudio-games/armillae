# Armillae Simulate 规范

> 状态：Active Spec；实现尚未开始
> 规范基线：2026-08-26
> 适用范围：未来的 `armillae-simulate`、`armillae-simulate-bevy` 及其它 Simulation Backend
> 设计入口：[Armillae 设计索引](../DESIGN.md)
> 决策来源：[RFC 0002：Simulate 与可替换 ECS 后端](../rfcs/0002-simulate.md)
> 实施清单：[Simulate TODO](../todos/simulate.md)

本文是 Simulate 子系统的实施依据。它冻结第一阶段的责任、生命周期、执行语义、Clock 不变量、
Backend 隔离、Bevy 适配边界和合约测试。没有被本文明确纳入的 Agent Harness、Tool 调度、
持久化和 Hosted Loader 行为不得由实现自行补全。

本文中的“必须”“不得”和“只”属于规范要求；明确标记为“实施门禁”或“后续范围”的内容尚未
授权对应产品实现。本文已经冻结第一阶段后端中立协议、公共 Rust 标识符、对象安全接口、
Clock 批处理语义和 Bevy-native API 形状；实现不得以“内部细节”为由改变这些契约。Bevy
精确依赖与最小 Features 仍须通过 P0 Spike 编译验证，若 Spike 证明签名不可实现，必须先修改
本 Spec，再修改代码。

## 1. 范围

### 1.1 第一阶段目标

- 提供长期存在、由下游显式驱动的 Simulation 实例；
- 提供无时间执行和 Clock 推进两种正交操作；
- 支持同一种用户 Clock 类型的多个运行实例；
- 在激活前注册 Module、执行入口、Clock 类型和响应 Systems；
- 定义可替换 Simulation Backend 的行为契约；
- 以 Bevy ECS 实现第一个 Backend 和 Bevy-native 扩展面；
- 允许 Native System 或受控的 Backend-native 写入直接修改工作世界；
- 为 Hosted Module 和未来状态系统保留不泄漏 Bevy/Rust ABI 的边界；
- 提供共享 Backend 合约测试和 Bevy Adapter 专项测试。

### 1.2 明确非目标

- 自动主循环、固定 Tick 线程或墙钟定时器；
- Agent、Turn、Action、Player、NPC、Scene 等领域类型；
- 自动调用 LLM、执行 Tool 或继续 Tool Loop；
- 多 Agent 或多 Tool 的顺序、并发、重试、权限和公平性；
- 数据库、StateStore、存档、Revision、Snapshot、Journal、分支和回放；
- 完整 Bevy `App`、渲染、物理、音频、输入或窗口；
- 首个版本中的 Module 热加载和热卸载；
- 首个版本中的 JavaScript、Python 或 Wasm Loader 实现；
- 跨 Backend 移植 Bevy-native System 源代码。

## 2. 术语与所有权

| 术语 | 规范含义 | 所有者 |
|---|---|---|
| Simulation | 持有一个活动工作世界、已冻结 Module 集合和执行生命周期的长期实例 | `armillae-simulate` 契约，具体 Backend 实现 |
| Driver | 决定何时请求 Execute 或 Advance 的下游代码 | 开发者应用或可选上层 |
| Working World | 当前可查询、可执行和可修改的内存数据面 | Simulation Backend |
| Execute | 不要求改变 Clock 的显式执行请求 | 下游发起，Simulation 执行 |
| Advance | 显式改变目标 Clock 并运行其响应 Systems 的请求 | 下游发起，Simulation 执行 |
| Clock Type | 开发者定义的时间值、Step 和转移规则 | 开发者 Module |
| Clock Instance | 某个 Clock Type 下具有独立身份和值的运行实例 | Simulation Working World |
| Module | 注册描述、Clock、执行入口和 Systems 的逻辑单元 | 开发者 |
| Backend | 实现 Working World、System 注册和执行的数据/执行引擎 | Adapter |
| Native System | 直接使用某个 Backend 数据访问 API 的 System | 开发者，绑定 Backend 版本 |
| Hosted System | 通过拥有所有权的稳定输入/输出协议执行的 System | 后续 Spec |

“用户”在本文中指使用 Armillae 构建产品的开发者，不指其游戏中的玩家。

## 3. crate 与依赖边界

### 3.1 `armillae-simulate`

该 crate 必须拥有：

- Simulation 生命周期和状态转换语义；
- Backend 中立的 Module 描述最低公共信息；
- Execute 与 Advance 的请求、结果和错误语义；
- Clock Type 与 Clock Instance 的后端中立不变量；
- Backend 能力描述和共享合约测试入口；
- 后端无关的故障和可观察结果边界。

该 crate 不得依赖：

- `bevy_ecs` 或其它具体 ECS；
- `armillae-llm`、`armillae-tools` 或 Agent SDK；
- 数据库 Client 或具体持久化实现；
- Tokio 等特定异步运行时类型。

本文展示的后端中立公共类型、Trait、常量和错误必须从 `armillae_simulate` crate root 可用；
第 15.1 节测试支持只位于 `armillae_simulate::testing`，并受 additive `testing` feature 控制。

### 3.2 `armillae-simulate-bevy`

该 crate 必须：

- 依赖 `armillae-simulate`；
- 精确锁定一个经过 P0 Spike 验证的 `bevy_ecs` 版本；
- 使用 Bevy `World` 作为工作世界；
- 使用 Bevy Query 与 Schedule 执行 Native Systems；
- 提供明确标记为 Bevy-native、版本相关的扩展面；
- 实现 `armillae-simulate` 共享 Backend 合约测试；
- 使用已注册 Rust Clock 类型和普通 Bevy Component；第一阶段不为未来 Hosted Module 预造
  动态 Component ABI，也不因此引入自有 `unsafe`。

该 crate 不得：

- 依赖完整 Bevy 渲染或窗口栈，除非后续 Spec 明确扩围；
- 把 Bevy 类型放进 `armillae-simulate`；
- 把 Bevy `Entity` 作为后端中立 Clock ID、Module ID 或稳定对象 ID；
- 用互斥 Cargo Features 选择不同 Bevy 依赖版本。

本文展示的 Bevy Builder、Module、Registrar、Context、Clock Component、concrete Simulation
和 Bevy identity 常量必须从 `armillae_simulate_bevy` crate root 可用。实现可以再按内部模块
组织，但不得要求下游依赖私有路径。

### 3.3 依赖图

```text
armillae-simulate-bevy ──depends on──► armillae-simulate
          │
          └──────────────depends on──► exact bevy_ecs version

应用 / 可选 runtime ────depends on──► simulate implementation
应用 / 可选 runtime ────optionally──► armillae-llm / armillae-tools
```

现有 `armillae-core`、`armillae-llm` 和 `armillae-tools` 的公共 API 不因本 Spec 改变。

## 4. 公共协议约定与生命周期

### 4.1 版本、Serde 与 Schema

公共数据协议版本固定为：

```rust
pub const SIMULATE_API_VERSION: &str =
    "armillae.simulate/v1alpha1";
```

该常量标识 `ModuleDescriptor` 的 wire shape，不等于任一 crate 的 SemVer。第一阶段没有定义
HTTP、RPC、消息队列或进程间 framing；“wire protocol”只指这些拥有所有权的 Rust 类型对应的
JSON 和 JSON Schema，可由 N-API、PyO3、Wasm 或其它 Binding 映射。

除错误类型、Trait 和明确标记为 Bevy-native 的类型外，公共协议类型必须派生
`Clone`、`Debug`、`PartialEq`、`Serialize`、`Deserialize` 和 `JsonSchema`。规则如下：

- 字段使用 `snake_case`；
- 携带数据的枚举使用 `#[serde(tag = "type", rename_all = "snake_case")]`；
- 字符串枚举使用 `snake_case`；
- 列表字段反序列化时允许缺失并视为空列表，规范序列化始终输出字段；
- `Option` 字段缺失或 `null` 均视为 `None`，规范序列化始终输出字段；
- JSON Schema 使用 Draft 2020-12，Schema 本身必须是 JSON Object；
- 生成的协议对象 Schema 必须允许额外属性，与解码器忽略未知对象字段的前向兼容规则一致；
  不得由 derive 默认值意外生成 `additionalProperties: false`；
- `ModuleDescriptor`、`ExecuteRequest`、`ExecuteOutcome`、`ClockState`、
  `AdvanceRequest`、`AdvanceOutcome` 和 `SimulationCapabilities` 是 Schema 快照根；
- 同一 `v1alpha1` 解码器允许并忽略未知对象字段以便前向兼容，但不承诺反序列化后无损转发
  这些字段；未知枚举 `type` 必须拒绝，不能静默映射为已有语义。

### 4.2 逻辑 ID

后端中立层拥有以下透明字符串新类型：

```rust
#[serde(transparent)]
pub struct ModuleId(String);

#[serde(transparent)]
pub struct ExecuteEntryId(String);

#[serde(transparent)]
pub struct ClockTypeId(String);

#[serde(transparent)]
pub struct ClockInstanceId(String);

#[serde(transparent)]
pub struct ClockErrorCode(String);

#[serde(transparent)]
pub struct SystemErrorCode(String);

#[serde(transparent)]
pub struct SystemId(String);

#[serde(transparent)]
pub struct BackendId(String);

#[serde(transparent)]
pub struct CapabilityId(String);
```

每个类型都必须实现 `Eq`、`Ord`、`Hash`、`AsRef<str>`、`Display`、`FromStr` 和以下同形
API：

```rust
impl ModuleId {
    pub fn new(value: impl Into<String>)
        -> Result<Self, InvalidIdentifier>;
    pub fn as_str(&self) -> &str;
    pub fn into_inner(self) -> String;
}
```

合法值必须为 1 至 255 字节的可见 ASCII 字符（`0x21..=0x7e`）；不得包含空格、控制字符或
依赖 Unicode normalization 的值。构造和反序列化执行同一校验，不实现 `Default`。错误为：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum IdentifierKind {
    Module,
    ExecuteEntry,
    ClockType,
    ClockInstance,
    ClockErrorCode,
    SystemErrorCode,
    System,
    Backend,
    Capability,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InvalidIdentifierReason {
    Empty,
    TooLong { max_bytes: usize },
    NonGraphicAscii,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("invalid {kind:?} identifier: {reason:?}")]
pub struct InvalidIdentifier {
    pub kind: IdentifierKind,
    pub reason: InvalidIdentifierReason,
}
```

`ModuleId`、`ExecuteEntryId`、`ClockTypeId` 和 `SystemId` 在一个 Builder 中全局唯一；不同
Clock Type 可以复用相同 `ClockInstanceId`，因此 Clock Instance 的完整身份始终是
`ClockTypeId + ClockInstanceId`。`ClockErrorCode` 和 `SystemErrorCode` 使用同一词法校验，但不
参与 ID 唯一性。
所有值按字节比较，不做大小写折叠或自动命名空间补全。

### 4.3 版本字符串

协议不把第三方 `semver` 类型暴露在公共字段中，而使用 Armillae 自有透明字符串新类型：

```rust
#[serde(transparent)]
pub struct SemanticVersion(String);

#[serde(transparent)]
pub struct VersionRequirement(String);

impl SemanticVersion {
    pub fn new(value: impl Into<String>)
        -> Result<Self, InvalidVersion>;
    pub fn as_str(&self) -> &str;
}

impl VersionRequirement {
    pub fn new(value: impl Into<String>)
        -> Result<Self, InvalidVersion>;
    pub fn as_str(&self) -> &str;
    pub fn matches(&self, version: &SemanticVersion) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VersionKind {
    SemanticVersion,
    Requirement,
}

#[derive(Clone, Debug, thiserror::Error)]
#[error("invalid {kind:?}: {message}")]
pub struct InvalidVersion {
    pub kind: VersionKind,
    pub message: String,
}
```

`SemanticVersion` 接受 SemVer 2.0.0；`VersionRequirement` 使用 Rust `semver::VersionReq` 的
语法和匹配语义。构造和反序列化都必须先解析，再保存对应 `semver` 类型 `Display` 产生的规范
字符串；禁止保留无法判断或仅因空白不同而产生多个表示的原始字符串。两种类型都实现
`Eq`、`Ord`、`Hash`、`AsRef<str>`、`Display` 和 `FromStr`，比较与 Hash 使用规范字符串；版本
匹配只使用解析后的 SemVer 语义。

### 4.4 Builder 与活动实例

`Building` 是 Backend Builder 的状态，不是可被下游持有的 `dyn Simulation` 状态：

```text
Backend Builder (Building) ──activate──► Simulation (Active)
                                             │
                                             ├──stop──► Stopped
                                             └──fatal failure──► Faulted
```

- Building 允许分阶段描述和暂存 Module；单次注册失败不得改变 Builder；
- `activate(self)` 消耗 Builder，完整校验依赖、能力、Clock 绑定和 System 图；失败时不产生
  Simulation；
- Active 的 Module、Clock Type 和 System 图冻结，但应用实体和 Clock Instance 可以变化；
- Stopped 拒绝新的写操作，不支持重新激活；重复 `stop` 幂等成功；
- Stopped 仍允许后端中立 `read_clock` 和 Bevy `inspect_world`；
- Faulted 表示无法证明工作世界一致，读写都拒绝；当前只能丢弃并重新构造实例；
- `status` 和 `capabilities` 在任何状态都可调用。

## 5. 执行所有权与并发

### 5.1 请求级调度归下游

只有下游 Driver 可以请求 Execute 或 Advance。Simulation 不得根据以下事实自行发起请求：

- 墙钟经过了一段时间；
- 某个 Component 被修改；
- LLM 返回 ToolCall；
- 玩家或 NPC 完成了行动；
- 一个 System 判断另一个 Clock 应该前进。

这些事实可以被下游转换为新的显式请求，但转换策略不属于本 Spec。

### 5.2 单次执行内部调度归 Backend

Simulation 校验请求并选择注册入口后，Backend 按已冻结的 System 图执行。Backend 可以利用
声明的数据访问并行运行无冲突 Systems；存在语义顺序时，Module 必须显式声明先后关系。

未声明顺序的可并行 Systems 不得被宣传为具有稳定执行顺序。确定性场景必须在 Module 和
Backend 配置中消除相关歧义。

### 5.3 不接管 Harness 并发策略

第一阶段的核心契约不提供 Agent/Tool 工作队列、优先级或公平调度器。Rust API 必须能够通过
独占可变访问表达一次写执行；下游需要跨任务共享时，可以自行使用 Mutex、Actor、请求队列或
其它 Harness。
若未来提供可克隆 `SimulationHandle`，它必须进入独立 Spec，明确请求排序、背压、取消和关闭
语义后才能实现。

## 6. Execute 契约

Execute 用于运行开发者注册的命名入口，不隐含时间推进。第一阶段 API 精确为：

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteRequest {
    pub entry: ExecuteEntryId,
    pub input: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteOutcome {
    pub entry: ExecuteEntryId,
    pub output: Option<serde_json::Value>,
}
```

一次 Execute 必须：

1. 只在 Active 状态接受；
2. 查找 `entry`，未知入口返回 `UnknownExecuteEntry`；
3. 使用入口的 Draft 2020-12 Schema 校验 `input`，失败时不运行任何 System；
4. 把完整 `ExecuteRequest` 和单赋值 output sink 作为本次执行上下文；
5. 在一个不可交错的工作世界执行边界内运行该入口的 System 图；
6. 若入口声明 output Schema，恰好一个 System 必须设置 output；未声明时任何 System 都不得
   尝试设置；
7. 在成功返回前应用 Backend 的所有 deferred world changes，并校验 output；
8. 返回唯一 `ExecuteOutcome` 或结构化错误；
9. 不自动改变 Clock，也不自动请求另一次 Execute 或 Advance。

`ExecuteOutcome.output` 是 Module 自己定义的领域结果，不是 World Diff、事件日志或持久化
Revision。`None` 只表示入口未声明 output；需要合法 JSON `null` 时，入口必须声明允许 null 的
Schema 并由 System 显式设置 `Some(Value::Null)`。output sink 是 single-assignment，并永久
记录所有写入违规：未声明却尝试写入、编码失败或第二次写入，即使 System 忽略返回的错误也不
能被当成成功。多个并行 System 竞争写入不会采用“最后写入获胜”，而是使本次执行失败并
Fault。

这允许 Binding、Tool 或 HTTP 层取得应用定义的结果，同时不替应用规定 ActionResult 结构。
如果以后需要通用 World Query 或 Diff 协议，仍须独立设计。

用户 System 可以通过 Bevy-native API 直接修改受管理 Clock Component；这种修改属于明确的
低层绕过，不运行 Advance 响应图，也不产生 `ClockTransition`。`armillae-simulate` 不内置
Action、Command、Turn 或 ToolCall 类型。

## 7. Clock 与 Advance 契约

### 7.1 Rust Clock Trait

`Clock` 是类型安全 Native API，同时要求值和 Step 具有稳定数据表示：

```rust
#[derive(
    Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema,
    thiserror::Error,
)]
#[error("clock transition rejected: {code}: {message}")]
pub struct ClockTransitionError {
    pub code: ClockErrorCode,
    pub message: String,
}

pub trait Clock:
    Clone
    + Send
    + Sync
    + serde::Serialize
    + serde::de::DeserializeOwned
    + schemars::JsonSchema
    + 'static
{
    type Step:
        Clone
        + Send
        + Sync
        + serde::Serialize
        + serde::de::DeserializeOwned
        + schemars::JsonSchema
        + 'static;

    fn validate(&self) -> Result<(), ClockTransitionError> {
        Ok(())
    }

    fn advance(
        &self,
        step: &Self::Step,
    ) -> Result<Self, ClockTransitionError>;
}
```

稳定逻辑身份不写在 Rust `TypeId` 或关联常量中，而由 `ClockDefinition.id` 注册；一个活动
Simulation 中，一个 Rust `Clock` 类型只能绑定一个 `ClockTypeId`。需要复用相同数据布局表达
多个逻辑 Clock Type 时，开发者必须使用 Rust newtype。

`validate` 校验独立 Clock 值；`advance` 实现 `current + explicit step -> next`。二者必须是
同步纯计算，不得修改 World、读取墙钟、休眠、执行 I/O、调用 LLM/Tool/Agent 或递归请求
Simulation。`advance` 返回的 next 必须再次通过 `validate`；动态 JSON 路径还必须在写入前完成
serde 编码并通过 `ClockDefinition.value_schema`。`code` 必须非空且是可供应用判断的稳定领域
码；`message` 用于脱敏诊断，不得拼入完整领域值或 Secret。

### 7.2 后端中立 Clock 协议

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub struct ClockKey {
    pub clock_type: ClockTypeId,
    pub instance: ClockInstanceId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClockState {
    pub key: ClockKey,
    pub value: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AdvanceTarget {
    pub instance: ClockInstanceId,
    pub step: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AdvanceRequest {
    pub clock_type: ClockTypeId,
    #[serde(default)]
    pub targets: Vec<AdvanceTarget>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClockTransition {
    pub instance: ClockInstanceId,
    pub before: serde_json::Value,
    pub step: serde_json::Value,
    pub after: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AdvanceOutcome {
    pub clock_type: ClockTypeId,
    #[serde(default)]
    pub transitions: Vec<ClockTransition>,
}
```

Native typed fast path 使用以下拥有所有权的泛型类型；它们与 JSON 路径必须共享同一内部执行，
不得形成第二套语义：

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct TypedAdvanceTarget<S> {
    pub instance: ClockInstanceId,
    pub step: S,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypedAdvanceRequest<S> {
    pub targets: Vec<TypedAdvanceTarget<S>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypedClockTransition<C: Clock> {
    pub instance: ClockInstanceId,
    pub before: C,
    pub step: C::Step,
    pub after: C,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypedAdvanceOutcome<C: Clock> {
    pub clock_type: ClockTypeId,
    pub transitions: Vec<TypedClockTransition<C>>,
}
```

### 7.3 Clock Instance 管理

- `insert_clock` 只接受已注册 Clock Type；同一个 `ClockKey` 重复插入返回
  `DuplicateClockInstance`，不得覆盖；
- 动态值先按 `ClockDefinition.value_schema` 校验，再反序列化为注册 Rust 类型并调用
  `Clock::validate`；
- `read_clock` 返回拥有所有权的 `ClockState`，不暴露 ECS 借用或 Entity；
- `read_clock` 在返回前完成 Native value 编码；`remove_clock` 必须先编码完整 `ClockState` 再
  删除。编码失败时返回 `ClockValueRejected` 的 well-known codec code，World 不变；
- `remove_clock` 返回被删除的完整 `ClockState`；未知实例不视为幂等成功；
- 成功移除后可以重新插入同一个 `ClockKey`；第一阶段不附带 generation 或稳定对象身份，旧
  `ClockState` 不构成对新实例的句柄；
- 同一种 Clock Type 必须允许多个实例，不存在当前玩家或默认实例；
- 第一阶段没有通用 pause 字段；暂停是开发者的领域 Component 或“不发起 Advance”的 Driver
  策略，核心不得猜测。

### 7.4 同类型批量 Advance

第一阶段一次 `AdvanceRequest` 只能推进一个 `ClockTypeId` 下的多个实例。精确语义为：

1. `targets` 必须非空，公共层不设置固定数量上限；
2. `ClockInstanceId` 在单次 `targets` 中不得重复；
3. 结果 `transitions` 与请求 `targets` 保持一一对应和输入顺序；
4. 在任何 World 修改前，Backend 必须定位全部实例、校验并解码全部 Step、计算 next、调用
   `Clock::validate`，并完成结果所需的 before/after JSON 编码及 value Schema 校验；
5. 任一目标在预计算阶段失败时，所有 Clock 值和其它 World 数据保持不变；
6. 预计算成功后一次性写入全部目标 Clock 的 after 值；
7. 构造包含 `clock_type` 及完整 typed transitions 的显式 Advance Context；
8. 运行该 Clock Type 的唯一响应 System 图，并在返回前应用 deferred changes；
9. 没有响应 System 时只完成 Clock 更新；
10. 未在 `targets` 中的同类型实例不得被 Adapter 自动改变。

`ClockTransition.after` 固定表示 `Clock::advance` 计算并在响应 Schedule 前写入的值。Native
System 仍可经第 10.3 节的低层逃生舱再次修改任意 Clock；这种绕过不会重写已经形成的
`AdvanceOutcome`，需要最终工作世界值的调用方应在成功返回后显式 `read_clock`。应用若要求
`after` 与最终值恒等，就不得在该响应图中直接改写受管理 Clock。

这是一种“预计算全有或全无 + System 阶段不可回滚”的边界，不是持久化事务。跨 Clock Type 的
原子批处理不属于第一阶段；下游要推进另一 Clock，必须在本次返回后发起新请求。上下文不包含
`AdvanceCause`、玩家、Agent、ToolCall 或隐式父请求。

### 7.5 System 阶段失败

Clock 写入和响应 Systems 对其它安全调用者不可交错，但第一阶段没有 World rollback：

- Schema、目标、重复 ID、`Clock::validate` 和 `Clock::advance` 失败发生在修改前，实例保持
  Active；
- System 开始后出现显式失败，即使失败发生在首个写入前，也一律进入 Faulted；第一阶段不尝试
  根据 Bevy access metadata 证明可恢复；
- System 失败前后已经执行的 Clock 写入、Component 写入或 deferred command 可能存在，调用方
  不得继续读取或复用 Faulted World；
- Backend panic 在 `panic = "unwind"` 时必须被执行边界捕获、转为 `BackendPanicked` 并
  Fault；`panic = "abort"` 无法提供进程内恢复保证；
- 成功返回时，本次 Clock 和响应 Systems 的全部可见修改已经完成，不能后台补写。

未来持久化协议可以把“预计算、提交和恢复”提升为更强事务；当前 Spec 不提前定义。

## 8. Module 契约

### 8.1 描述类型

`ModuleDescriptor` 是 Native 与未来 Hosted Module 共用的稳定控制面：

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModuleDescriptor {
    pub api_version: String,
    pub id: ModuleId,
    pub version: SemanticVersion,
    #[serde(default)]
    pub dependencies: Vec<ModuleDependency>,
    pub execution: ExecutionPlane,
    #[serde(default)]
    pub required_capabilities:
        std::collections::BTreeSet<CapabilityId>,
    #[serde(default)]
    pub execute_entries: Vec<ExecuteEntryDefinition>,
    #[serde(default)]
    pub clocks: Vec<ClockDefinition>,
    #[serde(default)]
    pub systems: Vec<SystemDefinition>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ModuleDependency {
    pub id: ModuleId,
    pub version: VersionRequirement,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExecutionPlane {
    Native {
        backend: BackendId,
        adapter: VersionRequirement,
    },
    Hosted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ExecuteEntryDefinition {
    pub id: ExecuteEntryId,
    pub input_schema: serde_json::Value,
    pub output_schema: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ClockDefinition {
    pub id: ClockTypeId,
    pub value_schema: serde_json::Value,
    pub step_schema: serde_json::Value,
}

#[derive(
    Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum SystemTrigger {
    Execute { entry: ExecuteEntryId },
    Advance { clock_type: ClockTypeId },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SystemDefinition {
    pub id: SystemId,
    pub trigger: SystemTrigger,
    #[serde(default)]
    pub before: Vec<SystemId>,
    #[serde(default)]
    pub after: Vec<SystemId>,
}
```

Rust Native Module 应使用以下 helpers 生成 Schema，避免手写 Schema 与绑定类型漂移：

```rust
impl ExecuteEntryDefinition {
    pub fn for_input<I: schemars::JsonSchema>(
        id: ExecuteEntryId,
    ) -> Self;

    pub fn for_input_output<
        I: schemars::JsonSchema,
        O: schemars::JsonSchema,
    >(
        id: ExecuteEntryId,
    ) -> Self;
}

impl ClockDefinition {
    pub fn for_clock<C: Clock>(
        id: ClockTypeId,
    ) -> Self;
}
```

`api_version` 必须精确等于 `SIMULATE_API_VERSION`。一个 Module 只能选择一种
`ExecutionPlane`；需要同时提供 Hosted 与 Native 实现时，应共享领域 Schema、使用不同
`ModuleDescriptor` 实例并由应用在注册前选择一个。语义等价的替代实现应保留相同
`ModuleId`、version、entry/clock/system ID 和 Schema，只改变 `execution`；语义或协议不兼容时
才提升 version 或使用不同 `ModuleId`。同一个 Builder 仍不得同时注册两个相同 `ModuleId`，也
不能在一个描述中混合 ABI。

Native `adapter` 匹配的是 Adapter crate 的 SemVer，不是 Bevy engine 版本；具体 engine 版本
由 `SimulationCapabilities.backend.engine` 报告并由 Cargo 类型一致性约束。Hosted 描述可以
被解析和验证，但 Backend 未报告 Hosted 能力时必须在激活前拒绝。

### 8.2 Schema 与身份规则

- `input_schema`、存在时的 `output_schema`、`value_schema` 和 `step_schema` 必须是有效的
  Draft 2020-12 Schema Object；
- Native `ClockDefinition` 必须与绑定 Rust `Clock` 的 serde 表示和 `Clock::validate` 语义一致：
  `validate` 接受的值序列化后必须通过 `value_schema`，Schema 接受且成功解码的值也必须通过
  `validate`；自定义收紧 Schema 时必须同步收紧 `validate`；
- `ExecuteEntryId`、`ClockTypeId` 与 `SystemId` 在整个 Builder 中分别全局唯一，而不是只在
  Module 内唯一；
- Module 应以自身命名空间构造上述 ID，但核心不自动拼接 `ModuleId`；
- 同一个 `ModuleDependency.id` 不得重复，也不得依赖自身；依赖只表达存在性和版本约束，第一
  阶段没有按依赖顺序运行的生命周期钩子，因此不同 Module 间的依赖环不改变执行语义，也不
  单独拒绝；
- 一个 Clock Type 由恰好一个 Module 提供；依赖 Module 的 System 可以订阅该 Clock；
- 一个 Execute Entry 由恰好一个 Module 提供；其它 Module 可通过 System trigger 扩展该入口；
- System 引用其它 Module 拥有的 trigger 或 ordering target 时，引用方必须在
  `dependencies` 中声明目标 Module；不得用全局 ID 的偶然存在替代版本依赖；
- `before` 和 `after` 只允许引用相同 `SystemTrigger` 下的已注册 System；
- 对 `SystemDefinition { id: current, .. }`，`before: [target]` 形成 `current -> target`，
  `after: [target]` 形成 `target -> current`；
- 同一个排序边重复声明可以去重；自引用、跨 trigger 边和环必须拒绝；
- 一个未声明 output 的入口或任一 Clock 可以没有 System，对应 Execute 成为 no-op receipt，
  Advance 只改变 Clock；声明 output 的入口若没有 System 设置值会在执行后 Fault；
- Native Module 描述中的每个 `SystemDefinition` 必须绑定且只绑定一个 Native System；
  未声明的 Native System 也不得注册。

### 8.3 System 执行结果

需要把显式执行失败交给 Simulation 的 Native System 必须返回 Armillae 自有结果，而不是把
`bevy_ecs::error::BevyError` 或其它 Backend error 放进公共契约：

```rust
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("system execution failed: {code}: {message}")]
pub struct SystemExecutionError {
    pub code: SystemErrorCode,
    pub message: String,
}

pub type SystemExecutionResult =
    Result<(), SystemExecutionError>;
```

`SystemErrorCode` 是应用可以判断的稳定错误码；`message` 只用于脱敏诊断。返回
`SystemExecutionError` 表示本次 System 执行已经开始，因而按照第 7.5 节进入 Faulted，而不是
领域校验失败后的可重试拒绝。普通不声明失败的 System 仍返回 `()`。

具体 Backend 必须把该结果接入自己的执行器，并保留 `SystemDefinition.id`；不得把它先擦除为
Backend 字符串再猜测错误码。Backend 自身的参数校验、Command、Observer 或执行器错误仍进入
`BackendFailure` / `BackendPanicked` 边界，不冒充应用声明的 `SystemExecutionError`。

### 8.4 构建错误

Builder 和 Module 注册统一使用以下结构化错误：

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SimulationBuildError {
    #[error("invalid module descriptor: {code}: {message}")]
    InvalidDescriptor {
        module: Option<ModuleId>,
        code: String,
        message: String,
    },

    #[error("duplicate module `{module}`")]
    DuplicateModule { module: ModuleId },

    #[error("duplicate execute entry `{entry}`")]
    DuplicateExecuteEntry {
        entry: ExecuteEntryId,
        first: ModuleId,
        second: ModuleId,
    },

    #[error("duplicate clock type `{clock_type}`")]
    DuplicateClockType {
        clock_type: ClockTypeId,
        first: ModuleId,
        second: ModuleId,
    },

    #[error("duplicate system `{system}`")]
    DuplicateSystem {
        system: SystemId,
        first: ModuleId,
        second: ModuleId,
    },

    #[error("module `{module}` requires missing module `{dependency}`")]
    MissingDependency {
        module: ModuleId,
        dependency: ModuleId,
    },

    #[error("module `{module}` has an incompatible dependency")]
    IncompatibleDependency {
        module: ModuleId,
        dependency: ModuleId,
        required: VersionRequirement,
        found: SemanticVersion,
    },

    #[error("system `{system}` references an unknown trigger")]
    UnknownTrigger {
        module: ModuleId,
        system: SystemId,
        trigger: SystemTrigger,
    },

    #[error("invalid ordering edge from `{system}` to `{target}`")]
    InvalidOrdering {
        system: SystemId,
        target: SystemId,
        reason: OrderingError,
    },

    #[error("system ordering contains a cycle")]
    OrderingCycle {
        trigger: SystemTrigger,
        systems: Vec<SystemId>,
    },

    #[error("module `{module}` requires unsupported capability `{capability}`")]
    UnsupportedCapability {
        module: ModuleId,
        capability: CapabilityId,
    },

    #[error("module `{module}` uses an unsupported execution plane")]
    UnsupportedExecutionPlane {
        module: ModuleId,
        execution: ExecutionPlane,
    },

    #[error("module `{module}` targets a different backend")]
    BackendMismatch {
        module: ModuleId,
        required: BackendId,
        actual: BackendId,
    },

    #[error("module `{module}` requires an incompatible adapter version")]
    IncompatibleAdapter {
        module: ModuleId,
        backend: BackendId,
        required: VersionRequirement,
        found: SemanticVersion,
    },

    #[error("native registration failed for module `{module}`: {code}")]
    NativeRegistrationFailed {
        module: ModuleId,
        code: String,
        message: String,
    },

    #[error("failed to build system graph")]
    SystemGraphBuildFailed {
        trigger: SystemTrigger,
        code: String,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum OrderingError {
    SelfReference,
    UnknownSystem,
    DifferentTrigger,
}
```

`message` 不得包含完整 Module 输入、Component 值或其它敏感数据；调用方应主要按错误变体和
逻辑 ID 判断。

### 8.5 注册原子性与冻结

Module 注册分为 incoming staging 与 Builder commit：

1. 读取并校验完整 `ModuleDescriptor`；
2. 在独立 staging registrar 中接收 Clock 绑定和 Native Systems；
3. 验证所有声明与实现一一对应；
4. 检查与 Builder 已有全局 ID 和能力的冲突；
5. 全部成功后一次性并入 Builder。

任一步失败都丢弃 staging，Builder 可继续注册其它 Module。跨 Module 依赖是否满足、完整排序
图和 Backend Schedule 构建在 `activate(self)` 统一验证；激活失败不返回部分 Simulation。
激活后不提供 Module 注册、卸载或热替换 API。

Backend-native `descriptor` / `register` 在 `panic = "unwind"` 下 panic 时，Builder 必须丢弃
payload 和 staging，返回 `NativeRegistrationFailed`，code 固定为
`armillae.simulate/native_module_panicked`、message 固定为 `native module panicked`，并保持
已有 Builder 内容可继续使用；
`panic = "abort"` 不可恢复。

第一阶段只保证单个构建错误的结构化分类，不保证同时存在多个独立描述错误时返回完整列表或
跨 Backend 的“第一个错误”顺序；合约测试必须一次隔离一个无效事实。原子性、错误变体中的
逻辑 ID 和 `first` / `second` 注册来源仍是稳定事实，Backend 不得用无结构字符串替代。

### 8.6 Native 与 Hosted 边界

Native System 可以直接使用目标 Backend 的 Query 和写入 API；其源代码兼容性跟随 Adapter
发布线。Module Descriptor、逻辑 ID 和 JSON Schema 仍不得依赖随机 Rust `TypeId`。

Hosted 是已接受但未实现的执行面。未来协议必须使用拥有所有权的批量输入和可校验变更输出，
不得跨语言持有 ECS 借用、裸指针、Rust Trait ABI 或跨 `await` 的 World 引用。首个实现只可
返回 `UnsupportedExecutionPlane`，不得用临时 Callback ABI 冒充 Hosted 支持。

## 9. Backend 契约

### 9.1 能力协议

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EngineInfo {
    pub name: String,
    pub version: SemanticVersion,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct BackendInfo {
    pub id: BackendId,
    pub adapter_version: SemanticVersion,
    pub engine: Option<EngineInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SimulationCapabilities {
    pub backend: BackendInfo,
    #[serde(default)]
    pub supported: std::collections::BTreeSet<CapabilityId>,
}

impl SimulationCapabilities {
    pub fn supports(&self, capability: &CapabilityId) -> bool;
}
```

第一阶段保留以下 well-known capability 字符串：

| 值 | 含义 |
|---|---|
| `armillae.simulate/native_modules` | 可以注册 Backend-native Module |
| `armillae.simulate/hosted_modules` | 可以执行稳定 Hosted ABI；首个 Bevy 实现不得报告 |
| `armillae.simulate/backend_native_access` | Concrete type 提供受作用域约束的 Native World 访问 |
| `armillae.simulate/parallel_systems` | 当前构建和执行器允许无冲突 System 并行 |

Execute、Clock 管理、Advance、状态查询和结构化错误属于所有 Backend 的基础契约，不作为可选
capability。未知 capability 可以保留在集合中；Module 只按自己理解且明确声明的 ID 预检。
实现不得通过可序列化配置伪造 Backend 实际不具备的能力。
Capabilities 在 `activate` 时冻结，并在 Active、Stopped 和 Faulted 中返回相同值；运行期不得因
某次 Schedule 是否恰好并行而改变集合。

### 9.2 活动 Simulation 接口

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SimulationStatus {
    Active,
    Stopped,
    Faulted,
}

pub trait Simulation: Send {
    fn status(&self) -> SimulationStatus;

    fn capabilities(&self) -> SimulationCapabilities;

    fn execute(
        &mut self,
        request: ExecuteRequest,
    ) -> Result<ExecuteOutcome, SimulationError>;

    fn read_clock(
        &self,
        key: &ClockKey,
    ) -> Result<ClockState, SimulationError>;

    fn insert_clock(
        &mut self,
        state: ClockState,
    ) -> Result<(), SimulationError>;

    fn remove_clock(
        &mut self,
        key: &ClockKey,
    ) -> Result<ClockState, SimulationError>;

    fn advance(
        &mut self,
        request: AdvanceRequest,
    ) -> Result<AdvanceOutcome, SimulationError>;

    fn stop(&mut self) -> Result<(), SimulationError>;
}
```

该 Trait 必须保持 object-safe，使下游可以持有 `Box<dyn Simulation>`。它刻意只要求 `Send`：
`&mut self` 是一次写执行的并发令牌，`Mutex<Box<dyn Simulation>>`、Actor 或请求队列由下游按
Harness 需要选择；核心不要求每个 Backend 内部可被 `&self` 并发写入，也不提供隐藏锁。

API 为同步接口，因为第一阶段 Native Clock 和 ECS Schedule 都在调用作用域内完成。需要调用
Agent、网络或文件的应用通常在进入 Simulation 前后完成 I/O，再把结果作为 Execute input 或
World 数据提交。Native System 仍可由开发者自行执行同步外部代码，但 Adapter 不轮询 Future，
也不为这些外部副作用提供超时、取消、幂等、回滚或补偿；未完成的异步工作不能持有 World
借用越过方法返回。未来 Hosted async ABI 若成立，需单独定义超时、取消、背压和持有状态，
不能直接把本 Trait 改成依赖某个异步运行时。

### 9.3 状态操作矩阵

| 操作 | Active | Stopped | Faulted |
|---|---|---|---|
| `status` / `capabilities` | 允许 | 允许 | 允许 |
| `read_clock` | 允许 | 允许 | 拒绝 |
| `execute` / `insert_clock` / `remove_clock` / `advance` | 允许 | 拒绝 | 拒绝 |
| `stop` | 转为 Stopped | 幂等成功 | 拒绝 |
| Backend-native inspect | 允许 | 允许 | 拒绝 |
| Backend-native write | 允许 | 拒绝 | 拒绝 |

第一阶段没有 `start`、`restart`、`cancel`、`try_lock` 或可克隆 `SimulationHandle`。同步方法一旦
开始就运行到成功或错误；调用方不能用 drop 中断正在运行的 Schedule。下游可以在调用开始前
取消排队请求，但那不是 Simulation 终止结果。

### 9.4 Backend 实现责任

每个 Backend 必须：

- 提供自己的 Building Builder，并在激活时原子冻结 Module 与 System 图；
- 实现上述 `Simulation` Trait；
- 为动态 Clock JSON 与 Native Clock 类型使用同一执行路径；
- 报告真实能力并在激活前拒绝不支持的 Module；
- 不把内部 Entity、archetype、borrow 或 Schedule 类型放进后端中立协议；
- 通过第 15 节共享合约测试。

Backend 可以提供 concrete-type 扩展，但不能通过扩展改变基础方法的可观察语义。兼容性由协议、
状态转换与合约测试决定，不要求不同 Backend 具有相同 Query 语法或调度算法。

## 10. Bevy Backend

### 10.1 边界与候选基线

`armillae-simulate-bevy` 只依赖 `bevy_ecs`，不隐式引入完整 `bevy::App`、渲染、窗口、输入或
资产栈。工作世界是一个 Bevy `World`；每个 Execute Entry 与 Clock Type 对应一张静态
Schedule，同类型不同 Clock Instance 共享 Schedule。

该 Adapter 的稳定身份为：

```rust
pub const BEVY_BACKEND_ID: &str =
    "armillae.simulate/bevy";
pub const BEVY_ENGINE_NAME: &str = "bevy_ecs";
```

`SimulationCapabilities.backend.id` 必须由 `BEVY_BACKEND_ID` 构造，`adapter_version` 是
`armillae-simulate-bevy` 自身 package version，`engine` 必须为
`Some(EngineInfo { name: BEVY_ENGINE_NAME, version: 精确的 bevy_ecs package version })`。
Native Module 的 `ExecutionPlane::Native.backend` 和 `adapter` 分别与这两个 Adapter 字段校验；
Backend ID 不符返回 `BackendMismatch`，版本要求不匹配返回 `IncompatibleAdapter`。
`supported` 必须包含 `armillae.simulate/native_modules` 和
`armillae.simulate/backend_native_access`，不得包含 `hosted_modules`；`parallel_systems` 只按第
10.7 节实际执行器条件报告。

本 API 设计以官方 `bevy_ecs 0.19.1` 为候选编译基线。官方 crate 元数据声明 Rust 1.95.0，
`Schedule::add_systems` 接受 `IntoScheduleConfigs<ScheduleSystem, M>`，`Schedule::run` 返回
`()`，Resource 在 0.19 中是 singleton Component。P0 Spike 仍必须在仓库工具链上编译本文
签名；在 Spike 完成前，“0.19.1”不是添加依赖的授权。

### 10.2 Builder 与 Native Module API

第一阶段 Bevy-native 公共接口冻结为：

```rust
pub struct BevySimulationBuilder {
    // private
}

impl Default for BevySimulationBuilder {
    fn default() -> Self;
}

impl BevySimulationBuilder {
    pub fn new() -> Self;

    pub fn register_module<M>(
        &mut self,
        module: M,
    ) -> Result<(), SimulationBuildError>
    where
        M: BevyModule;

    pub fn register_boxed_module(
        &mut self,
        module: Box<dyn BevyModule>,
    ) -> Result<(), SimulationBuildError>;

    pub fn activate(
        self,
    ) -> Result<BevySimulation, SimulationBuildError>;
}

pub trait BevyModule: Send + 'static {
    fn descriptor(&self) -> ModuleDescriptor;

    fn register(
        self: Box<Self>,
        registrar: &mut BevyModuleRegistrar<'_>,
    ) -> Result<(), SimulationBuildError>;
}

pub struct BevyModuleRegistrar<'a> {
    // private staging state
}

impl BevyModuleRegistrar<'_> {
    pub fn bind_clock<C>(
        &mut self,
        clock_type: &ClockTypeId,
    ) -> Result<(), SimulationBuildError>
    where
        C: Clock;

    pub fn add_system<M, S>(
        &mut self,
        system: &SystemId,
        implementation: S,
    ) -> Result<(), SimulationBuildError>
    where
        S: bevy_ecs::system::IntoSystem<(), (), M> + 'static;

    pub fn add_fallible_system<M, S>(
        &mut self,
        system: &SystemId,
        implementation: S,
    ) -> Result<(), SimulationBuildError>
    where
        S: bevy_ecs::system::IntoSystem<
                (),
                SystemExecutionResult,
                M,
            > + 'static;
}
```

`BevyModule` 使用 `self: Box<Self>`，因此既可通过泛型注册，也可把应用已编译的 Module 放进
`Vec<Box<dyn BevyModule>>` 后运行时选择。它仍是 Native Rust 接口，不构成动态库或跨语言 ABI。
每次 `register_module` / `register_boxed_module` 必须恰好调用一次 `descriptor()`，随后只使用该
次返回值完成 staging 和激活校验；不得重复调用并假定应用会返回相同描述。

Registrar 不暴露 `World` 或 `Schedule`，只向本次 Module 的 staging 写入：

- `bind_clock<C>` 只允许绑定当前描述中由该 Module 提供的 Clock；
- 一个 `ClockTypeId` 与一个 Rust `C` 一一对应；
- `ClockDefinition` 是动态协议的 Schema source of truth；Registrar 不要求它与
  `schemars::schema_for!(C)` 做 JSON 文本等值比较，因为开发者可以合法收紧领域约束，但
  开发者必须按第 8.2 节同步 `Clock::validate`；Schema 接受而 Rust decode 拒绝的漂移按第
  13.1 节 `armillae.simulate/codec` 处理；
- `add_system` 只接受返回 `()` 的 Bevy System；
- `add_fallible_system` 只接受返回 `SystemExecutionResult` 的 System，并自动接入 Armillae 失败
  收集；该显式 output bound 让 Adapter 在 Bevy 把结果交给 fallback handler 前通过 pipe 捕获；
- Bevy 函数直接返回 `bevy_ecs::error::Result<()>` 时会被 `IntoResult<()>` 视为 fallible
  `()` System，只能通过 `add_system` 进入 redacting fallback，不能产生 `SystemFailed` 的应用
  code；需要结构化错误的 Module 必须显式返回 `SystemExecutionResult`；
- 每个声明 System 必须恰好调用一次上述两种注册方法之一；
- System trigger 与 before/after 全部来自 `SystemDefinition`，Native 注册函数不再接受第二套
  ordering 参数；
- Module 注册期间不能创建应用实体或 Resource；初始化数据应在激活后、首次执行前通过
  `write_world` 或 typed Clock API 写入，因而单个 Module staging 可以整体丢弃。

`activate` 会在只含 Adapter 内部资源的工作世界上调用 `Schedule::initialize`，然后才返回可供
应用 `write_world` 的 Simulation。Bevy 在该步骤初始化 SystemParam；特别是 `Local<T>` 会调用
`T::FromWorld`。因此 Native Module 的 SystemParam/Local 初始化不得依赖激活后才插入的应用
Resource、Entity 或 Clock；普通 `Res`、`Query` 和 `NonSend` 的存在性仍在每次实际运行时验证。
`FromWorld` 只可初始化该 System 的私有参数状态，不得把其可执行的 World 写入当成 Module
生命周期钩子来创建应用事实；这类副作用没有跨 Backend 顺序或可移植性保证。应用初始化仍只
使用激活后的显式入口。
初始化 panic 在 unwind 构建中必须被捕获并转为 `SystemGraphBuildFailed`，code 固定为
`armillae.simulate/bevy_system_initialization_panicked`、message 固定为
`Bevy system initialization panicked`；`panic = "abort"` 仍不可恢复。

第一阶段不暴露 raw `Schedule` escape hatch。需要 run condition 的 System 应在函数内显式
return；若未来要暴露 `IntoScheduleConfigs`，必须先定义它与逻辑 System ID、错误收集和排序图
的组合规则。这里不取消 Bevy 自身的 SystemParam skipped 语义，其精确边界见第 10.6 节。

### 10.3 System 可见的运行上下文

```rust
#[derive(bevy_ecs::prelude::Resource)]
pub struct ExecuteContext {
    // private
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ExecuteOutputError {
    #[error("execute entry `{entry}` does not declare output")]
    NotDeclared { entry: ExecuteEntryId },

    #[error("execute entry `{entry}` output is already set")]
    AlreadySet { entry: ExecuteEntryId },

    #[error("failed to encode output for `{entry}`")]
    Encoding { entry: ExecuteEntryId },
}

impl From<ExecuteOutputError> for SystemExecutionError {
    fn from(error: ExecuteOutputError) -> Self;
}

impl ExecuteContext {
    pub fn request(&self) -> &ExecuteRequest;

    pub fn decode<T>(
        &self,
    ) -> Result<T, serde_json::Error>
    where
        T: serde::de::DeserializeOwned;

    pub fn set_output<T>(
        &self,
        output: &T,
    ) -> Result<(), ExecuteOutputError>
    where
        T: serde::Serialize + ?Sized;
}

#[derive(bevy_ecs::prelude::Component)]
pub struct ClockComponent<C: Clock> {
    // private
}

impl<C: Clock> ClockComponent<C> {
    pub fn instance(&self) -> &ClockInstanceId;
    pub fn value(&self) -> &C;
    pub fn value_mut(&mut self) -> &mut C;
}

#[derive(bevy_ecs::prelude::Resource)]
pub struct AdvanceContext<C: Clock> {
    // private
}

impl<C: Clock> AdvanceContext<C> {
    pub fn clock_type(&self) -> &ClockTypeId;

    pub fn transitions(
        &self,
    ) -> &[TypedClockTransition<C>];
}
```

Adapter 在 Execute Schedule 前插入唯一 `ExecuteContext`，结束后移除；在 typed Clock 写入完成后
插入对应 `AdvanceContext<C>`，运行该 Clock Type Schedule 后移除。上下文作为 Bevy Resource
只通过 `Res` 获取；`set_output` 使用内部线程安全 single-assignment cell，不要求 `ResMut`。
每次调用先原子认领唯一写入机会，再编码值。cell 必须永久记录 `NotDeclared`、`Encoding` 或
`AlreadySet`；即使 System 忽略该 Result，Adapter 仍在 Schedule 后分别返回
`UnexpectedExecuteOutput`、`ExecuteOutputEncodingFailed` 或 `ConflictingExecuteOutput` 并
Fault。若多个并行调用产生多类违规，未声明 output 时固定选择 `UnexpectedExecuteOutput`；
已声明时 conflict 优先于 encoding，因此结果不依赖竞争到达顺序。
Encoding 变体不得携带或格式化自定义 `Serialize` error，因为该字符串可能包含 output 数据；
诊断只保留 entry 和错误类别。

`From<ExecuteOutputError>` 分别使用 well-known `SystemErrorCode`
`armillae.simulate/execute_output_not_declared`、
`armillae.simulate/execute_output_already_set` 和
`armillae.simulate/execute_output_encoding`，便于 fallible System 使用 `?`。output sink 的
独立记录仍是最终分类事实，不能因 System 是否传播该错误而改变。System 不得通过
`Changed<ClockComponent<C>>` 猜测本次目标。

`ClockComponent<C>` 的 instance 字段不可修改，避免逻辑 ID 与内部索引分离。`value_mut` 是有意
保留的 Bevy-native 逃生舱：直接修改不会运行 `Clock::validate`、不会产生 Advance Context，也
不会触发任何 Schedule。需要 Advance 语义时必须调用 Simulation API。用户直接 despawn 或替换
受管理 Clock Component 会破坏 Adapter 索引；下一次检测到该事实时必须返回 Backend failure
并 Fault，而不是重建一个猜测身份。

### 10.4 Concrete Simulation 扩展面

```rust
pub struct BevySimulation {
    // private
}

impl Simulation for BevySimulation {
    // 第 9.2 节全部方法
}

impl BevySimulation {
    pub fn insert_clock_typed<C>(
        &mut self,
        instance: ClockInstanceId,
        value: C,
    ) -> Result<(), SimulationError>
    where
        C: Clock;

    pub fn read_clock_typed<C>(
        &self,
        instance: &ClockInstanceId,
    ) -> Result<C, SimulationError>
    where
        C: Clock;

    pub fn remove_clock_typed<C>(
        &mut self,
        instance: &ClockInstanceId,
    ) -> Result<C, SimulationError>
    where
        C: Clock;

    pub fn advance_typed<C>(
        &mut self,
        request: TypedAdvanceRequest<C::Step>,
    ) -> Result<TypedAdvanceOutcome<C>, SimulationError>
    where
        C: Clock;

    pub fn inspect_world<R>(
        &self,
        inspect: impl for<'w> FnOnce(&'w bevy_ecs::world::World) -> R,
    ) -> Result<R, SimulationError>;

    pub fn write_world<R>(
        &mut self,
        write: impl for<'w> FnOnce(&'w mut bevy_ecs::world::World) -> R,
    ) -> Result<R, SimulationError>;
}
```

Typed 与 JSON 方法必须访问相同 Clock 实体、索引和 Schedule。typed 方法以 Rust 类型保证结构
并调用相同 `Clock::validate` / `Clock::advance`，省去动态请求的 JSON Schema 与 serde 边界；
第 8.2 节的 Module 一致性义务保证两条路径的领域接受集合相同。typed 方法不改变批次顺序、
Clock 写入或 System/Fault 状态语义。`read_clock_typed` 返回 clone，不返回 ECS borrow。若 Rust
`C` 未在
激活图中绑定，typed 方法返回 `NativeClockTypeNotBound { rust_type: type_name::<C>() }` 并保持
当前状态；不得把它误报为一个不存在的逻辑 `ClockTypeId` 或使 World Fault。

World 访问使用 closure 而不是 `world()` / `world_mut()`，使安全 Rust 不能把借用作为返回值
逃出调用作用域或跨 `await`。`write_world` 只解释生命周期和 panic，不解释用户 closure 的
领域返回值；需要领域错误时令 `R = Result<T, E>`，调用方自行处理内层 Result。普通领域
`Err` 不会自动回滚 closure 已完成的写入，也不会自动 Fault。

Bevy 0.19.1 为 `World` 显式实现 `Send`，因此 `BevySimulation` 可以满足核心 Trait；这不取消
Bevy `NonSend` 数据的线程亲和性。应用必须先把 Simulation 移动到最终执行线程，再通过
`write_world` 插入 `NonSend` 数据，并在其余生命周期内从同一线程执行会访问该数据的闭包或
Systems。跨线程移动本身在 Rust 中安全，但随后从不同线程访问 `NonSend` 会由 Bevy panic；
Adapter 必须在 `panic = "unwind"` 下把它捕获为 `BackendPanicked` 并 Fault，不能把 `Send`
解释为 `NonSend` 可以跨线程访问。Actor/Binding 因此应在所有权线程完成激活后的应用初始化。

### 10.5 Schedule 与排序映射

- Adapter 为每个 `ExecuteEntryId` 与 `ClockTypeId` 创建内部 Schedule label，不暴露 label；
- 每个逻辑 `SystemId` 映射为一个私有 Bevy SystemSet，before/after 边映射为 set ordering；
- 激活时调用 `Schedule::initialize`，构建错误映射为 `SystemGraphBuildFailed`；
- `set_apply_final_deferred(true)` 是固定契约，成功前必须应用 `Commands`；
- 未声明顺序的无冲突 Systems 可以并行，执行和迭代顺序不稳定；
- 发生访问冲突时 Bevy 可以串行化，但业务结果不得依赖未声明顺序；
- 同类型所有 Clock Instance 共享 Schedule，不得按实例复制；
- Bevy change detection 只可优化 System 内查询，不是触发协议、事务日志或持久化 Revision。

### 10.6 Bevy 错误边界

Bevy 0.19.1 的 `Schedule::run(&mut World)` 返回 `()`；fallible System 默认把错误交给
`FallbackErrorHandler`，因此 Adapter 必须遵循以下精确策略：

Bevy `SystemParamValidationError` 明确标记为 skipped 的结果沿用 Bevy 语义：该 System 本次不
运行，且不产生 Armillae error。它不同于交给 fallback handler 的实际错误。Native Module 若
要求缺少目标时得到结构化失败，不得依赖 `Single`、`If` 等 skip 行为，应使用可选/普通 Query
自行检查并返回 `SystemExecutionError`。

1. `add_fallible_system` 使用显式 `IntoSystem<(), SystemExecutionResult, M>` output，把该 output
   pipe 到 Adapter 内部的线程安全 operation collector，保留逻辑 `SystemId`，再把 pipe 后的
   `()` System 加入 Schedule；该结果不进入 fallback handler；
2. collector 在每次操作开始前清空；`&mut self` 保证同一 Simulation 不存在并发 operation；
3. 捕获到 `SystemExecutionError` 不会短路当前 Schedule；pipe 返回 `()` 后，其余已调度 Systems
   仍按图执行，final deferred 仍在 Schedule 正常结束时应用。第一阶段只承诺终止结果为失败，
   不承诺“首错即停”；
4. Schedule 结束后只要 collector 非空，按 `SystemId` 字节序排序并返回最小 ID 对应的
   `SystemFailed`，同时将实例置为 Faulted；其余失败只用于脱敏诊断，不改变“一个终止错误”，
   也不把并行到达顺序伪装成稳定顺序；
5. Adapter 在 World 创建时安装、并在每次可能执行用户代码的写边界前恢复私有的 redacting
   fallback handler；该 handler 接收但不格式化、不记录原始 `BevyError` / `ErrorContext`，只用
   无数据的私有 marker 触发 unwind。该 `FallbackErrorHandler` Resource 归 Adapter 所有，用户
   通过 `write_world` 替换或删除它不会改变下一次边界的策略；未显式捕获的 Bevy System、
   Observer 或 Command 错误因此不能只记录后仍返回成功；
6. 执行边界捕获到私有 marker 时返回 code 为
   `armillae.simulate/unhandled_bevy_error`、message 固定为 `unhandled Bevy execution error` 的
   `BackendFailure` 并 Fault；所有可能调用用户 `Clock`、serde、Schema validator、Schedule、
   `inspect_world` 或 `write_world` 代码的 API 边界都捕获其它 unwind payload，不格式化或返回
   payload，改为 `BackendPanicked` 并 Fault；`panic = "abort"` 不可恢复；
7. 不得使用 `catch_unwind` 后继续访问或尝试修补发生 panic 的 World。

这意味着生产 Module 应使用 `add_fallible_system` 和 `SystemExecutionError` 表达需要进入
Armillae 错误模型的失败，并显式处理 fallible deferred Commands。`add_system` 的 `()` 只表示
该 System 不声明 Armillae 结构化失败；其 Bevy 参数、Observer、Command 或其它未捕获错误走
redacting fallback 边界。第一阶段 System 失败一律 Fault，不提供“报错但继续使用原 World”的
假保证。
Native System 自行产生的数据库、网络或其它外部副作用同样不会回滚，而且显式 System error
不会令同一 Schedule 首错即停；需要严格副作用顺序、幂等或补偿的应用必须在自身 Driver/Harness
中治理，不能从 Simulation Fault 推导外部世界未改变。

`catch_unwind` 不会抑制进程级 panic hook。Adapter 自己为 fallback 产生的 marker 不含领域
数据；任意 Native 用户代码主动 panic 时，panic hook 是否输出其 payload 仍由宿主进程控制，
Module 不得把 Secret、完整输入或世界数据放进 panic payload。Adapter 不得为局部执行临时替换
全局 panic hook。

### 10.7 版本与 Features

P0 Spike 必须验证以下候选配置，而不是直接写入 manifest：

| 项目 | 候选值 |
|---|---|
| engine | `bevy_ecs = "=0.19.1"` |
| engine MSRV | Rust 1.95.0 |
| required feature | `std` |
| Adapter additive feature | `parallel` -> `bevy_ecs/multi_threaded` |
| 非必需默认能力 | `bevy_reflect`、`async_executor`、`backtrace`、`serialize` |

最终应使用 `default-features = false` 和 Spike 证明必要的最小 feature 集合。若 `parallel` 未启用
或执行器未实际并行，Backend 不得报告 `armillae.simulate/parallel_systems`。Cargo feature
只表达可叠加能力，不选择 Bevy 版本。`armillae-simulate` 和
`armillae-simulate-bevy` 第一阶段都使用空的 crate default feature 集；核心的 `testing` 与
Adapter 的 `parallel` 均需下游显式启用，`std` 是首阶段实现前提而不是可关闭的 no-std 开关。
Adapter 未启用 `parallel` 时必须显式选择 single-threaded executor，即使依赖图中的其它 crate
因 Cargo feature union 打开了 `bevy_ecs/multi_threaded`；启用 `parallel` 且目标支持时才选择
multi-threaded executor 并报告 capability。

每次 Bevy 升级都需要新的 Adapter 发布、Native API 迁移说明、共享合约和专项测试；不使用
`bevy-019`、`bevy-020` 等互斥 features。必须并行维护不兼容版本时优先维护 Adapter 发布线，
只有同一应用必须同时依赖两代 Bevy 的真实需求才评估独立包名。

## 11. Binding、Tool 与 Agent 集成边界

### 11.1 Binding 与 Hosted 的区别

`ModuleDescriptor`、Execute/Clock/Advance 和 Capabilities 都是拥有所有权、可 Serde/Schema 的
协议，因此 N-API、PyO3 与 Wasm Binding 可以在不暴露 Bevy 借用的前提下包装
`Box<dyn Simulation>`。Binding 可以：

- 创建由宿主预先注册好 Native Module 的 Simulation；
- 调用 Execute、Clock 管理、Advance、status 和 stop；
- 把 JSON 对象映射为后端中立请求与结果；
- 在自己的事件循环中决定锁、队列和异步包装方式。

这不等于脚本语言已经可以定义 ECS System。让 JavaScript、Python 或 Wasm 在运行前提供新的
Hosted Module 仍需要批量 Query、ChangeSet、超时、能力、资源配额与 Loader ABI；这些属于后续
Active Spec。第一阶段 Binding 不得把任意跨语言 callback 放进 Bevy Schedule 并宣称实现了
Hosted Module。

### 11.2 Tool 与 Agent

Simulation 不提供 Agent Runner 或 Tool Loop。下游可以自行组合：

```text
Agent Harness
    -> LlmBridge.complete
    -> 用户决定是否执行 ToolCall
    -> ToolExecutor.execute
    -> Tool 从 ToolContext 取得应用提供的世界写入句柄
    -> 应用句柄调用 Execute、Advance 或 Backend-native 写入
```

`armillae-tools` 与 `armillae-simulate` 不互相依赖。Tool 使用何种句柄、是否加锁、是否排队、
是否允许并发以及如何把结果返回模型，全部由下游拥有。

## 12. 持久化兼容边界

当前 Spec 不提供保存或加载接口，也不创建状态 crate。为了避免未来无法接入持久化，实现必须
遵守：

- 不把 Bevy `Entity`、archetype、change tick 或 Schedule 状态暴露为稳定协议；
- 不声称直接序列化 Bevy World 就是长期兼容存档；
- 将缓存、索引、锁、任务和宿主句柄视为可重建或临时状态；
- 为未来从外部状态视图创建新工作世界保留生命周期入口；
- 工作世界进入 Faulted 后可以整体丢弃，不要求在原对象上继续运行；
- 用户若希望未来无损恢复，不能把影响后续行为的唯一事实隐藏在不可导出的 System Local 或
  Backend 私有缓存中。

未来状态 RFC 可以要求更强的物化、变更捕获和提交协议；届时必须先更新设计入口、本 Spec 和
Backend 合约测试，再实施集成。

## 13. Runtime 错误协议

### 13.1 精确类型

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SimulationOperation {
    Execute,
    ReadClock,
    InsertClock,
    RemoveClock,
    Advance,
    InspectWorld,
    WriteWorld,
    Stop,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaViolation {
    pub instance_path: String,
    pub schema_path: String,
    pub keyword: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdvanceRequestViolation {
    EmptyTargets,
    DuplicateInstance { instance: ClockInstanceId },
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SimulationError {
    #[error("cannot perform {operation:?} while simulation is {status:?}")]
    InvalidState {
        operation: SimulationOperation,
        status: SimulationStatus,
    },

    #[error("cannot perform {operation:?} because simulation is faulted")]
    Faulted {
        operation: SimulationOperation,
    },

    #[error("unknown execute entry `{entry}`")]
    UnknownExecuteEntry {
        entry: ExecuteEntryId,
    },

    #[error("unknown clock type `{clock_type}`")]
    UnknownClockType {
        clock_type: ClockTypeId,
    },

    #[error("native clock type `{rust_type}` is not bound")]
    NativeClockTypeNotBound {
        rust_type: &'static str,
    },

    #[error("unknown clock instance")]
    UnknownClockInstance {
        key: ClockKey,
    },

    #[error("clock instance already exists")]
    DuplicateClockInstance {
        key: ClockKey,
    },

    #[error("invalid execute input for `{entry}`")]
    InvalidExecuteInput {
        entry: ExecuteEntryId,
        violations: Vec<SchemaViolation>,
    },

    #[error("execute entry `{entry}` does not declare output")]
    UnexpectedExecuteOutput {
        entry: ExecuteEntryId,
    },

    #[error("failed to encode output for `{entry}`")]
    ExecuteOutputEncodingFailed { entry: ExecuteEntryId },

    #[error("execute entry `{entry}` did not produce required output")]
    MissingExecuteOutput {
        entry: ExecuteEntryId,
    },

    #[error("execute entry `{entry}` produced output more than once")]
    ConflictingExecuteOutput {
        entry: ExecuteEntryId,
    },

    #[error("invalid execute output for `{entry}`")]
    InvalidExecuteOutput {
        entry: ExecuteEntryId,
        violations: Vec<SchemaViolation>,
    },

    #[error("invalid clock value")]
    InvalidClockValue {
        key: ClockKey,
        violations: Vec<SchemaViolation>,
    },

    #[error("clock value rejected: {code}: {message}")]
    ClockValueRejected {
        key: ClockKey,
        code: ClockErrorCode,
        message: String,
    },

    #[error("invalid advance request")]
    InvalidAdvanceRequest {
        clock_type: ClockTypeId,
        reason: AdvanceRequestViolation,
    },

    #[error("invalid clock step")]
    InvalidClockStep {
        clock_type: ClockTypeId,
        instance: ClockInstanceId,
        violations: Vec<SchemaViolation>,
    },

    #[error("clock transition failed: {code}: {message}")]
    ClockTransitionFailed {
        clock_type: ClockTypeId,
        instance: ClockInstanceId,
        code: ClockErrorCode,
        message: String,
    },

    #[error("system `{system}` failed: {code}: {message}")]
    SystemFailed {
        system: SystemId,
        trigger: SystemTrigger,
        code: SystemErrorCode,
        message: String,
    },

    #[error("backend `{backend}` failed: {code}: {message}")]
    BackendFailure {
        backend: BackendId,
        operation: SimulationOperation,
        code: String,
        message: String,
    },

    #[error("backend `{backend}` panicked during {operation:?}")]
    BackendPanicked {
        backend: BackendId,
        operation: SimulationOperation,
    },
}
```

`SchemaViolation.instance_path` 和 `schema_path` 使用 RFC 6901 JSON Pointer，根位置编码为
空字符串；`keyword` 是触发失败的 Draft 2020-12 keyword，validator 无法提供时为 `None`。
该结构不复制失败值。Schema 已接受但 Native Clock 的 serde 编码或解码失败时，使用
`ClockValueRejected` 或 `ClockTransitionFailed`，稳定 code 为 `armillae.simulate/codec`；
message 固定为 `clock codec failed`，不得格式化底层 serde error。这通常表示 Module Schema、
Rust 类型或自定义 serde 实现漂移。所有动态 Clock 编码都必须在对应写入或删除前完成，因此
该类错误不导致 Fault，并保持调用前的 Active 或 Stopped 状态。

### 13.2 错误后的状态

| 错误类别 | 操作前 World 是否改变 | 返回后的状态 |
|---|---|---|
| `InvalidState` / `Faulted` / unknown / duplicate / native type 未绑定 | 否 | 保持原状态 |
| Schema / batch request validation | 否 | Active |
| `ClockValueRejected` / `ClockTransitionFailed` | 否 | 保持原状态 |
| Execute output 的 unexpected/encoding/missing/conflicting/invalid | 可能 | Faulted |
| `SystemFailed` | 可能 | Faulted |
| `BackendFailure` / `BackendPanicked` | 不可证明 | Faulted |

首个导致 Fault 的调用返回对应的具体 Execute output、`SystemFailed`、`BackendFailure` 或
`BackendPanicked` 变体；之后所有读取和写入返回 `Faulted`，不重复泄漏原始故障。Stopped 的
非法操作返回 `InvalidState`。

第一阶段没有 `Cancelled` 变体，因为同步 `&mut self` 方法没有诚实的中途取消机制。未来只有在
Backend 可以定义安全点、资源释放和 World 一致性后才能加入取消；不得预先保留一个永远无法
正确产生的错误变体。

错误 Display、Debug、日志和 tracing 不得默认携带完整 Component、Clock value/step、Execute
input、Tool 参数、Agent 对话或 panic payload。外部输入和 Backend 失败不得用 `unwrap()` 或
无上下文 `expect()` 转为 panic。

### 13.3 校验与错误优先级

为保证 Backend 合约一致，错误选择顺序固定：

1. 所有可能访问 World 的方法先校验生命周期；
2. Execute 依次校验 entry 是否存在、input Schema；Schedule 后先检查 output sink：未声明 output
   时 attempted write 固定为 `UnexpectedExecuteOutput`，已声明 output 时依次检查 conflict、
   encoding；随后依次检查 System failure、output 是否缺失和 output Schema，使传播或忽略
   `ExecuteOutputError` 都得到相同分类；
3. insert 依次校验 Clock Type、重复 key、value Schema、decode 与 `Clock::validate`；
4. read/remove 依次校验 Clock Type、instance、value encode；remove 只在 encode 成功后删除；
5. Advance 依次校验 Clock Type、非空 targets、重复 instance，然后按 target 输入顺序校验
   instance、Step Schema、decode、current value 和 transition；
6. 预计算阶段存在多个 target 错误时返回输入顺序中的第一个；
7. System 阶段存在多个显式错误时按第 10.6 节选择字节序最小的 `SystemId`；
8. typed Clock API 在生命周期后、实例查询前校验 Rust `C` 是否已绑定；未绑定返回
   `NativeClockTypeNotBound`，保持原状态；
9. Backend invariant 或 panic 一旦出现，覆盖尚未返回的普通错误并 Fault。

实现不得为了少一次查找而改变上述优先级；否则相同请求在不同 Backend 上会产生不同公共事实。
所有 `SchemaViolation` 必须按 `(instance_path, schema_path,
keyword.as_deref().unwrap_or(""))` 的字符串字节序稳定排序并去重；validator 的内部遍历顺序不得
成为协议差异。

## 14. 确定性与可观测性

### 14.1 确定性

使用 Bevy Schedule 不自动构成确定性保证。任何确定性声明必须说明：

- System 的显式顺序和允许并行集合；
- 同时写入的合并规则；
- 随机源与种子归属；
- 浮点和平台差异；
- 外部输入及其归档方式；
- Backend 与精确版本。

首个实现可以只声明“未提供跨平台确定性保证”，但不得暗示默认并行顺序可重放。

### 14.2 可观测性

每次已开始的 Execute 或 Advance 必须产生唯一终止结果：成功或一个结构化错误。实现可以记录
入口、Clock 逻辑 ID、耗时、System 名称和错误类别，但默认不得记录完整应用组件、Hosted
批量输入、Tool 参数或 Agent 对话。请求 correlation ID 属于下游调用上下文，不由 Simulation
自动生成或写进领域协议。

## 15. 合约测试

### 15.1 共享测试入口与 Scripted Test Double

`armillae-simulate` 的 additive `testing` feature 提供测试支持，但不改变生产 Trait：

```rust
pub mod testing {
    use super::*;

    #[derive(Clone, Debug)]
    pub struct ContractFixture {
        pub module: ModuleDescriptor,
        pub execute_request: ExecuteRequest,
        pub primary_clock: ClockState,
        pub secondary_clock: ClockState,
        pub probe_clock: ClockState,
        pub advance_request: AdvanceRequest,
    }

    pub fn standard_fixture(
        execution: ExecutionPlane,
    ) -> ContractFixture;

    pub trait BackendContractFactory: Send + Sync {
        fn capabilities(&self) -> SimulationCapabilities;

        fn execution_plane(&self) -> ExecutionPlane;

        fn create_fixture(
            &self,
            fixture: ContractFixture,
        ) -> Result<Box<dyn Simulation>, ContractSetupError>;
    }

    #[derive(Clone, Debug, thiserror::Error)]
    #[error("contract setup failed: {code}: {message}")]
    pub struct ContractSetupError {
        pub code: String,
        pub message: String,
    }

    #[derive(Clone, Debug, thiserror::Error)]
    #[error("backend contract `{case}` failed: {message}")]
    pub struct ContractViolation {
        pub case: String,
        pub message: String,
    }

    pub fn assert_backend_runtime_contract(
        factory: &dyn BackendContractFactory,
    ) -> Result<(), ContractViolation>;

    #[derive(Clone, Debug, PartialEq)]
    #[non_exhaustive]
    pub enum RecordedSimulationCall {
        Execute(ExecuteRequest),
        ReadClock(ClockKey),
        InsertClock(ClockState),
        RemoveClock(ClockKey),
        Advance(AdvanceRequest),
        Stop,
    }

    #[derive(Debug)]
    #[non_exhaustive]
    pub enum ScriptedReply {
        Execute(Result<ExecuteOutcome, SimulationError>),
        ReadClock(Result<ClockState, SimulationError>),
        InsertClock(Result<(), SimulationError>),
        RemoveClock(Result<ClockState, SimulationError>),
        Advance(Result<AdvanceOutcome, SimulationError>),
    }

    pub struct ScriptedSimulation {
        // private
    }

    impl ScriptedSimulation {
        pub fn new(
            capabilities: SimulationCapabilities,
            replies: impl IntoIterator<Item = ScriptedReply>,
        ) -> Self;

        pub fn calls(&self) -> Vec<RecordedSimulationCall>;
    }

    impl Simulation for ScriptedSimulation {
        // records each call and consumes one matching reply
    }
}
```

Script mismatch 或耗尽必须返回 code 为 `armillae.simulate/mock_script_mismatch` 的
`BackendFailure` 并进入 Faulted，不能 panic。`ScriptedSimulation` 只验证 Driver/Harness 的
请求编排，不执行 Module/System，也不能用于宣称某 Backend 符合规范。

`ScriptedSimulation` 初始为 Active，capabilities 始终返回构造值。除 `status` / `capabilities`
外，每次方法调用都先记录并校验生命周期；被拒绝的调用不消费 reply。Active 的数据调用以及
Stopped 中合法的 `read_clock` 只消费队首且方法种类相同的 reply；表 13.2 中的 fatal error
转为 Faulted，其它结果保持原状态。`stop` 不消费 reply：Active 时直接转为 Stopped，Stopped
时幂等成功，Faulted 时按矩阵拒绝。测试可以脚本化领域成功或失败，但不能用 reply 绕过生命
周期矩阵。

`assert_backend_runtime_contract` 先取得 Factory 声明的 capabilities 和 execution plane，以
后者调用 `standard_fixture`，并验证新实例报告相同 capabilities；Factory 不得重写 fixture
中的 Backend ID 或 Adapter requirement。Native Factory 应返回与自身精确 Adapter version
匹配的 `ExecutionPlane::Native`，Hosted Factory 只有在确实支持稳定 Hosted ABI 时才返回
`Hosted`。Helper 可以为相互隔离的 case 多次调用 `create_fixture`；不得在 Stopped 或 Faulted
实例上继续测试后续成功路径。

`standard_fixture` 使用 JSON Object `{ "value": i64 }` 表示 Counter/Probe Clock，使用
`{ "delta": i64 }` 表示 Step/Execute input；Counter transition 为 checked addition。它包含
两个同类型 Counter instance 和一个独立 Probe Clock。Execute System 按 input 增加 Probe，并
把更新后的 Probe value 设置为 output；Counter Advance response System 每次运行增加 Probe
一次。Factory 用自己的 Native API 绑定这些明确行为；`assert_backend_runtime_contract` 只通过
`dyn Simulation` 观察 Counter 和 Probe，
从而同时验证无隐式推进、实例隔离和响应 System 副作用。

Fixture 的逻辑身份固定为：

| 类型 | 值 |
|---|---|
| Module / version | `armillae.simulate.contract/fixture` / `1.0.0` |
| Execute entry | `armillae.simulate.contract/increment_probe` |
| Counter Clock Type | `armillae.simulate.contract/counter` |
| Probe Clock Type | `armillae.simulate.contract/probe` |
| Counter instances | `primary`、`secondary` |
| Probe instance | `probe` |
| Execute System | `armillae.simulate.contract/system/increment_probe` |
| Advance System | `armillae.simulate.contract/system/count_advance` |

上述 ID、Schema 和行为属于测试协议；Factory 不得替换为自己的命名，否则不同 Backend 的
合约结果不可比较。

### 15.2 所有 Backend 共享

共享验证分成三层，不能用一个 `dyn Simulation` helper 假装覆盖 Backend-native 注册：

- 前三项由 `armillae-simulate` 自身的协议单元测试执行；
- Building、Module 原子性、依赖/排序与能力拒绝由每个 Adapter 使用自己的 Native Module API
  跑同形 builder suite；核心不为此发明通用 System ABI；
- `assert_backend_runtime_contract` 使用 standard fixture 覆盖 capabilities、Execute 成功与输入
  拒绝、Clock 管理与批量预计算、顺序、响应副作用、无隐式推进和 Stop 状态；需要注入 output、
  System、Backend 或 panic 故障的条目由各 Adapter 专项 suite 按相同公共错误语义补齐。

- 所有透明 ID 的构造、反序列化非法值和 Serde round-trip；
- 所有协议根类型的 JSON Schema 快照和规范 JSON round-trip；
- Schema violation 的去重与规范排序不受 validator 遍历顺序影响；
- Building 阶段可注册，Active 后拒绝改变 Module 集合；
- Module 校验失败不留下部分注册；
- Native Module descriptor/register unwind panic 返回稳定 build error，且不污染 Builder；
- 缺失/不兼容依赖、未知 trigger、跨 trigger ordering 和 cycle 分类正确；
- Execute 不自动推进 Clock；
- Execute 输入先校验，失败时 probe 不执行；
- 未声明 output 返回 `None`；声明 output 时恰好一次写入并通过 Schema；
- output 未声明写入、编码失败、缺失、重复或 Schema 无效时进入 Faulted，且忽略
  `set_output` 错误不能绕过 sink 检测；
- 不调用 Advance 时 Clock 不自行变化；
- 单 Clock 推进只运行对应 Clock Type 的响应 Systems；
- 同类型多个 Clock Instance 保持独立；
- 空 targets、重复 instance、未知 instance 和非法 Step 在修改前失败；
- Clock serde codec 失败使用稳定 code，且 read/remove/Advance 不留下部分修改；
- Clock/serde/Schema validator unwind panic 不逃出 API，并进入 `BackendPanicked` / Faulted；
- 多 target transition 和 outcome 保持输入顺序；
- 任一 target 预计算失败时整批 Clock 值不变；
- 没有响应 System 时只更新目标 Clock；
- 响应 System 可以修改工作世界；
- 后续 Clock 不被隐式递归推进；
- 每个请求只有一个终止结果；
- Clock transition 失败保持 Active，System/Backend 失败进入 Faulted；
- Stopped 允许 read、拒绝写，重复 stop 幂等；
- Faulted 拒绝 read 与 write；
- Backend 能力不足时在激活前失败。

### 15.3 Bevy Adapter 专项

- Query 读取与写入访问正确；
- 无冲突 Systems 可按配置并行，冲突访问不会违反借用规则；
- 显式 before/after 顺序得到执行；
- `Commands` 在成功返回前应用；
- 多 Clock Instance 不创建逐实例静态 Schedule；
- Execute/Advance Context 在调用期间可见且结束后移除；
- `Schedule::initialize` 的 `Local<T>::FromWorld` 时序只看到 Adapter 初始化世界；初始化 panic
  返回稳定 build error，而不是产生部分 Simulation；
- `Changed<T>` 不被用作唯一 Advance 触发；
- Bevy `Entity` 不进入后端中立请求、结果或序列化快照；
- `inspect_world` / `write_world` 的借用通过 compile-fail 测试证明不能逃逸或跨 `await`；
- typed 与 JSON Clock API 指向同一实体和索引；
- typed Clock 使用未绑定 Rust 类型时保持状态并返回 `NativeClockTypeNotBound`；
- 语义等价且通过动态边界的 JSON 与 typed Advance 具有相同 Clock/System 状态结果；Schema 与
  codec 错误只属于 JSON 边界；
- `add_fallible_system` 保留逻辑 System ID、`SystemErrorCode` 并 Fault；
- 多个 fallible System 失败时 Schedule 不以到达顺序短路，并稳定选择字节序最小 System ID；
- Bevy 明确标记的 skipped SystemParam 是正常 no-op，非 skipped validation error 进入 redacting
  fallback；
- 未捕获 Bevy error 经无数据 marker 映射为稳定 `BackendFailure`，其它 unwind panic 映射为
  `BackendPanicked`，Adapter 对二者都不返回或记录原 payload；
- 编译期断言 `BevySimulation: Send`；Simulation 移到最终所有权线程后使用 `NonSend` 正常；
  错误线程访问会 Fault 而非返回成功；
- `panic = "abort"` 限制在 crate 文档中明确；
- 精确 Bevy 版本升级时完整复跑共享与专项测试。

### 15.4 Tool 与 Binding 组合场景

- 应用能够把自有 Simulation 写入句柄通过 `ToolContext` 透传；
- Tool 可以经该句柄直接修改组件或请求 Advance；
- ToolExecutor 不需要依赖 Simulate；
- 多 Tool 调度顺序不由 Simulate 合约测试假定；
- JSON Binding 可以调用 object-safe API，而不暴露 Bevy World 借用；
- Binding 不把普通 callback 宣称为 Hosted Module。

## 16. 实施门禁

以下设计门禁已经完成：公共 ID/版本/wire types、object-safe `Simulation`、Module 与 Bevy
Native API、同类型批量 Advance、Faulted 语义，以及 Scripted Test Double/共享合约入口均已
由本 Spec 冻结。

创建产品 crate 前仍必须完成 Bevy P0 Spike：

1. 在 `.agents/spikes/` 提交验证记录；
2. 用 `bevy_ecs = "=0.19.1"` 候选版本编译第 10 节全部签名；
3. 验证 Rust 1.95.0、`default-features = false`、`std` 和 additive `parallel` feature；
4. 验证 SystemSet ordering、`Schedule::initialize`、`Local<T>::FromWorld` 初始化时序和 final
   deferred application；
5. 验证 Module 注册/初始化与执行边界的 `catch_unwind`、`SystemExecutionResult` 显式 output
   pipe、redacting fallback marker 分类和 Faulted 状态；
6. 验证 `World: Send`、`NonSend` 线程亲和性、single-thread、multi-thread 和目标 Wasm 构建
   边界；
7. 若事实与本文不符，先修订 Spec，不得在实现中静默偏离。

Spike 完成且用户明确授权实现后，必须使用 Cargo CLI 创建 crate 和添加依赖，并同步检查实施
清单、用户文档计划和发布元数据。当前文档授权不包含产品代码或 manifest 修改。

Hosted Loader、持久化和 Agent Runtime 不阻塞第一阶段 Native Simulate，但也不得在第一阶段
实现中以临时接口提前冻结。

## 17. 实现与文档状态

当前仓库尚不存在 `armillae-simulate` 或 `armillae-simulate-bevy`，也没有任何实现可以被视为
符合本 Spec。根 README 和用户文档在首个可用实现与端到端示例通过前不得宣称该能力已经发布。

本 Spec 的实施差异由 [Simulate TODO](../todos/simulate.md) 跟踪；持久化不进入该清单。
