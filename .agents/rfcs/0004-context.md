# RFC 0004：Armillae 上下文组织与压缩（armillae-context）

> 状态：Accepted
> 接受日期：2026-08-26
> 修订：2026-08-28（未合并前设计期修正，全部回填：prepare 产物剥离 record_section 痕迹
> （Live 验证驱动，§4.3/§9）；usage 缺失容忍（§4.3）；Reject 评估阶段排除（§4.3）；
> 重组拒绝 Compressed 小节（§6.3）；会话清理移交下游（§4.7）；auto None 语义澄清（§4.10）；
> 缓存区创建即定与写入目标动态区（§4.4））
> 设计入口：[Armillae 设计索引](../DESIGN.md)
> 上位 RFC：[RFC 0001：Agentic 叙事运行时](0001-agentic-runtime.md)
> 落地规范：[Armillae 上下文组织与压缩 Spec](../specs/context.md)

本 RFC 记录 `armillae-context` 的架构决定：以**薄 `Context` trait** 提供跨范式的统一契约
（只专注生产可推理的上下文），让每种压缩范式（小节范式、传统范式、未来范式）作为该 trait
的**黑盒实现**，配置、构造、装配、持久化全部由范式内部自治。它不设计 Agent Harness、
Turn 流程、压缩任务的 LLM 执行或持久化实现。

RFC 接受只表示本文件中的架构边界已经确认。公共行为、接口签名、算法、失败语义和合约测试
以 [armillae-context Active Spec](../specs/context.md) 为准；创建 crate 前仍需
用户明确授权并建立实施清单。

## 1. 背景

长对话受模型上下文窗口限制；超窗内容必须被组织、裁剪或压缩。压缩决策影响 token 成本与
Provider 缓存经济性。不同应用需要不同的上下文组织方式：叙事应用需要按主题小节组织并保留
标签语义，工具密集应用可能只需要按轮压缩旧内容，未来还可能引入新的压缩范式。

范式之间的差异（组织模型、压缩逻辑、持久化需求）大于共性。统一内核会迫使所有范式共用
同一套模型；把持久化、查询等收进 Context trait 又会污染跨范式契约。因此采用"**薄
Context 契约 + 范式黑盒实现**"：Context 只定义"生产可推理的上下文"的统一接口，范式全权
负责内部一切。

## 2. 目标

1. 提供薄的 `Context` trait 契约，下游切换范式不改变调用方式。
2. 每种压缩范式是 `Context` 的**黑盒实现**（配置 / 构造 / 装配 / 持久化全部内部自治）。
3. 压缩执行管道（评估 → 准备 → 下游推理 → 提交）为统一接口方法，执行外包下游、压缩指令
   由范式组装（下游零组装推理）。
4. 导出适合推理的上下文：稳定前缀在前、易变内容在后（core `Vec<Message>`，v1 不含
   缓存断点）。
5. 持久化由范式自行定义接口与条目（如小节范式的 `SectionStore`），Context 不涉及。
6. 压缩管道遵循三态语义（评估后冻结；准备必须先评估），保证评估时效与操作序列合法。

## 3. 非目标

- 设计 Agent、Turn Runner、自动 Tool Loop 或 Memory；
- 在 `armillae-context` 内执行压缩 LLM 推理（由下游显式驱动）；
- 定义跨范式的持久化契约（持久化归各范式自行定义）；
- 决定并发与调度策略（范式实例的使用方式由范式与其下游约定）；
- 改变 `LlmBridge` 一次 Model Call、`ToolExecutor` 一次 ToolCall 的既有边界；
- 在 v1 契约中包含缓存断点（外部缓存事实复核与 Provider 落法为后续扩展点）。

## 4. 已接受决策

### 4.1 薄 Context 契约 + 范式黑盒实现

`Context` trait 是**薄接口契约**（object-safe、全同步、`Arc<dyn Context>` 持有），只专注
**生产可推理的上下文**：对话（写入 / 写回 / 导出，边界全部为 core `Message` / `Vec<Message>`
/ `TokenUsage`）与压缩管道（评估 / 准备 / 提交 / 放弃）。
**持久化、恢复、查询、配置、构造、装配全部由范式黑盒实现**，不进入 `Context` trait；范式
特有操作经范式自身 API（如小节范式的 `SectionOps` 与 `SectionStore`）。

公共协议类型（压缩目标、错误）跨范式统一；"怎么压"的指令参数为范式内部概念，不进入
公共协议。

### 4.2 范式 = Context 的黑盒实现

新范式 = 新 struct + `impl Context` + 自己的 Config/Builder + 自己的持久化接口。内置范式
**当前仅 `SectionContext`（小节范式）**；`TraditionalContext`（传统范式）为规划中的第二个
内置范式（后续实现，不改变架构与公共接口）。三层级模型（`Message` ⊂ 一轮完整对话 ⊂ 小节）
仅是**小节范式**的组织模型，不是通用模型；传统范式为两层级（`Message` ⊂ 轮）+ 分区。

### 4.3 压缩管道（统一接口，执行外包）

```
evaluate_compression()   → Option<压缩目标>（范式内部自检触发，冻结）
prepare_compression(target)（必须先评估，空闲调用直接报错）
                         → Vec<Message>（可直接推理；范式内部先落盘原文）
下游推理（LlmBridge）     → 压缩摘要（Vec<Message>）
apply_compression_result(summary) → 视图替换（范式内部保存压缩快照与状态）
```

- 压缩指令（结构 / 深度 / 工具轮次策略 / token 目标）由范式组装进压缩上下文，下游零组装；
- `prepare_compression` 必须先评估（空闲调用直接报错）；压缩推理与对话推理对称（都产出
  `Vec<Message>`）；
- `prepare_compression` 产物必须"可直接推理"：目标内容剥离 `record_section` 簿记痕迹
  （与 §4.6 导出同规则）——否则未配对的 tool_calls 会被严格 Provider 拒绝（Live 验证
  确认，2026-08-28）；
- `apply_model_output` 的 usage 参数由签名必填；`input_tokens` 缺失（`None`，真实 Provider
  可能不报 usage）时保留上一轮 token 事实、不报错；
- 工具轮次策略 `Reject` 在**评估阶段即排除**含工具轮次的小节（evaluate 不产出此类目标），
  prepare 侧防御性拒绝。

### 4.4 窗口模型与分区（小节范式）

分区语义：**缓存区**（前缀，永不压缩 / 重排，缓存保护）、**可压缩区**（压缩候选）、
**活跃区**（可后视修正、不压缩）。小节范式内部的可压缩区即固化区（Sealed 小节集合）；
滑动粒度随范式（小节范式按小节，传统范式规划中按轮）。

缓存区成员按创建顺序取前 `cache_prefix_sections` 个小节、**创建即定、永不变化**；
**写入目标必须是动态区小节**——最新小节若属缓存区则新建小节接收新内容，否则内容会永远
堆积在缓存小节、无法划分与压缩（实现期确认，2026-08-28）。

核心规则：*固化才可压缩、活跃才可修正*——一对互斥规则，消解"压缩中又被重划"的并发问题；
压缩候选仅限固化区（活跃区永不压缩）。

### 4.5 压缩管道三态语义（状态机）

压缩管道遵循三态语义（空闲 / 已评估 / 已准备）：评估后冻结（写回被拒，保证评估结果与
上下文一致、评估不过期），准备必须先评估（空闲调用直接报错），准备后等待提交或放弃。
**状态机状态由范式内部维护**，`Context` trait 不暴露状态查询接口（可观测由范式自身 API
提供）。

### 4.6 导出

`export()` 输出 armillae-core `Vec<Message>`，遵守 llm-bridge Spec 转换契约；布局为
缓存区 → 可压缩区（Raw 原文 / Compressed 摘要块）→ 活跃区；剥离 `record_section` 痕迹
（小节范式）。内容块级缓存断点不进 v1 契约：外部缓存事实复核（TTL / 断点上限 / usage
口径）与 Provider 落法（Anthropic ContentPart 级 `cache_control`）为后续扩展点。

### 4.7 持久化归范式（范式自有 Store 契约）

持久化由**范式黑盒实现**：每个范式定义自己的 Store 接口契约与条目类型（如小节范式的
`SectionStore`：状态 / 压缩 / 原文三组条目），由该范式的下游实现；存储介质、序列化、
懒加载、缓存策略、原子性均为范式与其下游的自由（如压缩快照可容忍丢失，缺失时降级为
原文视图）。`Context` trait 不涉及持久化。

### 4.8 token 计数内部化

`apply_model_output` 契约强制 usage 必填；token 事实 = 最近一轮 `usage.input_tokens`
（官方计数 ≈ 上下文规模）；压缩提交后下一轮 usage 自动校准。无需注入 tokenizer。

### 4.9 范式自身 API（特有操作 / 持久化 / 恢复 / 查询）

范式特有操作（小节范式：重标 / 重组 / 重压 / 小节粒度查询）、持久化接口（`SectionStore`）、
恢复与可观测查询均为**范式自身 API**，不属于 `Context` trait。本阶段使用方式：下游自己
构造、自己驱动——持有具体类型 `SectionContext` 直接调用范式 API。

### 4.10 小节范式（SectionContext）

三层级模型；`record_section` tool 划界（definition 构建时生成，tool schema 构建后不可变，
保证工具定义前缀缓存稳定）；标准标签集（Plan / Constraint / Preference 永不压缩；
Decision / Fact / Task / ToolExecution / Dialog / Uncategorized 可压缩）；标签映射表
（`SectionLabel → LabelPolicy`）不进对话上下文、构建后不可变；自动压缩模式
`auto_compression: Option<AutoCompression>`（`TokenThreshold{threshold}` / `SectionSwitch`；
None = 仅手动：关闭范式自动压缩，**压缩与持久化由下游完全自持**——下游自行实现压缩与
上下文注入，范式仅作对话/小节容器（写入与导出），其压缩管道在该模式下不生效），触发决策
为范式内部逻辑，不提供独立触发器抽象。

### 4.11 传统范式（TraditionalContext，规划中）

**暂不实现**。两层级模型 + 按轮分区；无标签、无 record_section；压缩目标形态待实现时
定义；自动压缩模式与结果形态为传统范式内部概念（实现时定义自己的类型）；持久化接口由
传统范式自行定义。

## 5. 依赖方向

```text
开发者应用 / 可选 Agentic Runtime
          │
          ├──────────────► armillae-llm / armillae-tools（可选、彼此独立）
          │
          ▼
  armillae-context ──────────► armillae-core（唯一依赖）
```

约束：

- `armillae-context` 只依赖 `armillae-core`；不依赖异步运行时、HTTP Client 或 LLM SDK；
- `armillae-context` 与 `armillae-llm`、`armillae-tools` 互不依赖（经 `ToolContext` 注入
  与协议耦合）；
- 调用方在 Bridge 前后充当中介（导出 → 推理 → 写回）；
- `record_section` tool 的 definition 由小节范式提供，注册与执行由下游完成。

## 6. 典型流程

### 6.1 对话轮次

```text
push_user_input(user_message)
  -> export() 导出可推理上下文（剥离 tool 痕迹，缓存区在前）
  -> LlmBridge.complete（tools 含 record_section，仅小节范式，由下游显式放入）
  -> 下游执行 record_section（仅小节范式）
  -> apply_model_output(assistant_message, usage) 写回（usage 必填）
  -> 窗口滑动（活跃区溢出 -> 可压缩区）
  -> 范式内部持久化（小节范式经 SectionStore，由范式编排）
```

### 6.2 压缩子流程

```text
evaluate_compression() -> None：跳过；Some(目标)：进入已评估（冻结）
  -> prepare_compression(目标)（必须先评估）：生成 Vec<Message>，进入已准备
     （范式内部先落盘原文 -> 记原文引用）
  -> 下游直接推理 messages -> 压缩摘要
  -> apply_compression_result(summary)：视图替换，回到空闲
     （范式内部保存压缩快照与状态）
```

### 6.3 手动操作与恢复（范式自身 API）

```text
解压：范式自身恢复 API（范式经 Store 取回原文并替换视图）-> 范式内部持久化
重标（小节范式，仅空闲）：relabel(section_id, label) -> 范式内部持久化
重组（小节范式，仅空闲）：merge_sections(ids, new_label) / split_section(id, boundary_turn)
  -> 范式内部持久化（涉及 Compressed 小节 → InvalidOperation，需先解压；重组后小节回
  Raw，原压缩失效，需重新压缩）
重压（小节范式，仅空闲）：recompress(section_id)（零 LLM，用压缩快照恢复压缩视图）
  -> 范式内部持久化
放弃：abandon_compression（已评估/已准备 -> 空闲；prepare 只读生成不修改结构，无需恢复
  快照；范式内部清理已存档条目，经其 Store 契约）
跨会话恢复：范式自身恢复 API（小节范式经 SectionStore 加载并装配）
```

## 7. 主要取舍

### 7.1 收益

- Context trait 最薄：跨范式契约只含"生产可推理的上下文"，范式切换无感；
- 范式完全黑盒自治：配置 / 构造 / 装配 / 持久化 / 内部实现全部自由；
- 新范式 = 实现薄 Context + 自己的 Config / Store 契约，无需修改既有代码与公共接口；
- 压缩执行外包、下游零组装，context 零 LLM 依赖（只依赖 core）；
- Store 契约贴合范式（每个范式定义自己的条目类型与接口）；
- 缓存区前置的导出天然适配 OpenAI 自动前缀缓存（Anthropic 断点落法为后续扩展）。

### 7.2 成本与风险

- 范式各自实现持久化与内部逻辑（共享辅助模块缓解，但无法完全消除）；
- 下游需要分别了解各范式的 API 与 Store 契约；
- 重标、重组等范式特有操作需持有具体类型；
- 缓存断点（v1 不含）的 Provider 落法未冻结，未来扩展不改变 v1 导出契约；
- 压缩质量依赖下游提示词注入，范式不保障事实完整性。

## 8. 被拒绝的方案

| 方案 | 结论 | 原因 |
|---|---|---|
| 统一内核 + 压缩模式插件注入 | 不采用 | 范式差异大于共性，统一内核迫使所有范式共用一套模型 |
| 三层级（小节）作为通用上下文模型 | 不采用 | 传统范式无需小节结构，只需轮 + 分区 |
| 跨范式 `ContextStore` 持久化契约 | 不采用 | 持久化归范式黑盒，各范式定义自己的 Store 契约与条目 |
| `Context` trait 含持久化 / 查询 / 状态查询接口 | 不采用 | 薄契约只专注生产可推理的上下文 |
| 泛型骨架 + 范式钩子注入（`Context<C>`） | 不采用 | 范式即 impl（黑盒），无需泛型化骨架 |
| 无状态 context（结构由 Store 持有） | 范式内部选择 | 是否无状态、是否泛型化存储是范式内部实现细节，Context 不规定 |
| 压缩文本并入状态快照（两组接口） | 不采用 | 三组条目使压缩快照可独立存放，存储后端可分离 |
| 压缩回调注入（`CompressionExecutor`） | 不采用 | 违反"显式优于隐式"，压缩推理由下游显式驱动 |
| 触发器组合 / 独立触发器抽象 | 不采用 | 触发决策为范式内部逻辑，不提供组合器或触发器类型 |
| 命名预设模式 | 不采用 | 只提供内置范式直接实例化 |
| 评估与准备合并为一个方法 | 不采用 | 拆分便于自由组合（只评估不执行、延迟准备） |
| `prepare_manual` 独立入口（手动压缩管道） | 不采用 | 压缩必须经过评估；手动压缩移出 trait 契约，由下游自持（范式不提供手动压缩入口） |
| 缓存断点进 v1 导出契约 | 不采用（延后） | 外部缓存事实与 Provider 落法未冻结；v1 导出保持纯 core `Vec<Message>` |

## 9. 验收场景

Active Spec 和后续合约测试至少覆盖：

1. 同一下游代码对 `SectionContext` 与一个测试用 Mock 范式行为一致（范式切换无感）；
2. `export()` 输出满足 convert.rs 契约且剥离规则正确（tool 痕迹移除、空消息移除）；
3. 压缩管道三态语义：评估后写回被拒（评估不过期）；准备后等待提交或放弃；
4. 窗口滑动：`Sections(n)` / `All` / `Hyper` 语义正确；缓存区永不压缩/重排；
5. 压缩管道产物（prepare 的 messages）可直接推理、下游零组装，且剥离 `record_section`
   痕迹（不出现未配对 tool_calls）；
6. 小节范式 `SectionStore` 通过 Mock 实现验证编排义务（准备时原文先落盘、提交时快照与
   状态保存）；
7. 跨会话恢复（范式自身 API）不丢数据；压缩快照缺失时降级为原文视图；
8. 小节划分（record_section）clamp / 合并 / Sealed 不可动 / 兜底正确；
9. token 事实以 usage.input_tokens 为准，压缩后自动校准。

## 10. 实施门禁与后续工作

1. 缓存断点不进 v1 契约：外部缓存事实复核（TTL / 断点上限 / usage 口径）与 Provider
   落法（Anthropic ContentPart 级 `cache_control`）为后续扩展点（当前 Anthropic
   Adapter 未实现 cache_control）；
2. `TraditionalContext`（传统范式）不在首个实现范围，作为后续第二个内置范式实现；
3. 建立实施清单（`.agents/todos/`）与 crate 计划（Cargo CLI），创建 crate 前需用户授权；
4. 按项目规范补齐合约测试、Mock、Serde round-trip 与 Schema 快照。

## 11. 影响范围

### 11.1 直接影响

- 新增 `armillae-context` crate（已创建，0.1.0-alpha.0）与 Active Spec、实施清单；
- 依赖方向：context 只依赖 core，与 llm/tools 正交；
- 设计索引已登记新子系统与本文档（`.agents/DESIGN.md` §2/§3）。

### 11.2 明确不受影响

- `LlmBridge` 仍只执行一次 Provider 无关 Model Call；
- `ToolExecutor` 仍只执行一次显式 `ToolCall -> ToolResult`；
- 不修改现有公共协议、Provider Adapter、crate、Cargo manifest 或版本；
- 不创建 Agent Harness、Turn Runner、Memory 或持久化实现。

## 12. 决策记录与依据

本 RFC 的核心边界由项目方在设计讨论中逐项确认：薄 `Context` 契约（只生产可推理的上下文）
+ 范式黑盒实现（配置 / 构造 / 装配 / 持久化全部内部）；范式即 impl，无需泛型骨架；压缩
执行外包下游、指令由范式组装；三窗口分区与滑动粒度随范式；压缩管道三态语义（状态机在
范式内部，不暴露查询）；持久化归范式（范式自有 Store 契约与条目，如小节范式 `SectionStore`）；
token 计数以 usage 内部化；范式特有操作与持久化 / 恢复 / 查询均为范式自身 API（下游持有
具体类型调用）；首个实现内置范式仅 `SectionContext`，`TraditionalContext` 规划中。

外部技术依据：

- [OpenAI Prompt caching](https://platform.openai.com/docs/guides/prompt-caching)：自动前缀
  缓存机制（前缀一致即命中、命中价约 0.1x；缓存断点后续扩展参考）；
- [Anthropic Prompt caching](https://docs.anthropic.com/en/docs/build-with-claude/prompt-caching)：
  显式 `cache_control` 断点（content block 内部字段；缓存断点后续扩展参考）；
- [OpenAI Chat Completions API](https://platform.openai.com/docs/api-reference/chat/create)：
  usage 缓存计数字段；
- [Anthropic Messages API](https://docs.anthropic.com/en/api/messages)：usage 缓存计数与
  断点口径；
- [Armillae LLM Bridge Spec](../specs/llm-bridge.md)：`CompletionRequest.messages`、
  convert.rs 转换契约、`TokenUsage`。
