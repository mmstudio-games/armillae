# RFC 0003：LLM canonical 投影与模型 fallback

> 状态：Accepted
> 接受日期：2026-08-27
> 设计入口：[Armillae 设计索引](../DESIGN.md)
> 生效规范：[LLM Bridge、Router 与 Tool Executor Spec](../specs/llm-bridge.md)
> 实施清单：[LLM Bridge、Router 与 Tool Executor TODO](../todos/llm-bridge.md)

## 1. 决策摘要

Armillae 的 `CompletionRequest`、`CompletionResponse` 和消息历史是 LLM 链路的 canonical
数据，不以任一 Provider、Driver 或 SDK 类型作为事实来源。Provider Adapter 在网络边界为
目标 Provider 生成 request projection，并把响应反序列化回同一 canonical 协议。

Adapter 必须为自己产生且 Provider 支持回放的私有数据提供双向转换。切换 Provider 时，源
Provider 私有数据继续保留在 canonical history 中，但不要求发送给无关的目标 Provider；这类
未投影事实必须结构化记录，而不是删除 canonical 数据或把它当作整个 LLM 链路的终止条件。

模型选择和 fallback 由独立于单个 `LlmBridge` 的 Router 承担。单个 Bridge 继续表示一次
Provider Model Call；Router 可以在一次逻辑 LLM 请求中按宿主提供的候选和策略执行多个 Bridge
attempt，但不执行 Tool、不维护 Conversation Memory，也不拥有 Turn。

## 2. 背景与问题

第一阶段实现已经做到：

- 公共 API 和可序列化消息使用 Armillae 类型；
- rig 类型只存在于 `armillae-llm-rig`；
- Provider 特有响应内容通过 `ProviderData` 保留；
- 不支持的标准能力通过 `UnsupportedCapability` 明确报告。

真实 DeepSeek 多轮对话暴露了未闭合的边界：响应侧把 `reasoning_content` 保存成
`ProviderData { provider = "deepseek", kind = "reasoning" }`，`as_assistant_message()` 将它
保留到 canonical history；下一次请求的公共 Rig 转换却拒绝所有 `ProviderData`。因此同一个
Bridge 产生的合法响应无法安全回放给自己，Tool 示例只是最先暴露了这一普遍问题。

同时，当前能力预检只有“发送”或“终止”两种结果，没有候选路由、目标 Provider 投影、兼容
事实或模型 fallback。该行为适合第一阶段的严格单 Provider 合约，但不满足 Armillae 尽可能
保持 LLM 链路可用、兼容未知模型和支持显式 fallback 的长期目标。

## 3. 目标

- 保持 Armillae 协议和 canonical history 为唯一事实源。
- 为每个 Provider 建立明确的请求投影与响应反序列化闭环。
- 同 Provider 回放已知私有数据，尤其是 reasoning、签名和 ToolCall 关联 metadata。
- 跨 Provider fallback 时保留 canonical 数据，只向目标 Provider 投影其可表达内容。
- 将不投影、兼容转换、候选跳过和 attempt 失败表达为结构化事实。
- 允许宿主按运行时配置提供候选 Provider/model 和 fallback 策略。
- 保持 `LlmBridge`、`ToolExecutor` 与 Agentic Runtime 的既有单向依赖边界。

## 4. 非目标

- 宣称无需 Adapter 或协议验证即可调用世界上所有 Provider 和模型。
- 在 Router 中执行 Tool、自动推进 Tool Loop、维护 Memory 或拥有 Turn。
- 自动发现凭证、扫描模型列表、探测任意 endpoint 能力或提供全局 Provider Registry。
- 将一个 Provider 的私有 reasoning、签名或原始 metadata 伪装成另一个 Provider 的输入。
- 在调用方不知情时把 Specific ToolChoice 改成 Auto、把 Developer 改成 System，或伪造
  Structured Output、ToolCall ID 和 Usage。
- 在已经向调用方发出流式语义内容后切换 Provider 并拼接另一条流。

## 5. 术语

### 5.1 Canonical request/history

调用方持有的 Armillae `CompletionRequest` 及其 `Message` 序列。Provider 投影不得原地修改、
删除或用目标 Provider wire shape 覆盖它。

### 5.2 Provider projection

Adapter 从 canonical request 为一个确定的 Provider/model 生成该目标可接受的请求。Projection
是一次派生过程，不是 canonical 数据迁移。

### 5.3 Replay data

Provider 响应产生、后续同 Provider 请求可能需要回传的私有内容，例如 DeepSeek reasoning、
Anthropic signed reasoning 或 Provider ToolCall metadata。

### 5.4 Compatibility fact

投影或路由过程中发生的结构化事实，例如私有数据未发送到不同 Provider、某候选因能力不足
被跳过、某个显式允许的兼容转换被采用。它不得包含完整消息正文、Tool 参数、ToolResult、
Secret 或原始 Provider 响应。

### 5.5 Candidate 与 attempt

Candidate 是宿主提供的一个具名 Provider/model Bridge。Attempt 是 Router 对某个 Candidate
执行的预检、投影和至多一次 Bridge 调用。

## 6. Canonical 数据与 Provider 投影

### 6.1 数据分类

Adapter 必须按目标 Provider 将内容分为：

1. **标准可移植内容**：Text、Role、ToolCall、ToolResult、ToolChoice、OutputFormat 和生成参数；
2. **同 Provider replay data**：由该 Adapter 明确认识并能够还原的 `ProviderData`；
3. **非请求观察数据**：Usage、请求 ID、system fingerprint 和只用于诊断的 metadata；
4. **外部或未知 ProviderData**：不属于目标 Provider，或目标 Adapter 尚未声明回放语义的数据。

标准内容按目标协议编码。同 Provider replay data 必须验证结构并还原到 Provider/Driver 类型；
不得先序列化成 `ProviderData`，随后在下一轮无条件拒绝。观察数据不进入消息请求。外部或未知
ProviderData 保留在 canonical history 中，但不注入无关目标 Provider 的 wire request，并产生
`not_forwarded` compatibility fact。

### 6.2 不可安全投影的数据

标准字段无法保持其语义、同 Provider 已知 replay data 结构损坏、ToolCall/ToolResult 关联不
完整，或者投影会越过安全策略时，本 Candidate 的 projection 必须失败。直接调用 Bridge 时向
调用方返回结构化错误；通过 Router 调用时，该失败可以按策略成为选择下一 Candidate 的依据。

Adapter 不得通过丢弃标准 Role、ToolCall、ToolResult、调用 ID 或必需的 Provider 签名来制造
一个表面成功的请求。跨 Provider 不发送源 Provider 私有数据不属于删除 canonical 数据，但
必须保留 compatibility fact。

### 6.3 双向但窄化的 Adapter 边界

`RigRequestMapper` 和 `RigResponseNormalizer` 继续保持各自单向、私有和不负责传输；不合并为
包含 HTTP Client、路由和状态的宽泛 Provider 对象。它们必须共享同一 Provider projection
规则或窄化 codec helper，使以下关系通过合约测试：

```text
Provider response
    │ decode
    ▼
Armillae canonical content
    │ encode to the same Provider
    ▼
Provider replay request
```

该闭环只要求保持后续请求所需语义，不要求逐字节复现原始响应，也不得把完整原始响应塞入
`ProviderData`。

## 7. 能力协商与兼容策略

`BridgeCapabilities` 继续表示一个 Candidate 的可用能力，并在网络前完成预检。它对直接 Bridge
调用仍可产生终止错误；在 Router 中，`UnsupportedCapability` 是候选不匹配事实，不自动等于
整个逻辑请求失败。

Router 必须优先选择无需语义降级即可表达请求的 Candidate。只有宿主策略显式允许时，Adapter
或 Router 才能采用已经在 Spec 中命名的兼容转换，并记录转换前后语义。以下事实永远不能由
通用“尽力而为”开关隐式改写：

- Role 的权限语义；
- ToolCall ID、ToolResult 关联与内容顺序；
- ToolChoice 的约束强度；
- Structured Output 的 schema 和 strict 语义；
- Secret、endpoint 和宿主安全策略；
- Usage、finish reason 和错误类别的事实值。

未知模型名称本身不能阻止 Adapter 构造。Adapter 以 Provider 协议 profile 和已验证的模型
限制提供能力事实；远端实际偏差继续标准化为 Provider 错误，并可由 Router 策略决定是否尝试
下一 Candidate。

## 8. LLM Router 与 fallback

### 8.1 责任与依赖

Router 属于运行时无关的 LLM 基础设施，组合宿主已经构造好的 `Arc<dyn LlmBridge>` Candidate。
它不解析宿主配置文件，不创建全局 Registry，不依赖具体 Driver，也不反向依赖 Agentic Runtime。

Provider projection 仍由 Candidate 对应的 Adapter 实现，Router 不读取或构造 Provider wire
类型。P7 必须为 Candidate 提供 Provider 无关、object-safe 的预检/投影结果边界，使 Router 能在
不复制 Adapter codec 的前提下判断 compatibility facts、projection failure 和请求是否已经发送；
直接 Bridge 调用与 Router attempt 必须复用同一套投影规则，不能形成两条行为不同的转换路径。

Router 使用独立 API，而不实现或伪装成“一次调用”的 `LlmBridge`。路由结果必须让调用方判断：

- 最终选择的 Candidate；
- 每个已评估 Candidate 是否只做了预检、是否实际发出请求；
- 安全脱敏后的失败类别；
- 所有 compatibility facts。

### 8.2 路由顺序

一次逻辑请求遵循：

1. 使用原始 canonical request 评估 Candidate；
2. 跳过无法精确表达请求的 Candidate，除非策略允许已命名兼容转换；
3. 为当前 Candidate 生成独立 Provider projection；
4. 执行至多一次 Bridge Model Call；
5. 成功后立即返回；失败时只有满足 fallback policy 才进入下一 Candidate；
6. 所有 Candidate 都不可用时返回聚合但脱敏的 routing error。

Router 每次 attempt 都从同一 canonical request 重新投影，不得把前一个 Provider 的 wire request
作为下一个 Provider 的输入。

`not_forwarded` compatibility fact 本身不是 Candidate failure：外部或未知 ProviderData 未进入
目标 wire request 时，只要所有标准语义仍可安全表达，该 Candidate 仍可继续调用。只有标准语义
无法保持、同 Provider 已知 replay data 无效或安全策略失败时，projection 才失败。

### 8.3 默认 fallback 边界

默认可进入下一 Candidate 的事实包括：

- `UnsupportedCapability` 或 Candidate projection 不兼容；
- Rate limit；
- Timeout；
- 明确标记 retryable 的 Transport error。

以下错误默认终止逻辑请求：

- 调用方 `InvalidRequest`；
- `Cancelled`；
- Secret、endpoint 或宿主安全策略失败；
- Authentication 和 PermissionDenied；
- 已经向调用方发出任何流式语义内容后的错误。

`ProviderRejected`、非 retryable Transport 和 InvalidProviderResponse 是否 fallback 由显式策略按
Provider/code/status 分类，Router 不根据错误字符串猜测。策略可以比默认值更严格；放宽时必须
可观察，并保持每个 Candidate 独立凭证和 endpoint 策略。

### 8.4 Streaming

Streaming Router 可以在创建流失败或首个语义事件前按策略选择下一 Candidate。一旦向调用方
发出 `ResponseStarted` 或任何内容/Usage/ProviderEvent，就固定当前 Candidate；后续失败按
`StreamInterrupted` 返回，不能拼接另一 Provider 的流。Drop Router Future/Stream 必须取消
当前 attempt，且不得启动新的 fallback。

## 9. 安全与可观测性

- Candidate 使用各自已经解析和脱敏的 credential，不共享 Secret 值。
- fallback 不得绕过每个 endpoint 的结构校验和宿主 EndpointPolicy。
- Attempt 与 compatibility fact 默认只记录 Candidate 标识、Provider、model、动作、错误类别、
  延迟和 token facts，不记录消息正文、reasoning、Tool 参数或 Provider 原始响应。
- Canonical history 中保留 ProviderData 不代表允许将其写入普通 tracing 或错误 Display。
- Router 聚合错误不得包含 Authorization header、Secret、完整 URL query 或响应正文。

## 10. 被拒绝的方案

### 10.1 在示例或历史中删除 ProviderData

拒绝。它会破坏 canonical history，并可能让 DeepSeek、Anthropic 等需要回放 reasoning 或签名
的 Tool continuation 被远端拒绝。

### 10.2 将所有 ProviderData 无条件发送给目标 Provider

拒绝。不同 Provider 的私有数据没有共享 wire 语义，可能造成协议错误、信息泄漏或跨 Provider
耦合。

### 10.3 所有不支持能力都立即终止

拒绝作为 Router 语义。对直接 Bridge 仍可严格失败，但 Router 应把候选能力不匹配作为选择
下一 Candidate 的结构化事实。

### 10.4 在单个 Provider Adapter 内实现 fallback

拒绝。Adapter 不应拥有候选顺序、成本、凭证、跨 Provider 错误策略或多次 Model Call。

### 10.5 只保留最低公分母协议

拒绝。它会丢失 Tool、reasoning、结构化输出和 Provider 新能力，也无法可靠回放同 Provider
历史。

## 11. 实施影响与顺序

1. 修复所有 Adapter 的 ProviderData 生命周期审计，先完成 DeepSeek reasoning 的响应到同
   Provider 请求闭环；
2. 为 Anthropic、Ollama、OpenAI/OpenAI-compatible、MiniMax 和 Moonshot 分类 replay data、
   观察数据与未知数据；
3. 建立 Provider projection 与 compatibility fact 的共享私有合约，不让 Rig 类型穿透；
4. 在 `armillae-llm` 中建立 host-owned Candidate、Router、policy、attempt/result/error 公共
   合约；
5. 实现非流式 fallback，再实现首个语义事件前可 fallback 的 Streaming；
6. 使用 Mock、Mock HTTP 和默认 ignored Live 测试验证同 Provider 回放和跨 Provider fallback。

本 RFC 不要求为现有 `ProviderData` 立即增加公共字段。Adapter 可以先按 `(provider, kind)`
注册已知 replay 规则；如果实现证明调用方必须持久化 replay/observation 分类，再通过公共协议
变更扩展 Schema，不能仅凭预想提前加入字段。

## 12. 验收标准

- `response.as_assistant_message()` 对同一个 Bridge 产生的已知 replay data 可以进入下一请求；
- DeepSeek reasoning + ToolCall + ToolResult continuation 不因 ProviderData 本地失败；
- 跨 Provider fallback 不修改 canonical history，也不把源 Provider 私有数据发送给目标；
- 标准 Text、Role、ToolCall、ToolResult、ID 和内容顺序在每次 projection 中保持；
- 未投影和兼容转换都有结构化、脱敏的 compatibility fact；
- Router 使用宿主候选顺序，遵守默认/显式 fallback policy 和取消语义；
- Streaming 在首个语义事件后绝不跨 Provider 拼接；
- Bridge 仍只执行一次 Provider Model Call，ToolExecutor 仍只执行一次 ToolCall；
- Rig 类型不穿透 `armillae-llm-rig`；
- 转换单测、共享合约、Mock HTTP、严格 Clippy、Rustdoc 和默认 ignored Live 门禁覆盖新边界。
