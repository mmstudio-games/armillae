# Armillae 上下文组织与压缩实施清单（armillae-context）

> 状态：Active；首阶段实现完成（P1–P4）并已通过 DeepSeek 官方 Live 验证（2026-08-28）
> 最后核对：2026-08-28
> 需求来源：[Armillae 上下文组织与压缩 Spec](../specs/context.md)
> 决策来源：[RFC 0004](../rfcs/0004-context.md)

本清单只记录 Active Spec 与当前实现之间的差异，不是独立需求来源。首阶段实现（P1–P4）已
完成：薄 `Context` 契约、压缩管道三态语义、`SectionContext` 小节范式与 `SectionStore` 契约
均已落地并通过合约测试；`TraditionalContext` 与缓存断点为后续范围，不属于本清单。

## Live 验证

- [x] 提供默认 ignored 的 Live harness（`armillae-llm-rig/tests/context_live.rs`，
  环境变量驱动、无凭证入库）：对话 → record_section 划界 → 自动压缩 → prepare 零组装 →
  真实 Bridge 推理 → apply → export 全链路断言。
- [x] DeepSeek 官方 API（`deepseek-v4-flash`）Live 通过（2026-08-28）；真实压缩摘要返回
  并提交，导出上下文被真实 Provider 直接消费。
- [x] Live 验证发现并修复：prepare 产物未剥离 record_section 痕迹 → 未配对 tool_calls 被
  严格 Provider 拒绝（400）；已按 §8.1 同规则剥离（Spec §7.1.0 同步，含契约测试）。
- [ ] 其他 Provider（OpenAI / Anthropic / 官方 DeepSeek 之外的中转站）Live 场景矩阵待有
  凭证时补充；opencodego 中转站与 rig 0.41.0 的兼容性差异（缺 DeepSeek cache usage 字段、
  拒绝多段 content）已记录，不做适配器补丁绕过。

## 全量审计修正（实现 vs 文档，2026-08-28）

- [x] `ToolTurnPolicy::Reject` 在**评估阶段即排除**含工具轮次的小节（候选过滤 + Hyper 分支；
  prepare 侧防御保留；Spec §7.1.0/RFC §4.3 同步，含测试）。
- [x] `merge_sections` 涉及 Compressed 小节 → `InvalidOperation`（显式拒绝，防静默丢内容；
  Spec §7.1.6/RFC §6.3 同步，含测试）。
- [x] `apply_model_output` 的 `input_tokens` 缺失（None）容忍：保留上一轮 token 事实不报错
  （决策；Spec §6.3/§9/RFC §4.3 同步，含测试）。
- [x] 会话清理（`delete_state` + `delete_compressed`/`delete_original`）移交下游直接执行，
  不进范式内部（决策；Spec §7.1.7 义务表同步）。
- [x] `auto_compression = None` 语义澄清为"仅手动：关闭范式自动压缩，压缩与持久化由下游
  完全自持（下游自行实现压缩与上下文注入，范式仅作对话/小节容器，其压缩管道不生效）"
  （Spec §7.1.5/RFC §4.10 同步）。
- [x] 补齐 8 类缺失测试：version 递增与 ref 关联、schema_version 校验、空消息移除、
  convert.rs 各条款、缓存区重排拒绝与永不压缩、压缩后 token 校准、Reject 候选排除、
  usage 缺失容忍。
- [x] 文档 stale 修正：DESIGN.md §2 分层图、Spec 头部"未来的"、RFC §11.1"待授权/待更新"；
  Spec 片段对齐（SectionConfig 补 `label_policies`、§7.1.4③ 幂等措辞、§8.3 ProviderData
  措辞、`section_mapping` 按值返回）。

## P0：实施门禁与授权

- [x] 经用户明确授权后，使用 Cargo CLI 创建 `armillae-context` crate；确认只依赖
  `armillae-core`，不依赖异步运行时、HTTP Client、LLM SDK、`rig-core` 或持久化客户端。
- [x] 冻结公共协议版本 `armillae.context/v1alpha1`：公共类型派生
  `Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema`，预期扩展的枚举标记
  `#[non_exhaustive]`，提交 JSON Schema 稳定快照。
- [x] 确认缓存断点不进 v1 契约：外部缓存事实复核（TTL / 断点上限 / usage 口径）与 Provider
  落法（Anthropic ContentPart 级 `cache_control`）保持后续扩展点，不纳入实现范围。
- [x] 确认 `TraditionalContext`（传统范式）不进入首个实现范围，保留设计作为第二个内置范式。

## P1：公共协议与 Context trait

- [x] 实现公共协议类型：`CompressionTarget::Section { id }`、`CompressionState`、
  `ContextError`（InvalidConfiguration / InvalidState / InvalidRequest / InvalidOperation /
  Store 归一化变体：`retryable + message`，对齐 simulate `BackendFailure` 先例）；
  范式标识与自定义标签为命名空间化字符串。
- [x] 实现 object-safe 薄 `Context` trait（`Send + Sync`，`Arc<dyn Context>` 持有）：
  `push_user_input` / `apply_model_output`（usage 必填）/ `export` / `evaluate_compression` /
  `prepare_compression` / `apply_compression_result` / `abandon_compression`。
- [x] 压缩管道三态语义逐条实现：空闲调用 prepare → InvalidState；评估后写回被拒（评估不过期）；
  已准备等待提交或放弃；abandon 清理已存档条目。
- [x] trait 不含持久化 / 恢复 / 查询 / 状态查询方法；状态机状态由范式内部维护，可观测经范式
  自身 API 提供。

## P2：小节范式（SectionContext）

- [x] 实现 `SectionContext`、`SectionConfig` / Builder 与范式自身 API（relabel /
  merge_sections / split_section / recompress / section_mapping / restore_session /
  compression_state / section_mappings），特有操作全部仅空闲。
- [x] 实现三层级模型（Message ⊂ 轮 ⊂ 小节）与三窗口分区滑动：缓存区永不压缩/重排、固化区
  （Sealed）为唯一压缩候选、活跃区（Open）可后视修正；`Sections(n)` / `All` / `Hyper` 语义。
- [x] 实现标准标签集与 `LabelPolicy` 映射表（构建后不可变、不进对话上下文）：Plan /
  Constraint / Preference 永不压缩；`compressible = false` 从任何压缩候选排除（硬约束）。
- [x] 实现 `record_section` tool definition（definition 构建时生成、tool schema 构建后不可变）
  与划分算法（clamp / 幂等 / 合并 / 只合并 Open / 兜底标签）。
- [x] 实现 `auto_compression: Option<AutoCompression>`（TokenThreshold / SectionSwitch；
  None = 仅手动，压缩与持久化由下游完全自持），触发决策为范式内部逻辑，不提供独立触发器
  抽象。
- [x] 压缩指令由范式在准备环节内部组装（结构 / 深度 / 工具轮次策略 / token 目标），产出
  `Vec<Message>` 可直接推理，下游零组装。

## P3：导出、token 计数与压缩管道编排

- [x] 实现 `export()` 组装规则（缓存区 → 可压缩区 → 活跃区；Raw 原文 / Compressed 摘要，
  摘要 role 默认 user）与剥离规则（record_section 痕迹移除、空消息移除、空序列报错）。
- [x] 保证 export 输出满足 convert.rs 契约：System 仅文本 / User 无 ToolCall / Assistant 无
  ToolResult / Tool 仅 ToolResult / 消息 content 非空 / 请求中 ProviderData 一律拒绝。
- [x] 实现 token 计数内部化：`apply_model_output` usage 参数必填（签名保证）；`input_tokens`
  缺失（None）时保留上一轮 token 事实（容忍，Spec §9）；token 事实 = 最近一轮
  `usage.input_tokens`；压缩提交后下一轮 usage 自动校准。
- [x] 实现压缩管道编排义务：prepare 内先 `save_original` 落盘原文并记 ref（只读生成、不修改
  上下文结构）→ apply 内视图替换 + `save_compressed` + `save_state`。

## P4：持久化与合约测试

- [x] 冻结 `SectionStore` 契约：state / compressed / original 三组条目的 save / load / delete，
  `StoreError` 结构化错误，`CompressedRef` / `OriginalRef` 不透明引用。
- [x] 冻结 `SectionState`（schema_version / session_id / sections / window / machine /
  token_facts）与压缩 / 原文条目类型。
- [x] Mock SectionStore 验证编排义务：准备时原文先落盘、提交时快照与状态保存、跨会话恢复不丢
  数据、压缩快照缺失降级为原文视图。
- [x] 提供默认构建可用的生产级纯内存 `InMemorySectionStore`（`memory` 模块）：与小节底层数据
  模型一致（原生 SectionState / Section / Turn / Message，按不透明引用类型键控），存储路径
  零 JSON 序列化；测试观测辅助经 `testing` feature 门控。
- [x] 合约测试覆盖：三态转移表逐条（含空闲调用 prepare → InvalidState）、小节划分（clamp /
  合并 / Sealed 不可动 / 兜底）、窗口滑动与缓存区保护（永不压缩、重排拒绝）、导出剥离与
  convert.rs 合规、Mock 范式切换无感。
- [x] 公共协议 Serde round-trip、Schema 快照与顺序 / ID 关联测试；格式检查、Clippy 与测试
  全部通过。

## 集成示例

- [x] 提供离线可跑的完整链路示例（对话 → record_section 划分 → 自动压缩 → export →
  Bridge 推理）：`armillae-llm-rig/examples/context_compression.rs`（`armillae-context`
  以 llm-rig dev-dependency 接入并使用生产级内存 Store，MockBridge 离线推理，验证"下游
  零组装"）。

## 后续范围

- 缓存断点（`CacheBreakpoint`、Anthropic `cache_control` 落法）与外部缓存事实复核为后续
  扩展点，不在 v1 契约。
- `TraditionalContext`（传统范式）作为第二个内置范式，未来按 Spec §7.3 接入契约实现，需单独
  授权。
- Agent Harness、Turn Runner、Memory、RAG 不属于 `armillae-context`。
