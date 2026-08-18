# Armillae 第一阶段 TODO

> 技术事实来源为 [第一阶段技术设计](docs/DESIGN.md)。本清单只记录该设计与当前实现之间的
> 差异；需求或技术方案变化时，先更新设计，再更新本清单。

当前 Workspace 与 P0 Spike 已完成，后续实现项按设计中的依赖顺序组织：`P1` 至 `P6`
逐步完成协议、执行器、Bridge、Adapter 和 Provider。

## 总体边界

- [ ] 保持 Bridge 一次只执行一个 Model Call，不自动执行 Tool 或驱动 Tool Continuation Loop。
- [ ] 保持 Tool Executor 一次只执行一个 ToolCall，不调用 Bridge。
- [ ] 保持 `armillae-core`、`armillae-llm`、`armillae-tools` 和
      `armillae-llm-rig` 的设计依赖方向。
- [ ] 确保除 `armillae-llm-rig` 外没有 crate 依赖或暴露 rig 类型。
- [ ] 不在第一阶段引入 Turn Runner、完整 Agent、Memory、Embedding、Vector Store、RAG、
      工作流编排或世界状态。

## P0：rig 低层可行性 Spike

### 非流式能力

- [x] 锁定并记录 Spike 使用的精确 `rig-core` 版本。
- [x] 验证不使用 `rig::Agent`、`AgentBuilder` 或 `AgentRun`，只通过 `CompletionModel` 完成调用。
- [x] 验证能够发送 Tool Definition 并接收单个 ToolCall。
- [x] 验证能够接收多个 ToolCall，并保持 ID 与顺序。
- [x] 验证 Assistant ToolCall 与手工构造的 ToolResult 可以进入下一次消息历史。
- [x] 验证 OpenAI/OpenAI-compatible 与 Anthropic 的消息和 Tool 差异可由 Adapter 消化。

### Streaming 与结论

- [x] 验证文本增量可转换为 Provider 无关的语义事件。
- [x] 验证 ToolCall 名称和 JSON 参数跨任意 chunk 后能够无损重组。
- [x] 验证多个 ToolCall 的交错流式增量可以按稳定 index/call 独立重组。
- [x] 验证 drop 底层 Future/Stream 后请求会终止或尽快释放。
- [x] 记录 Spike 的 API、Provider 差异、已知限制和替代方案结论。
- [x] 确认 rig 低层 API 满足设计，当前无需评估 `genai` 或原生 Provider Adapter。
- [x] Spike 通过并完成设计复核，允许进入 Armillae 公共协议和 Bridge API 的实现与冻结。

## P1：Workspace 与 `armillae-core`

### Workspace 基础

- [x] 初始化 Rust workspace。
- [x] 创建 `armillae-core`、`armillae-llm`、`armillae-tools` 和
      `armillae-llm-rig` 四个 crate。
- [x] 落实 crate 依赖方向，禁止 `armillae-llm` 与 `armillae-tools` 互相依赖。
- [x] 保证 `armillae-core` 不依赖异步运行时、HTTP Client 或 LLM SDK。
- [x] 建立统一的格式检查、Clippy、单元测试和文档构建基线。
- [x] 使用 Semifold 0.3.0 初始化 Rust workspace 版本管理，并将四个 crate 配置为 `alpha`
      发布通道。

### Message 与 Tool 协议

- [x] 实现 `Message`、`Role`、`ContentPart` 和 `TextContent`。
- [x] 实现 `ToolDefinition`、`ToolCall`、`ToolResult`、`ToolResultContent` 和 `ToolChoice`。
- [x] 支持单个 Assistant Message 中交错出现文本和多个 ToolCall。
- [x] 保持 `ContentPart`、多个 ToolCall 和 ToolResult 的原始顺序与调用 ID。
- [x] 为预期扩展的公共枚举使用适当的 `#[non_exhaustive]` 兼容策略。

### Completion 协议

- [x] 实现 `CompletionRequest`、`OutputFormat`、`GenerationOptions` 和 `ProviderExtensions`。
- [ ] 使用命名空间隔离 Provider 扩展，并默认拒绝未知扩展。
- [x] 实现 `CompletionResponse`、`AssistantContent`、`FinishReason` 和 `TokenUsage`。
- [ ] 在 Adapter 转换中确保 `ProviderData` 不被用于绕过已有标准字段。
- [x] 支持未知 finish reason 和 ProviderData 的前向兼容。

### Streaming 协议

- [x] 实现 `CompletionEvent` 和 `ContentKind`。
- [x] 定义并验证稳定 content index、started/completed 配对和事件顺序。
- [x] 通过 `ToolCall.arguments: Value` 保证 `ToolCallCompleted` 只携带完整 JSON。
- [ ] 保证成功流只产生一个 `ResponseCompleted`。
- [ ] 保证中断流不构造虚假的完整响应。

### Core 测试

- [x] 覆盖所有公共协议类型的 Serde round-trip。
- [x] 覆盖文本与 ToolCall 混合、多 ToolCall 顺序和 ToolCall/ToolResult ID 关联。
- [x] 覆盖未知 finish reason、ProviderData 和公共枚举的前向兼容。
- [x] 生成并验证 JSON Schema 的合法性与稳定快照。

## P2：`armillae-tools`

### 类型化 Tool 与类型擦除

- [x] 实现包含 `Args`、`Output`、`Error` 和 `NAME` 的类型化 `Tool` trait。
- [x] 实现 `IntoToolOutput`，为普通 `Serialize` 输出提供 JSON blanket conversion，并让
      `ToolOutput` 保持显式多段内容。
- [x] 从 `Tool::Args` 自动生成输入 JSON Schema。
- [x] 实现 object-safe `DynTool` 和 `call_json`。
- [x] 为满足约束的类型化 Tool 提供 `DynTool` blanket implementation。
- [x] 实现规范化 `ToolOutput`，支持默认 JSON 输出和显式多段内容。

### Context、Registry 与 Executor

- [x] 实现基于类型安全 extensions type map 的轻量 `ToolContext`。
- [x] 实现 `ToolExecutor::definitions` 和单个 `ToolCall` 的 `execute`。
- [x] 实现动态注册、注销和查找的 `ToolRegistry`。
- [x] 对 Tool Definition 使用稳定排序。
- [x] 重复注册同名 Tool 时返回结构化 `ToolRegistryError`，不静默覆盖。
- [x] 解析并验证 ToolCall JSON 参数，保持输出 `call_id` 与输入 ID 一致。
- [x] 实现 `UnknownTool`、`InvalidArguments`、`ExecutionFailed` 和
      `OutputSerialization` 错误分类。
- [x] 保持宿主执行错误与模型可见的 `ToolResult { is_error: true }` 相互独立。
- [x] 不在 Executor 中引入重试、Bridge 调用、并发调度或审批策略。

### Tool 测试

- [x] 覆盖 Tool Definition 和 Schema 自动生成。
- [x] 覆盖正确执行、缺少字段、错误类型和非法 JSON 值。
- [x] 覆盖未知 Tool、Tool 自身错误和输出序列化失败。
- [x] 覆盖重复注册、稳定定义排序和注销行为。
- [x] 覆盖 `ToolContext` extensions 透传和 ToolCall ID 保持。

## P3：`armillae-llm` 与 Mock

### Bridge 接口与能力

- [x] 实现 object-safe `LlmBridge::complete` 和 `LlmBridge::stream`。
- [x] 使用标准 Future/Stream 语义，不在公共接口暴露 Tokio 类型。
- [x] 实现细分 ToolChoice 与 OutputFormat 支持的 `BridgeCapabilities`。
- [x] 在请求发送前验证 Streaming、Tool Calling、ToolChoice、Structured Output 和 Role 能力。
- [ ] 能力由 Provider/模型基线和 Adapter 验证结果决定，不提供可序列化覆盖，也不得虚构
      Provider 能力。
- [ ] 不支持的能力必须明确报错，不伪造流或静默降级。

### 错误与取消

- [x] 实现设计中定义的完整 `BridgeError` 分类和 `ErrorMetadata`。
- [x] 保留 Provider、HTTP 状态码、请求 ID、retryable 和 retry-after 等可判断事实。
- [ ] 对 Future/Stream drop 定义并实现取消语义。
- [ ] 将完成前的流失败映射为 `StreamInterrupted`。
- [ ] 确保错误 Display、Debug 和 tracing 不包含 Secret 或完整敏感响应。

### 配置、Secret 与 Factory

- [x] 实现版本化 `BridgeConfig`、`TransportConfig`、`CredentialRef` 和
      `ResolvedBridgeConfig`。
- [x] 支持 TOML、JSON 和 Rust Builder 生成同一个配置模型。
- [x] 实现 Environment、File 和宿主 Resolver 三种 Secret 解析路径。
- [x] Secret Resolver 保持 object-safe 和运行时无关；File Secret 只移除一个结尾换行。
- [x] 确保 Secret 值不进入可序列化配置、Debug 或 tracing。
- [x] 在构造阶段校验配置版本、Provider、model、transport 和 `provider_options`。
- [x] 默认允许通过通用 URL 校验的自定义 endpoint，并允许宿主选择性限制 scheme、host 或
      网络范围。
- [x] 实现 object-safe `BridgeFactory`。
- [x] 在 `armillae-llm-rig` 中直接提供第一阶段的 `RigBridgeFactory`。
- [x] 不提前实现动态 Adapter 插件 Registry。

### MockBridge 与共享合约

- [x] 通过 `mock` feature 提供 Mock 和共享 Bridge 测试设施，默认构建不启用。
- [x] 实现固定非流式响应和按调用顺序返回的脚本响应。
- [x] 实现文本流式增量和 ToolCall 参数分片。
- [x] 支持注入 Provider 错误和流中断。
- [x] 记录收到的请求，供下游测试断言。
- [x] 建立可由 Mock 和真实 Adapter 复用的 Bridge 合约测试框架。

## P4：`armillae-llm-rig` 非流式 Adapter

### 隔离与转换

- [x] 使用泛型 `RigBridge<M>` 适配 `CompletionModel`，对外擦除为 `Arc<dyn LlmBridge>`。
- [x] 为 `RigBridge<M>` 注入私有 Provider Request Mapper，将标准请求字段和当前 Provider 的
      命名空间扩展显式映射到 rig 请求，不在通用转换中硬编码 Provider wire shape。
- [x] 为 `RigBridge<M>` 注入私有 Provider Response Normalizer，从 raw response 标准化
      finish reason、实际模型、ID 和安全 metadata，不依赖内容猜测。
- [x] 按设计合并构造期生成默认值与单次请求参数，并覆盖未指定/覆盖/stop 空列表语义。
- [x] 将所有 Armillae/Rig 转换集中在独立 `convert` 模块。
- [x] 转换 Message、Tool Definition、Completion Request、Completion Response 和错误。
- [x] 保持 ToolCall ID、多个 ToolCall 的顺序和 ToolResult 关联。
- [x] 为 `ToolResult.is_error` 实现 Provider 显式兼容策略：原生支持时映射，不支持时按设计
      保留 Armillae 语义且不得拒绝请求或改写模型可见内容。
- [x] 将未知 Provider 输出转换为 `ProviderData`，不静默丢失。
- [x] 请求扩展只读取当前 Provider/Adapter 命名空间，拒绝未知字段、错误类型及对标准字段的
      重复设置或覆盖。
- [x] 只将脱敏、受控的 Provider metadata 暴露到公共响应和错误。
- [x] 禁止依赖 Rig Agent 的 Tool 注册、执行、Memory、RAG 或 Hook 路径。

### OpenAI/OpenAI-compatible

- [x] 实现 OpenAI Provider factory 和配置转换。
- [x] OpenAI/OpenAI-compatible 使用固定 Provider 能力预设；已知模型限制只允许收紧，未知
      模型不得阻止构造，远端能力偏差必须返回 `ProviderRejected` 而非静默降级。
- [x] OpenAI 与 OpenAI-compatible 都要求 credential；后者必须显式提供自定义 endpoint，且
      不得用空凭证或伪造凭证模拟无认证请求。
- [x] OpenAI Rig Adapter 不支持 Developer role 时必须在能力中声明并本地拒绝，不得转换为
      System。
- [x] 支持 OpenAI-compatible 自定义 endpoint。
- [x] 完成纯文本的非流式请求与响应。
- [x] 将 `stop`、`seed`、JsonObject 及 JSON Schema 的 name/strict 无损映射到 OpenAI 请求。
- [x] 完成 Tool Definition、单 ToolCall 和多 ToolCall 转换。
- [x] 完成 Assistant ToolCall + ToolResult 的后续请求转换。
- [x] 验证 OpenAI ToolResult 转换保留 `call_id`、content 和顺序，不下发 `is_error`，且
      `is_error = true` 时不拒绝请求、不自动包装内容。
- [x] 完成 Usage、finish reason 和标准化错误映射。
- [x] 通过共享 Bridge 合约测试。
- [x] 使用显式下游流程验证 `LLM -> ToolCall -> ToolResult -> LLM` 闭环。

## P5：Streaming

### 流式转换与重组

- [ ] 将 Provider 原始 SSE/NDJSON chunk 转换为 Armillae 语义事件。
- [ ] 实现文本内容的 started/delta/completed 事件。
- [ ] 按稳定 index/call 分别缓冲 ToolCall 名称和参数。
- [ ] 支持名称、JSON token 和 UTF-8 字符跨底层 chunk。
- [ ] 支持多个 ToolCall 的交错增量。
- [ ] 完整参数解析成功后生成 `ToolCallCompleted`。
- [ ] 汇总 Usage、finish reason、ProviderData 和最终 `CompletionResponse`。
- [ ] 保证最终流式响应与等价非流式响应具有一致语义结构。
- [ ] 未识别 Provider 事件通过 `ProviderEvent` 暴露。

### Streaming 测试

- [ ] 使用任意数量和边界的文本 chunk 验证重组一致性。
- [ ] 覆盖 Tool 名称、JSON token 和 UTF-8 字节边界分片。
- [ ] 覆盖多 ToolCall 交错、稳定 index 和 ID 保持。
- [ ] 覆盖完成时 JSON 无效和流中断路径。
- [ ] 覆盖 Usage 位于最终事件或独立事件的两种路径。
- [ ] 验证成功流只产生一个 `ResponseCompleted`，失败流不产生该事件。
- [ ] 验证 drop Stream 后底层调用取消。

## P6：更多 Provider

### Anthropic

- [ ] 实现 Anthropic Provider factory、配置和能力矩阵。
- [ ] 明确处理 System/Developer role、ToolChoice 和 Tool Result 的 Provider 差异。
- [ ] 完成非流式文本、单/多 ToolCall 和后续 ToolResult 请求。
- [ ] 完成流式文本与 ToolCall 参数重组。
- [ ] 完成 Usage、finish reason、错误和未知事件映射。
- [ ] 通过共享 Bridge 与 Streaming 合约测试。

### Ollama

- [ ] 实现 Ollama Provider factory、配置和能力矩阵。
- [ ] 完成本地 endpoint 与 NDJSON 传输路径。
- [ ] 完成非流式文本、单/多 ToolCall 和后续 ToolResult 请求。
- [ ] 完成流式文本与 ToolCall 参数重组。
- [ ] 完成 Usage、finish reason、错误和未知事件映射。
- [ ] 通过共享 Bridge 与 Streaming 合约测试。

### Provider 一致性

- [ ] 为所有 Provider 维护同一外部协议和明确的能力矩阵。
- [ ] Provider 不支持的能力由本地预检拒绝，不静默降级。
- [ ] 转换单元测试保持离线，Mock HTTP/cassette 测试不依赖真实 Provider。
- [ ] Live 测试默认 ignored，仅在发布前使用明确提供的真实凭证运行。

## 横切要求

### 可观测性

- [ ] 提供结构化 tracing，记录 Adapter、Provider、model、请求 ID 和是否流式。
- [ ] 记录 Tool Definition/ToolCall 数量、token usage、总延迟和首 token 延迟。
- [ ] 使用标准化错误类别，避免记录完整正文或原始敏感响应。
- [ ] 内容级调试必须显式启用，并允许宿主提供脱敏器。

### 安全

- [ ] 验证 API Key、Authorization header 和 Secret 不出现在日志、错误、Debug、fixture 或快照。
- [ ] 验证默认不记录完整消息、Tool 参数和 ToolResult。
- [ ] 验证动态 endpoint 配置不能绕过宿主 SSRF 限制。
- [ ] 验证 `provider_options` 未知字段和错误类型在构造阶段被拒绝。
- [ ] 在提交 fixture 前扫描真实凭证、用户隐私内容和未经脱敏的 Provider 响应。

### 示例与文档

- [ ] 添加 `simple_completion` 示例。
- [ ] 添加 `streaming` 示例。
- [ ] 添加显式 `manual_tool_flow` 示例。
- [ ] 文档化配置文件与 Rust Builder 的等价用法。
- [ ] 文档化 Provider 能力矩阵、兼容策略和不支持能力的错误行为。
- [ ] 文档化 Secret、endpoint、日志和 Live 测试的安全边界。

## 第一阶段完成条件

- [ ] 同一 `CompletionRequest`/`CompletionResponse` 协议可用于所有已支持 Provider。
- [ ] TOML、JSON 和 Rust Builder 可生成同一个 Bridge 实例。
- [ ] 非流式和流式文本响应均通过共享合约测试。
- [ ] Tool Definition 可以发送给模型。
- [ ] 单个和多个 ToolCall 均可完整解析并保持顺序与 ID。
- [ ] ToolCall 参数在任意流式分片下无损重组。
- [ ] ToolResult 可以作为后续请求消息发送给模型。
- [ ] 下游可以通过 ToolRegistry 类型安全地执行 ToolCall。
- [ ] Bridge 不执行 Tool，Tool Executor 不调用 Bridge。
- [ ] Usage、finish reason、请求 ID 和错误类别已标准化。
- [ ] MockBridge 和所有真实 Adapter 均通过共享合约测试。
- [ ] 除 `armillae-llm-rig` 外没有 crate 依赖或暴露 rig 类型。
- [ ] 格式检查、Clippy、单元测试、文档构建和离线合约测试全部通过。
- [ ] `docs/DESIGN.md`、`TODO.md`、示例和当前实现保持一致。

## 第一阶段之后（当前不实施）

以下方向保留在设计文档的后续演进章节中，不应在第一阶段 TODO 中拆成实施任务：

- `armillae-turn` 与有界 Tool Loop；
- 多 ToolCall 串行、并行或 Executor-defined 调度；
- 人工审批、权限与副作用策略；
- MCP、远程或录制/回放 ToolExecutor；
- 多模态内容、Provider 路由、回退与负载均衡；
- `armillae-embedding` 与 Provider 无关的 Embedding Bridge；
- `armillae-vector-store` 与具体向量数据库 Adapter；
- `armillae-rag` 对 Embedding、Vector Store、重排、上下文组装和 LLM 调用的上层编排；
- Conversation Memory、叙事上下文、完整 Agent 和世界运行时。
