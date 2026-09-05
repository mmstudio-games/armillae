# Armillae 上下文组织与压缩（armillae-context）规范

> 状态：Active Spec；首阶段实现完成并已通过 DeepSeek 官方 Live 验证（2026-08-28）
> 规范基线：2026-08-28
> 适用范围：`armillae-context` crate（首阶段已实现并 Live 验证通过）
> 设计入口：[Armillae 设计索引](../DESIGN.md)
> 决策来源：[RFC 0004：Armillae 上下文组织与压缩](../rfcs/0004-context.md)
> 实施清单：[todos/armillae-context.md](../todos/armillae-context.md)

本文是 `armillae-context` 子系统的实施依据。它冻结薄 `Context` 契约（只生产可推理的
上下文）、范式黑盒实现、压缩管道三态语义、导出契约、错误分类和合约测试。持久化由各范式
自行定义（本 Spec 只冻结小节范式的 `SectionStore` 契约作为示例）。没有被本文明确纳入的
Agent Harness、Turn 流程、压缩 LLM 执行、并发协调行为不得由实现自行补全。

本文中的"必须""不得"和"只"属于规范要求；明确标记为"实施门禁"或"后续范围"的内容尚未授权
对应产品实现。本文已经冻结公共 Rust 标识符、对象安全接口、压缩管道语义、导出与错误契约
形状；实现不得以"内部细节"为由改变这些契约。

## 1. 范围

### 1.1 第一阶段目标

- 提供薄的 `Context` trait 契约：只专注生产可推理的上下文（对话写入 / 写回 / 导出 +
  压缩管道）；
- 提供范式黑盒实现：当前内置 `SectionContext`（小节范式）；`TraditionalContext`（传统
  范式）为规划中（后续范围），新范式 = 新 struct + `impl Context` + 自己的 Config 与
  持久化接口；
- 提供压缩管道（评估 → 准备 → 下游推理 → 提交），执行外包下游、压缩指令由范式组装
  （下游零组装推理）；
- 提供压缩管道三态语义（空闲 / 已评估 / 已准备），保证评估时效与操作序列合法；
- 提供稳定前缀在前的导出（core `Vec<Message>`；v1 不含缓存断点）；
- 持久化由范式自行定义（小节范式提供 `SectionStore` 契约，由该范式的下游实现）；
- 提供 token 计数内部化（以 `usage.input_tokens` 为上下文规模事实）。

### 1.2 明确非目标

- Agent、Turn Runner、自动 Tool Loop、Memory、RAG；
- 在 `armillae-context` 内执行压缩 LLM 推理（由下游显式驱动）；
- 跨范式的持久化契约（持久化归各范式自行定义）；
- 并发与调度策略（范式实例的使用方式由范式与其下游约定）；
- 改变 `LlmBridge` 一次 Model Call、`ToolExecutor` 一次 ToolCall 的既有边界；
- 在 v1 契约中包含缓存断点（外部缓存事实复核与 Provider 落法为后续扩展点）。

## 2. 术语与所有权

| 术语 | 规范含义 | 所有者 |
|---|---|---|
| 轮（Turn） | 一轮完整对话（用户输入 → 最终输出，含中间 ToolCall 轮次）；写入/积累原子单位 | 范式实现 |
| 小节（Section） | 一组连续同标签的轮；小节范式组织单位，含标签、视图与压缩状态 | `SectionContext` |
| 分区（Zone） | 缓存区（前缀，永不压缩）/ 可压缩区（压缩候选）/ 活跃区（可修正、不压缩） | 范式实现 |
| 范式（Paradigm） | `Context` 的黑盒实现（配置 / 构造 / 装配 / 持久化全部内部自治） | 范式实现 |
| 压缩记录 | 一次压缩的产物（目标、摘要、快照、原文引用、版本） | 范式实现 |
| Store 契约 | 范式定义的持久化接口与条目类型（如小节范式 `SectionStore`） | 范式 + 其下游 |
| 视图（View） | 小节/压缩目标的当前内容形态：Raw（原文）/ Compressed（摘要） | 范式实现 |

"用户"在本文中指使用 Armillae 构建产品的开发者，不指其产品中的最终用户。

## 3. crate 与依赖边界

### 3.1 `armillae-context`

该 crate 必须拥有：

- `Context` trait（object-safe 薄契约）与压缩管道三态语义；
- 公共协议类型（压缩目标、错误；对话边界全部为 core `Message` / `Vec<Message>`）；
- 当前内置范式实现 `SectionContext` 及其 Config/Builder、`SectionStore` 契约、范式自身
  API（特有操作 / 持久化 / 恢复 / 查询）；`TraditionalContext` 为规划中（后续范围）。

该 crate 不得依赖：

- `armillae-llm`、`armillae-tools` 或 Agent SDK；
- 数据库 Client、Redis Client 或具体持久化实现；
- Tokio 等特定异步运行时类型（公共接口不暴露运行时类型）；
- `rig-core` 或任何 Provider SDK。

本文展示的公共类型、Trait、常量和错误必须从 `armillae_context` crate root 可用。

### 3.2 依赖图

```text
armillae-context ────depends on──► armillae-core（唯一依赖）

应用 / 可选 runtime ────depends on──► context implementation
应用 / 可选 runtime ────optionally──► armillae-llm / armillae-tools
```

约束：

- `armillae-context` 只依赖 `armillae-core`；与 `armillae-llm` / `armillae-tools` 互不依赖
  （经 `ToolContext` 注入与协议耦合）；
- 调用方在 Bridge 前后充当中介（导出 → 推理 → 写回）；
- 小节范式的 `record_section` tool definition 由该范式提供，注册与执行由下游完成；
- 现有 `armillae-core`、`armillae-llm`、`armillae-tools` 的公共 API 不因本 Spec 改变。

## 4. 公共协议约定

### 4.1 版本、Serde 与 Schema

公共数据协议版本固定为 `armillae.context/v1alpha1`。公共类型默认派生
`Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema`；预期扩展的枚举标记
`#[non_exhaustive]`；所有公共协议根类型生成 JSON Schema 并提交稳定快照。

### 4.2 命名空间约定

范式标识为命名空间化字符串（如 `armillae-context/section`）；自定义标签为命名空间化
字符串（如 `custom.xxx`）。

## 5. 公共协议类型

公共协议类型 = 跨范式统一、出现在 `Context` trait 签名中的类型；范式自身的类型（配置、
状态、Store 条目、特有操作）由范式定义，不属于本层。对话边界（写入 / 写回 / 导出）全部
为 armillae-core 标准类型（`Message` / `Vec<Message>` / `TokenUsage`），不在此列。

### 5.1 压缩目标

压缩流程的评估与准备环节统一以"压缩目标"表达。"怎么压"的指令参数为范式内部概念，由范式
在准备环节内部按目标与自身配置生成，不进入公共协议。

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum CompressionTarget {
    Section { id: u64 },           // 小节范式（当前唯一实现）
    // 传统范式（规划中）所需形态在实现时作为新变体加入
}
```

### 5.2 压缩管道状态（错误分类用）

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompressionState {
    Idle,          // 空闲
    Evaluated,     // 已评估（冻结）
    Prepared,      // 已准备（冻结）
}
```

`CompressionState` 仅用于 `ContextError::InvalidState` 的错误表达与压缩管道语义契约；
`Context` trait 不提供状态查询方法（状态机状态由范式内部维护，可观测由范式 API 提供）。

## 6. Context trait（薄契约）

### 6.1 完整签名

```rust
pub trait Context: Send + Sync {
    // —— 对话：core 消息进出 ——
    fn push_user_input(&mut self, message: Message) -> Result<(), ContextError>;
    fn apply_model_output(&mut self, message: Message, usage: TokenUsage)
        -> Result<(), ContextError>;   // usage 必填
    fn export(&self) -> Result<Vec<Message>, ContextError>;

    // —— 压缩管道：生产压缩推理上下文 ——
    /// 评估：范式内部自检触发条件，产出压缩目标（None = 本轮不压缩）
    fn evaluate_compression(&mut self) -> Result<Option<CompressionTarget>, ContextError>;
    /// 准备：按目标生成待导出压缩上下文；必须先评估（空闲调用 → InvalidState）；
    /// 只读生成；范式内部先落盘原文
    fn prepare_compression(&mut self, target: CompressionTarget)
        -> Result<Vec<Message>, ContextError>;
    fn apply_compression_result(&mut self, summary: Vec<Message>) -> Result<(), ContextError>;
    fn abandon_compression(&mut self) -> Result<(), ContextError>;
}
```

- 持久化动作（原文落盘 / 压缩快照 / 状态保存）由范式内部执行（范式持有自己的 Store 契约，
  见 §7.1.7）——`Context` trait 不暴露持久化参数（如原文引用）；
- `Context` trait 不含持久化 / 恢复 / 查询 / 状态查询方法——这些由范式自身 API 提供。

### 6.2 压缩管道三态语义

```
空闲 ──evaluate→ Some(目标) ──► 已评估 ──prepare(目标)──► 已准备 ──apply_compression_result──► 空闲
  ▲                                 │                        │
  └────────────────── abandon ◄─────┴────────────────────────┘
```

| 当前状态 | 允许的操作 | 被拒的操作 |
|---|---|---|
| 空闲 | push_user_input、apply_model_output、export、evaluate、abandon（忽略） | prepare（必须先评估） |
| 已评估 | export、prepare（→ 已准备）、abandon（→ 空闲） | 写回 / 手动操作 |
| 已准备 | export、apply_compression_result（→ 空闲）、abandon（→ 空闲） | 写回 / 手动操作 |

冻结语义：已评估 / 已准备期间写回必须被拒 → 评估结果与上下文必然一致 → 评估不过期。
状态机状态由范式内部维护；本表是各范式必须遵守的压缩管道语义契约。

### 6.3 方法语义契约（范式必须遵守）

| 方法 | 前置 | 后置 | 错误 |
|---|---|---|---|
| push_user_input | 空闲 | 内容进入当前轮/小节 | InvalidState |
| apply_model_output | 空闲 | 轮次落定、token 事实更新、窗口滑动 | InvalidState |
| export | 任意 | 纯函数，无副作用 | InvalidRequest（剥离后空序列） |
| evaluate_compression | 空闲 | Some → 已评估；None 留空闲 | InvalidState |
| prepare_compression | 已评估（**必须先评估**，空闲调用直接报错） | → 已准备；产出 `Vec<Message>`（**范式内部先落盘原文**）；**不修改上下文结构（只读生成）** | InvalidState（空闲调用）/ InvalidOperation（目标与评估结果不符） |
| apply_compression_result | 已准备 | → 空闲；视图替换；范式内部保存压缩快照与状态 | InvalidState |
| abandon_compression | 任意（空闲调用为 no-op 成功；已评估/已准备时清理已存档条目） | → 空闲；**无需恢复快照（prepare 未改结构）**；范式内部清理已存档条目（经其 Store 契约） | — |

## 7. 范式实现

### 7.1 小节范式（SectionContext）

#### 7.1.0 压缩指令组装（范式内部）

"怎么压"的指令参数（结构固定为保持小节结构、压缩方式按目标小节标签查映射表、工具轮次
策略取配置、目标 token 数取配置）**由范式在准备环节内部生成并翻译为压缩指令消息**，不
经过公共协议外传；下游拿到 prepare 产出的 `Vec<Message>`（指令消息 + 目标内容）直接推理，
零组装。**目标内容同样剥离 `record_section` 簿记痕迹（与 §8.1 同规则）**——否则未配对的
tool_calls 会被严格 Provider 拒绝，产物无法"直接推理"（Live 验证发现）。

工具轮次策略（`ToolTurnPolicy`，取配置）作用于含工具轮次（含 ToolCall / ToolResult 的轮）的
压缩候选小节：

- `Downgrade`：允许压缩，但压缩指令要求把工具轮次降级为自然语言摘要（不保留结构化
  ToolCall / ToolResult 痕迹），摘要并入所属小节；
- `Reject`：含工具轮次的小节从压缩候选排除（不压缩，保持原文）——**评估阶段即排除**
  （evaluate 不产出此类目标）；prepare 侧防御性拒绝。

#### 7.1.1 模型与内存结构

三层级（`Message` ⊂ 轮 ⊂ 小节）。小节是范式内部结构，包含当前视图（Raw 原文 /
Compressed 摘要）、压缩快照、原文引用、版本、标签。压缩提交后内存释放原文；视图文本在
内存，`export` 是纯函数。结构坍缩（多条消息 → 单条摘要）是压缩的预期语义。

#### 7.1.2 三窗口分区与滑动

```
位置维度（按小节创建顺序）：
[缓存区（前缀，永不压缩/重排）] [固化区（Sealed = 可压缩区）] [活跃区（Open）]
       最老 ←———————————————————————————————————————————→ 最新
状态维度（与位置正交）：每小节 view = Raw | Compressed
```

滑动规则（每轮写回后）：新对话进入当前小节；活跃区小节数超限 → 左端溢出小节固化；固化区
不再参与模型后视修正；缓存区 = 前 `cache_prefix_sections` 个小节（确定后永不变化）。

分区语义（可压缩性列指**自动压缩**）：

| 分区 | 状态约束 | 可压缩？（自动） | 可后视修正？ | 理由 |
|---|---|---|---|---|
| 缓存区（前 N 小节，创建时确定） | 恒 Raw | 永不 | 否 | 前缀缓存保护（字节级稳定） |
| 固化区（Sealed） | Raw（候选）/ Compressed（完成） | 是（仅此区） | 否 | 已定稿，可安全压缩 |
| 活跃区（Open） | 恒 Raw | 永不 | 是 | 模型修正保护（保留原始轮次结构） |

特殊值：`Sections(n)` 常规；`All` = 整个动态区为活跃区 → 禁用自动压缩；`Hyper` = 活跃窗口
0 + 每次"非小节追加"事件立即压缩刚结束小节（无条件，区别于 `auto_compression =
SectionSwitch` 时的条件评估）。

缓存区细节：内容必须字节级不变；手动重标/重组涉及缓存区 → `InvalidOperation`。压缩候选
仅限固化区 ∩ Raw ∩ 可压缩标签；活跃区永不压缩。

#### 7.1.3 record_section tool

```json
{
  "name": "context.record_section",
  "description": "每次回答结束后调用：划定最新一小节的起始边界，并可选标注其标签",
  "parameters": {
    "type": "object",
    "properties": {
      "section_start_rounds": { "type": "integer", "minimum": 1,
        "description": "最新小节从最近第几轮完整对话开始（1=仅最新一轮自成小节）" },
      "label": { "enum": ["plan","constraint","preference","decision","fact","task","tool_execution","dialog","uncategorized"] }
    },
    "required": ["section_start_rounds"]
  }
}
```

`label` 可选（模型不标/标错不阻塞，程序兜底）；标签候选集 = 标准集 + 已注册扩展，构建时
合并生成 tool schema，构建后不可变；definition 由小节范式提供，注册与执行由下游完成。

#### 7.1.4 划分算法（apply_model_output 处理 record_section 时）

```
输入 rounds（模型输出），T = 当前轮数：
① clamp：非正整数 → 1；> T → T
② 边界：最新小节覆盖轮次 [T - rounds + 1, T]
③ 定位：边界恰好铺满最新小节 → 幂等（仅应用标签）；最新小节的真子集 → 裁出新小节；
   跨多个 Open 小节 → 合并为新小节（新自增 ID）；触及 Sealed/缓存区 → 只合并 Open 部分
④ 标签：模型给 label → 用；未给 → 沿用当前小节；无 → Uncategorized
⑤ 模型未调用 → 并入当前小节
```

#### 7.1.5 标签与映射表

标准集：Plan/Constraint/Preference（永不压缩）；Decision（可压 / 高优先级 / Shallow）；
Fact/Task/ToolExecution/Dialog（可压 / Deep）；Uncategorized（兜底 / 默认策略）。映射表 =
`SectionLabel → LabelPolicy`，程序侧配置、不进对话上下文、构建后不可变；`compressible =
false` 的小节从任何压缩候选排除（硬约束）。

小节范式类型（范式公开配置）：

```rust
pub enum SectionLabel { Standard(StandardLabel), Custom(String) }
pub enum StandardLabel { Plan, Constraint, Preference, Decision, Fact, Task, ToolExecution, Dialog, Uncategorized }
pub struct LabelPolicy { pub compressible: bool, pub priority: u8, pub method: CompressionMethod }
pub enum CompressionMethod { Shallow, Deep }
pub enum ToolTurnPolicy { Downgrade, Reject }
pub enum ActiveWindow { Sections { count: usize }, All, Hyper }
pub enum AutoCompression { TokenThreshold { threshold: u64 }, SectionSwitch }
```

小节范式公开配置（`SectionConfig`，构建后不可变）：

```rust
pub struct SectionConfig {
    pub cache_prefix_sections: usize,              // 缓存区小节数（创建时确定，永不变化）
    pub active_window: ActiveWindow,               // 活跃区窗口模式
    pub auto_compression: Option<AutoCompression>, // None = 仅手动：关闭范式自动压缩，压缩与持久化由下游完全自持
    pub tool_turn_policy: ToolTurnPolicy,          // 含工具轮次小节的压缩策略（§7.1.0）
    pub compressed_message_role: Role,             // 压缩摘要 role（armillae-core Role，默认 User）
    pub compression_token_target: Option<u64>,     // 压缩指令目标 token 数（None = 范式默认）
    pub label_policies: BTreeMap<StandardLabel, LabelPolicy>, // 标准标签映射表（默认标准策略，可覆盖）
}
```

`ActiveWindow` 序列化：`Sections(n)` → `{"type": "sections", "count": n}`；`All` →
`{"type": "all"}`；`Hyper` → `{"type": "hyper"}`（与 §4.1 的 `tag = "type"` 风格一致）。

#### 7.1.6 范式自身 API（特有操作 / 构造 / 持久化 / 恢复 / 查询）

小节范式的特有操作、持久化、恢复与查询均为**范式自身 API**，不属于 `Context` trait。
**本阶段使用方式：下游自己构造、自己驱动——持有具体类型 `SectionContext` 直接调用**。
范式构造时注入其 Store 契约实现（`Arc<dyn SectionStore>`），持久化动作由范式内部执行；
下游只实现 `SectionStore` 并注入。

```rust
// 范式自身 API（示例；小节范式实现）
impl SectionContext {
    pub fn builder(config: SectionConfig, store: Arc<dyn SectionStore>) -> SectionContextBuilder;

    // 特有操作（全部仅空闲；merge/split 另限动态区）
    pub fn relabel(&mut self, section_id: u64, label: SectionLabel) -> Result<(), ContextError>;
    pub fn merge_sections(&mut self, ids: Vec<u64>, new_label: Option<SectionLabel>) -> Result<(), ContextError>;
    pub fn split_section(&mut self, id: u64, boundary_turn: u64) -> Result<(), ContextError>;
    pub fn recompress(&mut self, section_id: u64) -> Result<(), ContextError>;  // 零 LLM，用压缩快照
    pub fn section_mapping(&self, section_id: u64) -> Option<MappingRecord>;  // 按值返回

    // 恢复（跨会话）：范式经 SectionStore 加载并装配
    pub fn restore_session(&mut self, session_id: &str) -> Result<(), ContextError>;

    // 查询（可观测）
    pub fn compression_state(&self) -> CompressionState;
    pub fn section_mappings(&self) -> Vec<MappingRecord>;
}
```

#### 7.1.7 小节范式持久化（SectionStore 契约）

小节范式定义自己的 Store 契约与条目类型，由该范式的下游实现：

```rust
/// 窗口状态（三窗口分区，随 Store 条目持久化）
pub struct WindowState {
    pub mode: ActiveWindow,           // Sections(n) / All / Hyper
    pub cache_prefix_sections: usize, // 缓存区小节数（创建时确定，永不变化）
    pub sealed_count: usize,          // 固化区（可压缩候选）小节数
    pub active_count: usize,          // 活跃区小节数
}

/// token 计数事实：以最近一轮 usage.input_tokens 为准（官方计数 ≈ 上下文规模）
pub struct TokenFacts {
    pub input_tokens: u64,
}

/// 小节范式状态（含小节结构、窗口、压缩管道状态、token 事实）
pub struct SectionState {
    pub schema_version: u32,
    pub session_id: String,
    pub sections: Vec<Section>,
    pub window: WindowState,
    pub machine: CompressionState,
    pub token_facts: TokenFacts,
}

/// 压缩条目（小节范式）；summary 视图由 compressed_text 原生承载，持久化
/// Store 自行序列化，内存路径不经过 JSON
pub struct SectionCompressedEntry {
    pub session_id: String,
    pub record_id: String,
    pub compressed_text: Vec<Message>,
    pub original_ref: OriginalRef,
    pub version: u64,
    pub archived_at: SystemTime,
}

/// 原文条目（小节范式）
pub struct SectionOriginalEntry {
    pub session_id: String,
    pub target: CompressionTarget,
    pub messages: Vec<Message>,
    pub version: u64,
    pub archived_at: SystemTime,
}

pub struct OriginalRef(String);          // 不透明引用，非空约束（小节范式）

/// prepare 阶段范式内部生成的原文快照（范式内部经 Store 落盘，不返回下游）
pub struct OriginalSnapshot {
    pub section_id: u64,
    pub messages: Vec<Message>,
    pub version: u64,
}

/// 小节范式 Store 契约（范式定义，下游实现）
pub trait SectionStore: Send + Sync {
    fn save_state(&self, state: &SectionState) -> Result<(), StoreError>;
    fn load_state(&self, session_id: &str) -> Result<Option<SectionState>, StoreError>;
    fn delete_state(&self, session_id: &str) -> Result<(), StoreError>;
    fn save_compressed(&self, entry: &SectionCompressedEntry) -> Result<CompressedRef, StoreError>;
    fn load_compressed(&self, session_id: &str, reference: &CompressedRef)
        -> Result<Option<SectionCompressedEntry>, StoreError>;
    fn delete_compressed(&self, session_id: &str, reference: &CompressedRef) -> Result<(), StoreError>;
    fn save_original(&self, entry: &SectionOriginalEntry) -> Result<OriginalRef, StoreError>;
    fn load_original(&self, session_id: &str, reference: &OriginalRef)
        -> Result<Option<SectionOriginalEntry>, StoreError>;
    fn delete_original(&self, session_id: &str, reference: &OriginalRef) -> Result<(), StoreError>;
}
```

`StoreError` 由小节范式定义（结构化错误，供其 Store 实现返回）；`CompressedRef` 为小节
范式的不透明引用类型（同 `OriginalRef` 风格）。

```rust
/// Store 契约错误（小节范式；结构化，供其 Store 实现返回）
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StoreError {
    #[error("persistence backend error: {message}")]
    Backend { message: String },
    #[error("invalid store entry: {message}")]
    InvalidEntry { message: String },
}
```

**编排义务（范式内部执行；下游只实现 `SectionStore` 并注入）**：

| 时机 | 义务 |
|---|---|
| 压缩准备（prepare 内） | `save_original`（**原文先落盘**）→ 记 ref |
| 压缩提交（apply 内） | 视图替换 + `save_compressed`（快照）+ `save_state` |
| 写回后 / 压缩提交后 / 解压后 / 重组后 | `save_state` |
| 跨会话恢复 | `load_state → restore_session`；需压缩视图 `load_compressed`、需原文 `load_original` |

**会话清理与恢复性维护由下游直接经 `SectionStore` 执行，不进范式内部**：会话清理需覆盖
三组条目（`delete_state` + 相关 `delete_compressed` / `delete_original`）；解压 / 重组等
恢复性操作为下游显式调用的范式自身 API（非范式自动义务）。

实现自由：介质（DB / Redis / 内存 / 文件）、序列化、懒加载、缓存替换、跨条目原子性均为
小节范式与其下游的自由；压缩快照可容忍丢失（缺失时降级为原文视图，权威数据在持久存储）。

### 7.2 传统范式（TraditionalContext，规划中 / 后续范围）

**首个实现范围不包含本范式**；设计保留，未来实现时自行定义模型、配置与持久化契约。
两层级模型（`Message` ⊂ 轮）+ 按轮分区；无标签、无 record_section；压缩目标形态与结果
形态待实现时定义；自动压缩模式与结果形态为传统范式内部概念（实现时定义自己的类型）；
持久化接口由传统范式自行定义。

### 7.3 未来范式接入契约

1. 实现 `Context` trait 通用方法（遵守 §6.3 语义契约）；
2. 定义自己的 Config/Builder 与范式自身 API（特有操作 / 持久化 / 恢复 / 查询）；
3. 范式标识为命名空间化字符串（`armillae-context/<name>`）；
4. 持久化由范式自行定义（Store 契约与条目类型），Context 不涉及。

## 8. 导出（详细）

### 8.1 组装规则

```
export() 遍历（缓存区 → 可压缩区 → 活跃区）：
  缓存区小节/轮   → 输出 messages（原文）
  可压缩区 Raw    → 输出 messages（原文）
  可压缩区 Compressed → 输出摘要消息（summary，role = compressed_message_role，默认 user）
  活跃区小节/轮   → 输出 messages（原文）
剥离规则：
  - record_section 的 ToolCall（Assistant 消息中的 ContentPart）与对应 ToolResult
    （Role::Tool 消息）从输出中移除
  - 剥离后消息为空 → 整条消息移除
  - 剥离后消息序列为空 → 返回错误（理论上不会：至少存在用户消息）
```

### 8.2 缓存断点（v1 不含，后续扩展）

内容块级缓存断点（`CacheBreakpoint { message_index, part_index }`）不进 v1 契约；外部
缓存事实复核（TTL / 断点上限 / usage 口径）与 Provider 落法（OpenAI 自动前缀缓存忽略；
Anthropic 未来在对应 ContentPart 上落 `cache_control`，当前 Adapter 未实现）为后续
扩展点。范式只需保证缓存区内容字节级稳定（见 §7.1.2）。

### 8.3 convert.rs 契约（export 输出必须满足）

System 仅文本 / User 无 ToolCall / Assistant 无 ToolResult / Tool 仅 ToolResult / 消息
content 非空 / 不含 `ProviderData`（导出侧校验拒绝）。注：convert.rs 对 `ProviderData`
按投影规则处理（同 Provider 已知 kind 校验回放、外部/未知记录 `not_forwarded` 不注入
wire request，见 llm-bridge Spec），export 输出校验比请求转换更严格。

## 9. token 计数

`apply_model_output` 契约强制 usage 参数必填（由签名保证）；`input_tokens` 缺失（`None`，
真实 Provider 可能不报 usage）时**保留上一轮 token 事实、不报错**；token 事实 = 最近一轮
`usage.input_tokens`（有值才更新，官方计数 ≈ 当前上下文规模）；压缩提交后下一轮 usage
自动校准；`TokenThreshold` 自动压缩模式评估：`token_facts.input_tokens >= threshold`；
无需注入 tokenizer。

## 10. 并发模型

范式实例的使用方式由范式与其下游约定。当前小节范式为会话级状态容器：绑定一个会话，由
驱动方串行使用（`Send + Sync`，与 trait 约束一致）；多会话 = 多实例，天然并行；压缩推理期间
（已准备冻结）新消息由下游协调；存储并发由下游 `SectionStore` 实现保证（按 session_id
隔离或内部锁）。未来范式可自行选择内部实现（如无状态 + Store 持有结构），Context 不规定。

## 11. 错误处理

```rust
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ContextError {
    #[error("invalid context configuration: {message}")]
    InvalidConfiguration { message: String },
    #[error("invalid state for {operation}: expected {expected:?}, actual {actual:?}")]
    InvalidState { operation: &'static str, expected: CompressionState, actual: CompressionState },
    #[error("invalid request: {message}")]
    InvalidRequest { message: String },
    #[error("invalid operation: {message}")]
    InvalidOperation { message: String },
    #[error("store failure (retryable: {retryable}): {message}")]
    Store { retryable: bool, message: String },
}
```

对齐既有 crate 错误惯例（`armillae-llm` / `armillae-tools` / `armillae-simulate`）：错误枚举派生
`PartialEq + Eq`（可 assert_eq）、`#[non_exhaustive]`、结构化具名字段；底层失败在跨范式错误里
**归一化**（对齐 `simulate::SimulationError::BackendFailure` 先例）——`StoreError::Backend`
归一化为 `retryable = true`，`StoreError::InvalidEntry` 归一化为 `retryable = false`，
`ContextError` 不携带 `StoreError` 类型本体。`StoreError` 由小节范式定义（见 §7.1.7），
经 `#[non_exhaustive]` 允许在冻结列表基础上扩展，下游按需匹配。

## 12. 测试策略

- 压缩管道三态语义：转移表逐条（合法推进 / 非法操作 → InvalidState，含空闲调用
  prepare → InvalidState）；
- 小节划分：record_section clamp / 合并 / Sealed 不可动 / 兜底；
- 窗口滑动：Sections(n) / All / Hyper；缓存区保护（永不压缩、重排拒绝）；
- 导出：剥离规则（痕迹移除、空消息移除）、压缩块 role、convert.rs 契约合规；
- 压缩管道：Mock 范式（确定性评估）→ prepare 产物（指令消息 + 目标内容）→ apply 视图
  替换 + ref 校验 + version 递增；
- 持久化：Mock SectionStore → 三组条目 round-trip、恢复按需加载、快照缺失降级、版本校验；
- 范式切换：同一下游代码对 SectionContext 与一个测试用 Mock 范式行为一致（验证薄契约，
  不依赖第二个内置范式）。

## 13. 实施门禁

1. 缓存断点不进 v1 契约（外部缓存事实复核与 Provider 落法为后续扩展点；当前 Anthropic
   Adapter 未实现 cache_control）；
2. `TraditionalContext`（传统范式）不在首个实现范围，作为后续第二个内置范式实现；
3. 创建 `armillae-context` crate 前需用户授权并建立实施清单（`.agents/todos/`）。

## 14. 验收标准

1. `Context` trait 方法语义契约（§6.3）全部通过合约测试；
2. `SectionContext` 满足 §12 测试策略；`TraditionalContext` 为后续范围（未来走 §7.3
   接入契约）；
3. `export()` 输出满足 convert.rs 契约且剥离规则正确；
4. 压缩管道三态语义逐条通过；评估冻结保证时效；
5. `SectionStore` 契约通过 Mock 实现验证编排义务（准备时原文先落盘、提交时快照与状态保存）；
6. 压缩管道产物可直接推理（下游零组装）；
7. 恢复流程（范式自身 API）不丢数据；压缩快照缺失可降级原文视图。

## 15. 调研参考

- [OpenAI Prompt caching](https://platform.openai.com/docs/guides/prompt-caching)（缓存断点
  后续扩展参考）；
- [Anthropic Prompt caching](https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching)
  （缓存断点后续扩展参考）；
- [OpenAI Chat Completions API](https://platform.openai.com/docs/api-reference/chat/create)；
- [Anthropic Messages API](https://docs.anthropic.com/en/api/messages)；
- [Armillae LLM Bridge Spec](llm-bridge.md)：`CompletionRequest.messages`、convert.rs 转换
  契约、`TokenUsage`。
