# rig-core 0.41.0 P0 可行性 Spike

> 日期：2026-08-15  
> 结论：通过  
> 依赖：`rig-core = "=0.41.0"`

## 目标

在冻结 Armillae 公共协议前，验证 `rig-core` 的低层 Completion API 能否支持第一阶段的
LLM Bridge 与 Tool Calling 设计。Spike 只验证 Adapter 所需的底层能力，不实现公共 API，
也不引入 Agent、自动 Tool Loop、Memory 或 RAG。

## 验证结果

- 可以直接为 `CompletionModel` 构造 `CompletionRequest`，调用 `completion` 与 `stream`，
  不依赖 Rig Agent runtime。
- 请求能够携带多个 Tool Definition；响应能够保留单个或多个 ToolCall 的 ID 与原始顺序。
- Assistant ToolCall 和手工构造的 ToolResult 可以进入后续消息历史。
- OpenAI 将 Tool Definition 编码为 `type=function`，将 Assistant 调用编码为
  `tool_calls`，并将每个 ToolResult 编码为 `role=tool` 消息。
- Anthropic 将 Tool Definition 编码为 `input_schema`，将调用编码为 `tool_use` content
  block，并将 ToolResult 编码为 user message 中的 `tool_result` block；请求必须提供
  `max_tokens`，响应顶层类型为 `message`。
- OpenAI 与 Anthropic 的真实 Rig Provider model 均通过离线 Recording HTTP fixture 完成
  请求编码、响应解析和通用 ToolCall 归一化，无需真实凭证或网络请求。
- 流式文本、ToolCall 名称和 JSON 参数可以表达为增量事件。OpenAI-compatible streaming
  按 Provider index 维护调用状态，通过稳定的 `internal_call_id` 关联后续 delta，并在完成
  时解析完整 JSON、按 index 输出 ToolCall。
- 多个 ToolCall 的名称和参数可以交错到达并独立重组；7 字节 HTTP 分片覆盖了 SSE、JSON
  token 和中文 UTF-8 字符边界，结果保持 ID、顺序和参数值。
- `StreamingCompletionResponse` 只暴露首个 Final response，并从中汇总 Usage。
- drop 未完成的 Completion Future 或 `StreamingCompletionResponse` 会向下释放内部 Future
  或 Stream；Rig 另提供显式 `cancel`，但客户端取消不能证明远端 Provider 已停止计算。

## Provider 与 Adapter 注意事项

| 方面 | OpenAI/OpenAI-compatible | Anthropic | Armillae Adapter 要求 |
|---|---|---|---|
| Tool 定义 | `type=function` + `function.parameters` | `name` + `input_schema` | 从统一 `ToolDefinition` 显式转换 |
| Assistant 调用 | `tool_calls[]` | `tool_use` content block | 保持调用 ID 和内容顺序 |
| ToolResult | 独立 `role=tool` 消息 | user message 内 `tool_result` block | 保持 `tool_use_id` 关联，不暴露原生类型 |
| 流式关联 | Provider index；后续 delta 可不含外部 ID | content block/index 语义 | 为每个内容分配稳定 Armillae index 并独立缓冲 |
| Token 限制 | 可选参数由模型接口表达 | `max_tokens` 必填 | 在能力预检或 Provider 默认策略中显式处理 |
| 原始响应 | Provider 专用类型 | Provider 专用类型 | 受控转换为标准字段或 `ProviderData`，不得静默丢失 |

## 已知限制

- Spike 是纯离线测试，没有验证真实 Provider 的网络行为、限流、错误正文或服务端取消。
- 当前只验证了 OpenAI-compatible 的完整流式 HTTP 转换路径；Anthropic streaming 留在 P6
  的共享 Streaming 合约测试中验证。
- Rig 的 Provider 响应、错误和部分 streaming 事件仍是 Provider 专用语义，后续 Adapter
  必须实现 Armillae 的能力预检、错误分类、脱敏和 `ProviderData` 转换。
- `internal_call_id` 是 Adapter 内部关联手段，不属于 Armillae 公共或持久化协议。

## 决策

第一阶段采用并精确锁定 `rig-core 0.41.0`，继续保持其只存在于
`armillae-llm-rig`。当前没有启用 `genai` 或原生 Provider Adapter 的必要。若升级 Rig、
转换合约不再成立或 Provider 能力无法无损表达，应先复跑本 Spike 和共享 Bridge 合约；失败
时再按设计顺序更新 `docs/DESIGN.md`，评估替代 Adapter。

测试证据位于 `crates/armillae-llm-rig/tests/p0_spike.rs`，运行方式：

```bash
rtk cargo test -p armillae-llm-rig --test p0_spike
```
