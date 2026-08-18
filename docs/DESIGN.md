# Armillae 第一阶段技术设计：LLM Bridge 与 Tool Executor

> 状态：Draft  
> 设计基线：2026-08-13  
> 适用范围：Armillae 第一阶段

## 1. 背景

Armillae 的长期目标是成为一个面向 Agentic 叙事的通用运行时，可被上层用于构建叙事引擎、TRPG 运行时以及大世界游戏引擎。长期系统将涉及上下文组织、叙事状态、世界状态、工具调度、持久化、回放以及更高层的 Agent 行为，但这些能力不属于第一阶段的实现范围。

第一阶段聚焦两个基础设施能力：

1. **LLM Bridge**：通过统一协议连接不同 LLM Provider，支持普通内容、流式内容以及完整 Tool Calling 协议。
2. **Tool Executor**：让下游以类型安全的方式实现 Tool，并可根据 LLM 返回的 `ToolCall` 显式执行 Tool。

Armillae 在本阶段不实现完整 Agent，也不实现自动的多轮 Tool Continuation Loop。下游可以用 Bridge 和 Tool Executor 自行组织如下流程：

```text
输入上下文
    │
    ▼
LLM Bridge ───────────────► LLM
    │                        │
    │                        ▼
    │                    ToolCall
    │                        │
    ▼                        ▼
下游调度器 ◄────────── Tool Executor
    │
    │ 将 Assistant ToolCall 与 ToolResult 加入上下文
    ▼
再次调用 LLM Bridge
    │
    ▼
最终内容
```

未来可以在这两个模块之上实现 `armillae-turn`，封装一次用户交互内的有界 Tool Loop；该层不应要求修改本设计中的 Bridge 或 Tool Executor 核心接口。

长期的检索与知识增强能力按独立职责演进：`armillae-embedding` 提供 Provider 无关的
Embedding Bridge，`armillae-vector-store` 提供数据库无关的向量存储与检索接口，未来的
`armillae-rag` 组合 Embedding、Vector Store、重排、上下文组装与 LLM 调用。上述能力均不
属于第一阶段，当前 LLM Bridge 不承载 Embedding、向量存储或 RAG 编排。

## 2. 术语

### 2.1 Model Call

一次对模型 Provider 的请求。输入是一组消息、Tool 定义和生成参数，输出是 Assistant 内容、ToolCall 或两者的组合。

### 2.2 Tool Definition

提供给 LLM 的工具描述，包含名称、说明和 JSON Schema。它只告诉模型“有哪些能力可被请求”，不包含实际执行逻辑。

### 2.3 ToolCall

LLM 返回的结构化调用意图，包含调用 ID、工具名称和参数。ToolCall 本身不产生任何业务副作用。

### 2.4 Tool Execution

宿主根据 ToolCall 查找 Tool 实现、解析参数并实际执行函数、网络请求或游戏操作的过程。

### 2.5 ToolResult

Tool 执行结果的协议表示。下游可以将其加入消息历史，并通过新的 Model Call 交还给 LLM。

### 2.6 Turn

一次用户输入到最终 Assistant 输出的完整交互，中间可能包含多个 Model Call 和 Tool Execution。Turn 是未来模块，本阶段不实现。

### 2.7 Agent

跨 Turn 持有目标、记忆、规划与自主行为的上层系统。Agent 不属于本阶段范围。

## 3. 目标与非目标

### 3.1 目标

- 定义稳定、Provider 无关的消息和 Completion 协议。
- 支持从结构化配置创建 LLM Bridge；配置既可从文件解析，也可在运行时动态构造。
- 通过同一接口使用不同 LLM Provider。
- 支持非流式和流式 Model Call。
- 支持完整的 Tool Calling 协议：
  - 向模型发送 Tool Definition；
  - 接收一个或多个 ToolCall；
  - 在消息历史中表达 Assistant ToolCall；
  - 将 ToolResult 作为输入发送给模型；
  - 在流式响应中重组 ToolCall 参数。
- 提供类型安全的 Tool 开发接口和可动态注册的 Tool Executor。
- 隔离 rig-rs，使其仅作为可替换的 Provider Adapter。
- 提供 Mock、合约测试和协议转换测试，保证未来更换 Adapter 时上层行为不变。

### 3.2 非目标

第一阶段明确不实现：

- 自动 Tool Loop 或 Turn Runner；
- 完整 Agent、规划器或工作流编排；
- 跨 Turn 的 Conversation Memory；
- Embedding、RAG、向量数据库或上下文检索；
- Tool 批量调度、并发策略、自动重试或人工审批；
- 世界状态、叙事状态或游戏事务；
- 长期 transcript 持久化和存档；
- 由 Bridge 自动执行 Tool；
- 由 Tool Executor 自动再次调用 LLM。

## 4. 核心设计决策

### 4.1 Armillae 拥有公共协议，rig 仅负责适配

Armillae 的公共 API、配置和持久化数据中不得暴露 rig 类型。第一阶段的 `rig-core` 只允许
出现在 `armillae-llm-rig` 中。

原因如下：

- Armillae 的协议稳定性不能绑定到一个仍在快速演进的 0.x 依赖。
- rig 的 `CompletionModel` 使用关联类型和返回位置 `impl Future`，不能直接作为 `dyn CompletionModel` 使用。
- Armillae 需要由配置在运行时创建异构 Provider 实例，因此需要自己的 object-safe Bridge。
- 未来可以增加基于其他库或原生 SDK 的 Adapter，而不改变下游接口。

### 4.2 LLM Bridge 只执行一次 Model Call

Bridge 接收完整请求并返回一次模型响应。即使响应包含 ToolCall，Bridge 也不会执行 Tool 或继续调用模型。

### 4.3 Tool 协议与 Tool 实现分离，但 Schema 与实现保持关联

`ToolDefinition`、`ToolCall` 和 `ToolResult` 是共享协议；类型化 `Tool` 同时提供参数类型和执行逻辑，由参数类型生成 JSON Schema，避免 Schema 与实现漂移。

### 4.4 Tool Executor 负责单个 ToolCall 的执行

本阶段的 Executor 只定义一次 `ToolCall -> ToolResult`。多个 ToolCall 的顺序、并发、审批和失败策略由调用方决定。

### 4.5 保留 Provider 扩展而不追求最低公分母

统一协议覆盖稳定的公共能力，同时提供受控的 Provider 扩展字段。无法统一的输入和输出不应被静默丢弃。

### 4.6 按模型能力划分 crate

当前提供 LLM Bridge 的 crate 命名为 `armillae-llm`，其 rig Adapter 命名为
`armillae-llm-rig`。`Bridge` 保留为职责和 trait 概念，例如 `LlmBridge`；crate 名使用具体
模型能力，避免未来出现多个 Bridge 后产生歧义。

未来能力按以下边界独立演进，不合并进 `armillae-llm`：

- `armillae-embedding`：一次或批量 Embedding Model Call，统一 Dense、Sparse 和
  Multivector 等能力差异；对应公共接口为 `EmbeddingBridge`。
- `armillae-vector-store`：数据库无关的向量写入、删除、过滤与检索接口；具体数据库通过
  独立 Adapter 接入。
- `armillae-rag`：组合 Embedding、Vector Store、可选重排、上下文组装和 LLM 调用的上层
  编排。

LLM、Embedding 和 Vector Store 的请求、响应、能力与错误语义分别定义，不抽象一个统一的
万能 Bridge trait。只有在实际实现暴露稳定的共同需求后，才考虑复用认证、Endpoint 或传输
配置。该命名与长期边界决定不改变第一阶段范围。

## 5. Workspace 与 crate 结构

Workspace 使用 Rust 2024 edition 和 Cargo resolver 3。根 manifest 统一管理 workspace package
元数据；各 crate 在初始化阶段只保留空的 library target，不提前实现公共类型或业务逻辑。
初始化同时建立设计要求的本地 crate 依赖方向，并提供统一的格式检查、Clippy、测试和文档
构建命令。

```text
armillae/
├── Cargo.toml
├── crates/
│   ├── armillae-core/
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── message.rs
│   │       ├── completion.rs
│   │       ├── tool.rs
│   │       ├── stream.rs
│   │       ├── usage.rs
│   │       └── error.rs
│   │
│   ├── armillae-llm/
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── bridge.rs
│   │       ├── capability.rs
│   │       ├── config.rs
│   │       ├── factory.rs
│   │       ├── secret.rs
│   │       └── mock.rs
│   │
│   ├── armillae-tools/
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── tool.rs
│   │       ├── dyn_tool.rs
│   │       ├── executor.rs
│   │       ├── registry.rs
│   │       ├── context.rs
│   │       └── error.rs
│   │
│   └── armillae-llm-rig/
│       └── src/
│           ├── lib.rs
│           ├── adapter.rs
│           ├── factory.rs
│           ├── convert/
│           │   ├── mod.rs
│           │   ├── message.rs
│           │   ├── request.rs
│           │   ├── response.rs
│           │   ├── stream.rs
│           │   └── tool.rs
│           └── providers/
│               ├── mod.rs
│               ├── openai.rs
│               ├── anthropic.rs
│               └── ollama.rs
│
└── examples/
    ├── simple_completion.rs
    ├── streaming.rs
    └── manual_tool_flow.rs
```

未来可增加：

```text
crates/armillae/          # 稳定后提供 facade 和常用 re-export
crates/armillae-turn/     # 自动或显式驱动一次完整 Turn
crates/armillae-embedding/        # Provider 无关的 Embedding Bridge
crates/armillae-embedding-rig/    # rig Embedding Adapter（若经 Spike 验证后采用）
crates/armillae-vector-store/     # 数据库无关的向量存储与检索接口
crates/armillae-vector-qdrant/    # Qdrant Adapter 示例
crates/armillae-vector-pgvector/  # pgvector Adapter 示例
crates/armillae-rag/              # 组合检索、重排、上下文组装与 LLM 调用
```

### 5.1 依赖方向

```text
                   armillae-core
                    ▲         ▲
                    │         │
              armillae-llm  armillae-tools
                    ▲
                    │
             armillae-llm-rig
```

约束：

- `armillae-core` 不依赖异步运行时、HTTP Client 或 LLM SDK。
- `armillae-llm` 与 `armillae-tools` 不互相依赖。
- 第一阶段只有 `armillae-llm-rig` 依赖 `rig-core`。
- Provider 专用类型不能出现在其他 crate 的公共 API 中。

### 5.2 预期依赖

第一阶段应保持依赖最小化：

| crate | 主要依赖 |
|---|---|
| `armillae-core` | `serde`、`serde_json`、`schemars`、`thiserror` |
| `armillae-llm` | `armillae-core`、`futures-core`/`futures-util`、`serde`、`serde_json`、`toml`、`url`、`secrecy` |
| `armillae-tools` | `armillae-core`、`futures-util`、`schemars`、`serde`、`serde_json`、`thiserror` |
| `armillae-llm-rig` | `armillae-core`、`armillae-llm`、`rig-core`、`tokio` |

公共 Bridge 和 Tool 接口只暴露标准 `Future`/`Stream` 语义，不把 Tokio 类型放入协议层。首个 rig Adapter 可以使用 Tokio 作为执行环境。rig 依赖以 Spike 验证过的精确版本锁定；本设计调研基线为 `rig-core = 0.41.0`。

Workspace 初始化阶段只添加上述 crate 之间的本地 path 依赖。外部依赖在对应实现开始且实际
需要时通过 Cargo CLI 引入，避免空 crate 提前携带未使用依赖；这不改变本节记录的第一阶段
预期依赖与版本约束。

### 5.3 版本与发布

Workspace 使用 Semifold 管理 changeset、crate 版本和发布流程，配置保存在
`.changes/config.toml`。初始化工具基线为 Semifold 0.3.0；0.2.5 不能解析本 workspace 使用的
Cargo `version.workspace = true`，不得用于维护当前配置。

第一阶段的发布约定为：

- 使用 Rust workspace resolver 发现四个 crate；
- base branch 与 release branch 均为 `main`；
- 四个 crate 均使用 `alpha` 发布通道，首次进入通道时保留当前稳定版本基准，不额外执行
  patch、minor 或 major 提升；
- 使用 Semifold 默认 changelog 标签；
- 初始化阶段不生成 GitHub Actions，CI 发布流程在得到单独设计和授权后再增加；
- 配置发布通道不等于授权执行版本提升或向 registry 发布。

## 6. `armillae-core`：共享协议

公共协议类型默认派生 `Clone`、`Debug`、`Serialize` 和 `Deserialize`。预期继续增加变体的公共枚举应标记 `#[non_exhaustive]`，避免新增 Provider 能力时迫使下游进行同步升级。配置通过显式 `api_version` 管理文件格式演进。

公共 JSON 协议使用稳定、可读的 wire format：Role 和 `FinishReason` 使用 `snake_case`
字符串；包含数据的内容枚举使用 `type` 字段判别的对象，例如
`{ "type": "text", "text": "..." }`。`FinishReason` 的未知字符串必须反序列化为
`Unknown(String)`，再序列化时恢复原值。类型名、Rust 默认的 externally tagged enum 表示和
调试字符串都不属于 wire contract。所有公共协议根类型生成 JSON Schema，并通过提交到仓库
的快照约束意外变化。

### 6.1 Message

消息必须能够无损表达 Tool Calling 历史。第一阶段的最小模型如下：

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentPart>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub enum Role {
    System,
    Developer,
    User,
    Assistant,
    Tool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ContentPart {
    Text(TextContent),
    ToolCall(ToolCall),
    ToolResult(ToolResult),
    ProviderData(ProviderData),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextContent {
    pub text: String,
}
```

约束：

- `ToolCall` 必须保留 Provider 返回的调用 ID。
- 单个 Assistant Message 可以包含文本和多个 ToolCall。
- `ContentPart` 的顺序必须保持不变。
- Provider 不支持某种 Role 时，Adapter 必须按明确策略转换或报错，不能静默丢弃。
- 多模态内容暂不进入 MVP；后续可以为 `ContentPart` 增加 Image、Audio、Document 等变体。

### 6.2 Tool 协议

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub content: Vec<ToolResultContent>,
    pub is_error: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ToolResultContent {
    Text { text: String },
    Json { value: serde_json::Value },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ToolChoice {
    Auto,
    None,
    Required,
    Specific { name: String },
}
```

`ToolCall.arguments` 在非流式响应和流式完成事件中必须是完整 JSON 值。只有流式增量事件允许暂时携带未完成的 JSON 字符串片段。

### 6.3 Completion Request

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletionRequest {
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDefinition>,
    pub tool_choice: Option<ToolChoice>,
    pub output_format: Option<OutputFormat>,
    pub generation: GenerationOptions,
    pub extensions: ProviderExtensions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputFormat {
    Text,
    JsonObject,
    JsonSchema {
        name: String,
        schema: serde_json::Value,
        strict: bool,
    },
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GenerationOptions {
    pub temperature: Option<f64>,
    pub max_output_tokens: Option<u64>,
    pub stop: Vec<String>,
    pub seed: Option<u64>,
}
```

第一阶段仅标准化有明确跨 Provider 语义的参数。特有参数通过 `ProviderExtensions` 传递：

```rust
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProviderExtensions {
    pub values: std::collections::BTreeMap<String, serde_json::Value>,
}
```

扩展键以 Provider 或 Adapter 命名空间隔离，例如 `openai.reasoning_effort`。Adapter 只读取自己的命名空间；未知扩展默认报错，是否允许忽略必须由显式兼容选项控制。

### 6.4 Completion Response

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompletionResponse {
    pub id: Option<String>,
    pub model: Option<String>,
    pub content: Vec<AssistantContent>,
    pub finish_reason: FinishReason,
    pub usage: Option<TokenUsage>,
    pub provider_metadata: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum AssistantContent {
    Text(TextContent),
    ToolCall(ToolCall),
    ProviderData(ProviderData),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum FinishReason {
    Stop,
    Length,
    ToolCall,
    ContentFilter,
    Cancelled,
    Unknown(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
}
```

响应不能退化为 `text + tool_calls` 两个字段，因为不同内容可能交错出现，且未来可能加入更多 Assistant 内容类型。

### 6.5 ProviderData

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderData {
    pub provider: String,
    pub kind: String,
    pub value: serde_json::Value,
}
```

它是兼容新 Provider 内容的逃生舱，主要用于：

- 暂未标准化的响应项；
- Provider 原生托管工具事件；
- 需要透传但不参与通用逻辑的数据。

ProviderData 不能用于绕过已经存在的标准字段。

## 7. `armillae-llm`：LLM Bridge 与一次模型调用

### 7.1 Bridge 接口

```rust
pub type BoxFuture<'a, T> =
    Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub type CompletionStream =
    Pin<Box<dyn Stream<Item = Result<CompletionEvent, BridgeError>> + Send>>;

pub trait LlmBridge: Send + Sync {
    fn capabilities(&self) -> BridgeCapabilities;

    fn complete<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionResponse, BridgeError>>;

    fn stream<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionStream, BridgeError>>;
}
```

选择 object-safe 接口的原因是 Bridge 需要通过结构化配置在运行时创建，并以 `Arc<dyn LlmBridge>` 被下游持有。

取消语义遵循 Rust Future/Stream 的生命周期：调用方 drop Future 或 Stream 即表示取消。Adapter 必须确保底层 HTTP 请求随之终止或尽快释放。传输和请求超时由 Bridge 配置控制。

### 7.2 能力模型

```rust
#[derive(Clone, Debug)]
pub struct BridgeCapabilities {
    pub streaming: bool,
    pub tool_calling: bool,
    pub parallel_tool_calls: bool,
    pub tool_choice: ToolChoiceCapabilities,
    pub output_format: OutputFormatCapabilities,
    pub system_message: bool,
    pub developer_message: bool,
}

#[derive(Clone, Debug)]
pub struct ToolChoiceCapabilities {
    pub auto: bool,
    pub none: bool,
    pub required: bool,
    pub specific: bool,
}

#[derive(Clone, Debug)]
pub struct OutputFormatCapabilities {
    pub json_object: bool,
    pub json_schema: bool,
}
```

Bridge 在发送请求前必须验证能力：

- 请求包含 Tool，但模型不支持 Tool Calling：返回 `UnsupportedCapability`。
- 请求要求 Specific Tool，而 Provider 不支持：返回错误，不自动降级。
- Provider 不支持 Developer role：按配置的兼容策略转换，或明确拒绝。
- 不支持 Streaming：`stream` 直接返回能力错误，而不是伪造流。

`tool_choice` 存在时请求必须同时提供至少一个 Tool Definition；`Specific { name }` 指定的
名称必须出现在本次请求的 Tool Definition 中，否则属于 `InvalidRequest`，不能交给 Provider
猜测或修正。

Text 是所有 Bridge 的基础输出能力。`ToolChoiceCapabilities` 分别表达四种 ToolChoice，
`OutputFormatCapabilities` 分别表达 JSON Object 与 JSON Schema，不能用一个总开关掩盖
Provider 的部分支持。`tool_calling = false` 时所有 ToolChoice 能力必须为 false；请求使用的
具体变体不受支持时必须在本地拒绝。

第一阶段的能力信息来自 Provider 类型、模型能力表和 Adapter 验证结果，不在可序列化
`BridgeConfig` 中提供能力覆盖。Adapter 不得声称底层实际不具备的能力；宿主若需要主动关闭
已支持能力，可以在 Bridge 外层实施策略。只有出现稳定的跨宿主需求后，才设计纯收紧的能力
限制配置。

### 7.3 流式事件

Bridge 对外发送语义事件，不暴露 Provider SSE/NDJSON chunk：

```rust
#[derive(Clone, Debug)]
pub enum CompletionEvent {
    ResponseStarted {
        id: Option<String>,
        model: Option<String>,
    },
    ContentStarted {
        index: usize,
        kind: ContentKind,
    },
    TextDelta {
        index: usize,
        text: String,
    },
    ToolCallStarted {
        index: usize,
        id: String,
        name: Option<String>,
    },
    ToolCallArgumentsDelta {
        index: usize,
        fragment: String,
    },
    ToolCallCompleted {
        index: usize,
        call: ToolCall,
    },
    ContentCompleted {
        index: usize,
    },
    Usage {
        usage: TokenUsage,
    },
    ProviderEvent {
        data: ProviderData,
    },
    ResponseCompleted {
        response: CompletionResponse,
    },
}
```

```rust
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum ContentKind {
    Text,
    ToolCall,
    ProviderData,
}
```

流式协议必须满足：

- `index` 在同一响应内稳定标识内容块。
- Tool 参数可以跨任意网络 chunk 分割。
- `ToolCallArgumentsDelta` 保留原始字符串片段。
- 只有成功组装并解析完整 JSON 后才产生 `ToolCallCompleted`。
- 成功流必须以唯一的 `ResponseCompleted` 结束。
- `ResponseCompleted.response` 与等价非流式请求的语义结构一致。
- 流在完成事件前失败时返回 `StreamInterrupted`，不得构造虚假的完整响应。
- 未识别 Provider 事件通过 `ProviderEvent` 暴露，不能静默丢弃。

### 7.4 错误模型

```rust
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("invalid bridge configuration: {message}")]
    InvalidConfiguration { message: String },

    #[error("unsupported capability: {capability}")]
    UnsupportedCapability { capability: String },

    #[error("invalid request: {message}")]
    InvalidRequest { message: String },

    #[error("authentication failed")]
    Authentication { metadata: ErrorMetadata },

    #[error("permission denied")]
    PermissionDenied { metadata: ErrorMetadata },

    #[error("rate limited")]
    RateLimited {
        retry_after: Option<Duration>,
        metadata: ErrorMetadata,
    },

    #[error("request timed out")]
    Timeout { metadata: ErrorMetadata },

    #[error("request cancelled")]
    Cancelled,

    #[error("transport error")]
    Transport {
        retryable: bool,
        metadata: ErrorMetadata,
    },

    #[error("provider rejected request: {message}")]
    ProviderRejected {
        code: Option<String>,
        message: String,
        metadata: ErrorMetadata,
    },

    #[error("invalid provider response: {message}")]
    InvalidProviderResponse {
        message: String,
        metadata: ErrorMetadata,
    },

    #[error("stream interrupted")]
    StreamInterrupted { metadata: ErrorMetadata },
}
```

`ErrorMetadata` 只保存跨 Provider 可安全判断的事实：

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorMetadata {
    pub provider: String,
    pub http_status: Option<u16>,
    pub request_id: Option<String>,
}
```

Provider 原始响应、header 和正文不得放入该结构。错误日志和 Display 不得包含 API Key、
Authorization header 或完整敏感响应。

Bridge 只提供 `retryable`、`retry_after` 等事实，不决定是否重新执行推理请求。自动重试策略属于未来 Turn 或下游调度器。

### 7.5 配置

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BridgeConfig {
    pub api_version: String,
    pub driver: String,
    pub provider: String,
    pub model: String,
    pub endpoint: Option<Url>,
    pub credential: Option<CredentialRef>,
    pub transport: TransportConfig,
    pub defaults: GenerationOptions,
    pub provider_options: serde_json::Value,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TransportConfig {
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialRef {
    Environment { name: String },
    File { path: PathBuf },
    Resolver { key: String },
}
```

`TransportConfig` 只控制网络传输，不承载模型生成参数或重试策略。第一阶段默认连接超时为
5 秒、请求超时为 60 秒，两者必须大于零；不设置跨 Provider 的任意最大值。自动重试不属于
Transport，仍由下游根据 `BridgeError` 中的事实决定。

`BridgeConfig` 的 `api_version` 必须等于 `armillae.llm/v1alpha1`，`driver`、`provider` 和
`model` 必须非空，`provider_options` 必须是 JSON Object。通用层只验证跨 Provider 的结构；
具体字段、类型和未知字段由 Adapter Factory 在构造阶段严格验证。生成默认值在通用层拒绝
非有限或负数 temperature、零 `max_output_tokens` 和空 stop string；Provider 特有的范围继续
由 Adapter 验证。

Bridge 执行单次请求时，将构造期 `defaults` 与 `CompletionRequest.generation` 合并：单次请求
中 `temperature`、`max_output_tokens` 和 `seed` 的非空值覆盖对应默认值，否则使用默认值；
单次请求的 `stop` 非空时整体覆盖默认 stop，为空时使用默认 stop。第一阶段的
`GenerationOptions.stop: Vec<String>` 不提供“单次请求显式清空非空默认 stop”的第三种状态；
若后续出现该需求，再将其演进为能区分未指定与显式空列表的协议，不在 Adapter 中引入隐式
哨兵值。

示例 TOML：

```toml
api_version = "armillae.llm/v1alpha1"
driver = "rig"
provider = "openai"
model = "example-model"
endpoint = "https://api.openai.com/v1"

[credential]
type = "environment"
name = "OPENAI_API_KEY"

[transport]
connect_timeout_ms = 5000
request_timeout_ms = 60000

[defaults]
temperature = 0.7
max_output_tokens = 2048

[provider_options]
reasoning_effort = "medium"
```

文件解析和运行时 Builder 最终生成同一个 `BridgeConfig`。配置生命周期为：

```text
TOML / JSON / Rust Builder
           │
           ▼
      BridgeConfig
           │ SecretResolver + 默认值
           ▼
  ResolvedBridgeConfig
           │ 校验 + Adapter Factory
           ▼
   Arc<dyn LlmBridge>
```

Secret 解析使用不绑定异步运行时的 object-safe 接口：

```rust
pub trait SecretResolver: Send + Sync {
    fn resolve<'a>(
        &'a self,
        key: &'a str,
    ) -> BoxFuture<'a, Result<SecretString, BridgeError>>;
}

pub struct ResolvedBridgeConfig {
    config: BridgeConfig,
    credential: Option<SecretString>,
}
```

Environment 和 File 由 `armillae-llm` 在构造阶段解析，Resolver variant 委托宿主提供的
`SecretResolver`。File Secret 必须是 UTF-8；只移除文件末尾的一个 `\n` 或 `\r\n`，不得使用
`trim()` 改变 Secret 的其他空白。解析后的 Secret 不可序列化，`Debug` 必须脱敏；空 Secret
统一视为无效配置。

自定义 endpoint 默认允许，不要求额外授权策略。通用校验要求 URL 使用 HTTP 或 HTTPS、
包含 host 且不携带 userinfo；Adapter 继续验证自身的路径或协议要求。宿主处理不可信或
多租户配置时，可以额外传入 object-safe `EndpointPolicy`，按 scheme、host、解析后的网络
范围或自己的信任规则收紧；没有策略时不会因为 endpoint 是自定义地址而拒绝。

安全约束：

- Secret 值不进入可序列化配置。
- Debug 和 tracing 输出必须脱敏。
- 自定义 endpoint 必须通过通用 URL 校验；默认允许合法地址，宿主可按不可信配置的来源
  选择性限制 scheme、host 或网络范围，避免动态配置导致 SSRF。
- `provider_options` 必须在构造阶段校验类型和已知字段。

### 7.6 Factory

```rust
pub trait BridgeFactory: Send + Sync {
    fn driver(&self) -> &'static str;

    fn create<'a>(
        &'a self,
        config: ResolvedBridgeConfig,
    ) -> BoxFuture<'a, Result<Arc<dyn LlmBridge>, BridgeError>>;
}
```

第一阶段可以直接实例化 `RigBridgeFactory`。如果后续需要插件式 Adapter，再增加按 `driver` 注册的 Factory Registry；本阶段不提前实现动态插件加载。

### 7.7 Mock Bridge

`MockBridge` 是 Bridge 合约的一等实现，用于离线测试下游调度：

```rust
let bridge = MockBridge::scripted([
    MockResponse::tool_call("call-1", "get_weather", json!({ "city": "上海" })),
    MockResponse::text("上海今天有雨。"),
]);
```

Mock 相关实现由 `armillae-llm` 的 `mock` feature 提供，默认构建不携带测试辅助设施。下游
测试和 `armillae-llm-rig` 的 dev-dependency 可以显式启用该 feature；feature 内同时提供可由
Mock 和真实 Adapter 复用的 Bridge 合约测试工具。

共享工具位于 `armillae_llm::mock::contract`，提供运行时无关的异步 `verify_completion`、
`verify_stream`，以及同步 `validate_stream_events`。它们验证期望响应、唯一且位于末尾的
`ResponseCompleted`、内容 index 生命周期、文本增量和 ToolCall 参数重组；失败返回不携带
请求或响应正文的 `BridgeContractError`，由调用方自己的测试运行时驱动。

```rust
pub enum MockResponse {
    Completion(CompletionResponse),
    Stream(Vec<Result<CompletionEvent, BridgeError>>),
    Error(BridgeError),
}
```

`MockBridge::fixed` 在每次调用中重复返回同一个脚本项；`MockBridge::scripted` 按 Future 被
poll 的顺序从一个共享队列消费脚本项。`Completion` 只用于 `complete`，`Stream` 只用于
`stream`，调用类型与脚本项不匹配或队列耗尽时返回 `InvalidRequest`，不得猜测或自动转换。
能力预检失败不消费脚本项。

`MockResponse::text`、`tool_call`、`text_stream` 和 `tool_call_stream` 提供语义完整的便利构造；
流式便利构造必须生成稳定 index，并以唯一 `ResponseCompleted` 结束。原始 `stream` 构造允许
测试显式注入任意事件或 `StreamInterrupted`。Mock 记录所有收到的请求，包括被本地能力预检
拒绝的请求；请求正文和脚本内容不得出现在默认 `Debug` 输出中。

Mock 至少支持：

- 固定非流式响应；
- 按调用顺序返回脚本响应；
- 文本流式增量；
- ToolCall 参数分片；
- 注入 Provider 错误和流中断；
- 记录收到的请求用于断言。

## 8. `armillae-tools`：Tool 与 Executor

### 8.1 类型化 Tool

```rust
pub trait Tool: Send + Sync {
    type Args: DeserializeOwned + JsonSchema + Send;
    type Output: IntoToolOutput + Send;
    type Error: std::error::Error + Send + Sync + 'static;

    const NAME: &'static str;

    fn description(&self) -> Cow<'static, str>;

    fn call(
        &self,
        context: ToolContext,
        args: Self::Args,
    ) -> impl Future<Output = Result<Self::Output, Self::Error>> + Send;
}
```

类型化接口为 Tool 作者提供：

- 编译期参数和结果类型检查；
- 自动参数反序列化；
- 从 `Args` 自动生成 JSON Schema；
- 统一错误映射。

`IntoToolOutput` 负责把 Tool 作者的返回值转换为规范化输出：

```rust
pub trait IntoToolOutput {
    fn into_tool_output(self) -> Result<ToolOutput, ToolExecutionError>;
}
```

所有满足 `Serialize` 的普通返回类型通过 blanket implementation 转换为单个
`ToolResultContent::Json`。`ToolOutput` 本身不实现 `Serialize`，而是直接实现
`IntoToolOutput` 并原样返回，因此两种实现不会重叠，也不依赖 nightly specialization。
Tool 作者默认返回自己的类型；只有需要显式文本、多段内容或其他模型可见内容语义时才直接
返回 `ToolOutput`。

### 8.2 类型擦除 DynTool

由于 `Tool` 包含关联类型，Registry 内部需要 object-safe 接口：

```rust
pub trait DynTool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    fn call_json<'a>(
        &'a self,
        context: ToolContext,
        arguments: serde_json::Value,
    ) -> BoxFuture<'a, Result<ToolOutput, ToolExecutionError>>;
}
```

`DynTool` 的规范化成功结果为：

```rust
#[derive(Clone)]
pub struct ToolOutput {
    pub content: Vec<ToolResultContent>,
}
```

blanket implementation 默认将类型化 `Tool::Output` 序列化为 `ToolResultContent::Json`。需要返回多段文本或定制内容的 Tool 可以显式返回 `ToolOutput`，或直接实现 `DynTool`。

`ToolOutput` 的 `Debug` 实现只能显示内容数量和类型，不得显示文本或 JSON 正文；它不是配置
或 transcript 类型，也不得派生或实现 `Serialize`，以保持普通数据转换与规范化输出之间的
明确边界。

为所有满足约束的 `Tool` 提供 blanket implementation。通常只有需要完全动态 Schema 或远程代理的高级用户才直接实现 `DynTool`。

### 8.3 ToolContext

本阶段的 Context 应保持轻量且可扩展：

```rust
#[derive(Clone, Default)]
pub struct ToolContext {
    extensions: Extensions,
}
```

`Extensions` 是类型安全的运行时 type map，可用于传递：

- session/run 标识；
- 世界状态句柄；
- 权限或身份信息；
- tracing context；
- 下游自定义服务。

Context 中的信息不发送给 LLM，也不要求可序列化。Armillae 不解释这些值的业务含义。

### 8.4 ToolExecutor

```rust
pub trait ToolExecutor: Send + Sync {
    fn definitions(&self) -> Vec<ToolDefinition>;

    fn execute<'a>(
        &'a self,
        context: ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolResult, ToolExecutionError>>;
}
```

第一阶段语义：

- 一次只执行一个 ToolCall。
- Executor 根据 Tool 名称查找实现。
- Executor 解析和验证 JSON 参数。
- Executor 保留 `ToolCall.id`，生成对应的 `ToolResult.call_id`。
- Executor 区分参数错误、未知 Tool 和执行失败。
- Executor 不重试、不再次调用 LLM、不决定错误是否应反馈给 LLM。

### 8.5 ToolRegistry

```rust
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn DynTool>>,
}
```

Registry 是默认的本地 Executor 实现，提供：

- 注册和注销 Tool；
- 名称唯一性校验；
- 生成稳定排序的 Tool Definition；
- 按名称查找和执行；
- ToolCall 到 ToolResult 的关联。

重复注册同名 Tool 默认返回构建错误，不允许静默覆盖。

### 8.6 Tool 错误

```rust
#[derive(Debug, thiserror::Error)]
pub enum ToolExecutionError {
    #[error("unknown tool: {name}")]
    UnknownTool { name: String },

    #[error("invalid arguments for tool {name}: {message}")]
    InvalidArguments { name: String, message: String },

    #[error("tool {name} failed: {message}")]
    ExecutionFailed { name: String, message: String },

    #[error("tool output serialization failed: {message}")]
    OutputSerialization { message: String },
}
```

重复注册属于 Registry 构建或变更失败，不属于一次 Tool 执行失败，使用独立的结构化错误：

```rust
#[derive(Debug, thiserror::Error)]
pub enum ToolRegistryError {
    #[error("duplicate tool: {name}")]
    DuplicateTool { name: String },
}
```

Executor 返回宿主错误还是构造 `ToolResult { is_error: true }` 不应混为一谈：

- `ToolExecutionError` 表示执行 API 失败。
- 是否将该错误转成模型可见的 ToolResult，由下游或未来 Turn 策略决定。

## 9. `armillae-llm-rig`：rig LLM Adapter

### 9.1 使用范围

本阶段使用 rig 的低层能力：

- Provider Client；
- `CompletionModel`；
- Rig Completion Request/Response；
- Rig Message 与 Tool Definition；
- Provider streaming。

本阶段不使用：

- `rig::Agent`；
- `AgentBuilder`；
- `AgentRun`；
- Rig 自动 Tool 执行；
- Rig Memory、RAG 和 Agent Hook。

### 9.2 泛型与类型擦除

rig 的 `CompletionModel` 不是 dyn-compatible，因此 Adapter 内部使用泛型，对外通过 Armillae Bridge 擦除类型：

```rust
pub struct RigBridge<M> {
    model: M,
    capabilities: BridgeCapabilities,
    defaults: GenerationOptions,
    request_mapper: Arc<dyn RigRequestMapper>,
    normalizer: Arc<dyn RigResponseNormalizer<M::Response>>,
}

impl<M> LlmBridge for RigBridge<M>
where
    M: rig_core::completion::CompletionModel + Send + Sync + 'static,
    M::Response: Send + Sync,
    M::StreamingResponse: Send + Sync,
{
    // Armillae 与 Rig 协议转换
}
```

Factory 在运行时匹配 Provider，构造具体的 `RigBridge<M>` 后返回 `Arc<dyn LlmBridge>`。

rig 的通用 `CompletionRequest` 直接承载 Message、Tool、temperature、max tokens、ToolChoice 和
一部分 output schema，但不统一 `stop`、`seed`、JSON Object，以及 JSON Schema 的 name/strict
等 Provider wire 差异。`RigBridge<M>` 因此持有私有、窄化的 `RigRequestMapper`，由对应
Provider Factory 注入。Mapper 只负责合并生成默认值、复用公共协议转换，并将标准请求字段与
本 Provider 的命名空间扩展映射到 rig 请求及其 `additional_params`；它不发送请求，也不处理
响应。

构造期 `provider_options` 是已由 Factory 校验的 Provider 默认选项；请求级扩展只读取当前
Provider/Adapter 命名空间，并可覆盖同名的 Provider 特有选项。未知命名空间、未知字段和错误
类型必须在请求发送前拒绝。`provider_options` 和请求扩展都不得重复设置或覆盖已经由
`GenerationOptions`、`OutputFormat`、`ToolChoice` 等公共字段表达的标准语义。

rig 的通用 `CompletionResponse<T>` 只标准化 choice、usage 和 message ID；实际模型、完整结束
原因及部分安全 metadata 仍位于 Provider-specific `raw_response`。`RigBridge<M>` 因此持有一个
私有、窄化的 `RigResponseNormalizer<M::Response>`，由对应 Provider Factory 注入。Normalizer
只负责从 raw response 提取 Armillae 已定义的响应事实和经过筛选的 metadata，不重新实现
请求发送，也不把完整 raw response 暴露到公共协议。

`RigRequestMapper` 和 `RigResponseNormalizer` 是两个单向边界，不合并为同时负责请求、响应或
传输的宽泛 Provider Codec。实际网络调用始终只由 rig `CompletionModel` 执行。

不得根据“是否出现 ToolCall”等内容猜测 Provider 已明确返回的结束原因。Provider 返回未知
结束值时转换为 `FinishReason::Unknown`；协议允许缺失的 ID/model 保持 `None`，协议要求存在
但实际缺失时返回 `InvalidProviderResponse`。第一阶段 OpenAI Normalizer 读取 OpenAI raw
response；后续 Provider 各自实现同一私有边界。

rig 通用 Message 无法原样发出 Developer role 时，相应 Rig Adapter 必须声明
`developer_message = false` 并在能力预检阶段拒绝，不能静默转换为 System。

`BridgeCapabilities` 表达当前 Adapter 与所选 Provider profile 能够承载的公共协议能力，不把
模型名称当作运行时能力发现机制。第一阶段 OpenAI/OpenAI-compatible Factory 使用固定的
OpenAI Provider 能力预设：支持 Tool Calling、并行 ToolCall、全部已定义 ToolChoice、JSON
Object、JSON Schema 和 System role，不支持 Developer role；P4 的非流式 Adapter 还必须声明
`streaming = false`。选择 `provider = "openai-compatible"` 表示调用方声明自定义 endpoint 符合
该 OpenAI profile，而不是要求 Adapter 根据 endpoint 或模型名称进行探测。

未来若 Adapter 内置某些已知模型的可靠限制，只能在 Provider 预设基础上收紧能力，不能因为
未知模型名称而拒绝构造，也不能静默放宽或降级请求。若远端实际能力与声明的 Provider profile
不一致，Provider 的拒绝必须转换为 `ProviderRejected`；Adapter 不根据失败响应自动改写请求或
切换能力。

第一阶段 OpenAI/OpenAI-compatible Factory 直接使用 rig 的 OpenAI Chat Completions Client。
该 Client 的原生构造契约要求 API Key 并发送 Bearer Authorization Header，因此两种 Provider
配置都必须提供 `credential`；不得使用伪造或空凭证模拟无认证请求。`provider = "openai"` 未
配置 endpoint 时使用 rig 的 OpenAI 默认地址，也可显式提供经过宿主策略校验的自定义 endpoint；
`provider = "openai-compatible"` 必须显式提供自定义 endpoint。完全无认证的兼容端点不属于
该 Factory 的第一阶段契约，后续应通过对应的具名 Provider Adapter 或明确的无认证 Provider
实现支持，而不是在 OpenAI Client 中隐式省略认证。

### 9.3 转换边界

转换代码必须集中在 `convert` 模块，并单独测试：

```text
Armillae Message          ↔ Rig Message
Armillae ToolDefinition   ↔ Rig ToolDefinition
Armillae CompletionRequest → Rig CompletionRequest
Rig CompletionResponse     → Armillae CompletionResponse
Rig Streaming Item         → Armillae CompletionEvent
Rig Error                  → Armillae BridgeError
```

转换必须满足：

- 不丢失 ToolCall ID。
- 不丢失多个 ToolCall 及其顺序。
- Assistant ToolCall 和对应 ToolResult 能在下一次请求中正确关联。
- Provider 原始未知输出转换为 ProviderData。
- Provider 原始响应只进入受控 metadata，不把 Secret 写入日志。
- 不依赖 Rig Agent 的 Tool 注册或执行路径。

`ToolResult.is_error` 是 Armillae 公共协议的一部分，但 Provider 线协议未必有对应字段。Adapter
必须为每个 Provider 显式定义该字段的兼容策略：Provider 原生支持错误标记时进行语义映射；
Provider 不支持时，不得因此拒绝 ToolResult，也不得擅自改写或包装调用方提供的内容。

第一阶段 OpenAI/OpenAI-compatible 的 tool message 不承载独立的 `is_error` 字段，因此转换时
保留 `call_id`、content 及其顺序，但不把 `is_error` 下发到 Provider。原始 Armillae 请求和
调用方维护的消息历史仍保留该字段；调用方在 `is_error = true` 时必须通过 ToolResult content
向模型表达失败事实。Adapter 不自动添加错误前缀或结构，以免改变调用方定义的模型可见内容。
此行为必须由转换测试覆盖，不能作为未记录的字段丢弃。后续 Anthropic 等 Provider 若有原生
错误标记，应映射该标记，而不是沿用 OpenAI 的省略策略。

### 9.4 首批 Provider

建议按以下顺序支持：

1. OpenAI 或 OpenAI-compatible：验证主协议和自定义 endpoint。
2. Anthropic：验证原生 Tool 协议和消息差异。
3. Ollama：验证本地 Provider 和 NDJSON 流式路径。

Provider 支持必须通过同一套 Bridge 合约测试，而不是分别定义不同的外部行为。

## 10. 下游显式 Tool 流程

第一阶段的典型用法如下：

```rust
let bridge = factory.create(config).await?;

let tools = ToolRegistry::builder()
    .register(GetWeatherTool::new(weather_client))?
    .register(RollDiceTool)?
    .build();

let first = bridge
    .complete(CompletionRequest {
        messages: vec![Message::user("上海今天天气怎么样？")],
        tools: tools.definitions(),
        ..Default::default()
    })
    .await?;

let mut history = vec![Message::user("上海今天天气怎么样？")];
history.push(first.as_assistant_message());

for call in first.tool_calls() {
    let result = tools
        .execute(context.clone(), call.clone())
        .await?;
    history.push(Message::tool_result(result));
}

let final_response = bridge
    .complete(CompletionRequest {
        messages: history,
        tools: tools.definitions(),
        ..Default::default()
    })
    .await?;
```

上述循环由下游显式编写。未来 `armillae-turn` 可以封装相同过程，但不改变 Bridge 和 Executor 的职责。

## 11. 测试策略

### 11.1 协议测试

`armillae-core` 必须覆盖：

- 所有公共类型的 Serde round-trip；
- Assistant 文本与 ToolCall 混合内容；
- 多 ToolCall 顺序保持；
- ToolCall/ToolResult ID 关联；
- 未知 finish reason 和 ProviderData 的前向兼容；
- JSON Schema 的合法性和稳定快照。

### 11.2 Tool Executor 测试

使用确定性的本地 Tool 覆盖：

- 自动生成 Tool Definition；
- 正确参数执行；
- 缺少字段、错误类型和非法 JSON 值；
- 未知 Tool；
- Tool 自身错误；
- Output 序列化；
- 重复注册；
- ToolContext 扩展透传；
- call ID 保持不变。

### 11.3 Bridge 合约测试

定义可被 Mock 和每个真实 Adapter 复用的测试套件：

- 纯文本请求和响应；
- 系统消息和多轮历史；
- Tool Definition 输入；
- 单 ToolCall；
- 多 ToolCall；
- Assistant ToolCall + ToolResult 的后续请求；
- Usage 与 finish reason；
- Provider 拒绝、认证失败、限流和超时映射；
- 不支持能力的本地预检。

### 11.4 Streaming 合约测试

- 文本分成任意数量 chunk 后重组一致；
- Tool 名称和参数跨 chunk 分割；
- UTF-8 字符跨底层字节 chunk；
- 多 ToolCall 交错增量；
- 完成时完整 ToolCall JSON 可解析；
- 流中断不产生 ResponseCompleted；
- Usage 出现在最终或独立事件时均正确汇总；
- 未知 Provider 事件不会丢失；
- drop Stream 后底层调用被取消。

### 11.5 Provider 测试分层

- 转换单元测试：完全离线，不访问 Provider。
- Mock HTTP/cassette 测试：验证请求和响应协议。
- Live 测试：使用真实凭证，默认 ignored，仅用于发布前验证。

测试夹具不得包含 API Key、Authorization header、用户隐私内容或未经脱敏的 Provider 响应。

## 12. 可观测性与安全

第一阶段即应提供结构化 tracing，至少包含：

- Adapter 和 Provider 名称；
- 模型名称；
- 请求 ID；
- 是否流式；
- Tool Definition 数量与返回 ToolCall 数量；
- 输入、输出和缓存 token；
- 总延迟与首 token 延迟；
- 标准化错误类别。

默认不得记录：

- API Key 或认证 header；
- 完整消息正文；
- 完整 Tool 参数和 ToolResult；
- Provider 原始响应正文。

需要内容级调试时必须通过显式配置启用，并允许宿主提供脱敏器。

## 13. 实施计划与优先级

### P0：rig 低层可行性 Spike

在正式冻结 API 前验证：

- 不使用 `rig::Agent`，只用 `CompletionModel`；
- 可以发送 Tool Definition 并接收单个/多个 ToolCall；
- 可以手工将 ToolResult 放回消息历史；
- 流式 ToolCall 参数能够无损重组；
- OpenAI 和 Anthropic 的差异可以被 Adapter 层消化。

Spike 代码可以是临时实验，不作为公共 API。若低层 API 无法满足上述要求，应在此阶段评估 `genai` 或原生 Provider Adapter。

#### P0 结论（2026-08-15）

Spike 使用精确版本 `rig-core = 0.41.0`，结论为通过。离线测试确认可以只调用
`CompletionModel` 完成非流式和流式请求，无需引入 Rig Agent runtime；Tool Definition、单个及
多个 ToolCall、Assistant ToolCall 与 ToolResult 历史、OpenAI 与 Anthropic 原生消息差异均可
在 Adapter 边界处理。OpenAI 流式路径能够跨任意 HTTP 字节与 UTF-8 边界重组交错的多个
ToolCall，并保留调用 ID、稳定的内部关联 ID、输出顺序和 Usage。

因此第一阶段继续采用 `rig-core 0.41.0` 实现 `armillae-llm-rig`，暂不转向 `genai` 或原生
Provider Adapter。Armillae 仍拥有公共协议；Spike 中使用的 Rig 类型、原始响应和内部关联 ID
都不得穿透 Adapter。依赖继续精确锁定，任何版本升级都必须先复跑转换与 Bridge 合约测试。

已知边界是：Rig 只保证客户端 Future/Stream 被 drop 后释放或取消其内部资源，无法保证远端
Provider 已经停止计算；OpenAI 的后续 ToolCall delta 可能不重复外部调用 ID，需要按稳定的
内部关联 ID 分组；Anthropic 使用 `tool_use`/`tool_result` content block，且请求要求
`max_tokens`。这些差异由后续 Armillae 协议、取消语义和 Provider Adapter 显式吸收，不改变
Bridge 一次只执行一个 Model Call 的边界。完整测试证据和限制记录见
[rig-core 0.41.0 P0 Spike](spikes/rig-core-0.41.0.md)。

### P1：`armillae-core`

- 建立 Workspace。
- 完成 Message、Completion、Tool、Streaming、Usage 协议。
- 完成序列化、验证和协议单元测试。

### P2：`armillae-tools`

- 实现 `Tool`、`DynTool` 和 blanket implementation。
- 实现 `ToolContext`、`ToolExecutor` 和 `ToolRegistry`。
- 完成参数、Schema、执行和错误合约测试。

### P3：`armillae-llm` 与 Mock

- 实现 `LlmBridge`、能力和错误模型。
- 实现配置解析、SecretResolver 和 Factory 接口。
- 实现 MockBridge 和 Bridge 合约测试框架。

### P4：rig 非流式 Adapter

- 首先支持 OpenAI/OpenAI-compatible。
- 完成 Message、Tool 和响应转换。
- 验证显式 `LLM -> ToolCall -> ToolResult -> LLM` 闭环。

### P5：Streaming

- 实现文本和 ToolCall 语义事件。
- 完成参数重组、中断和取消测试。

### P6：更多 Provider

- Anthropic。
- Ollama。
- 完成统一合约测试和能力矩阵。

## 14. 第一阶段验收标准

第一阶段完成需要满足：

1. 同一 `CompletionRequest`/`CompletionResponse` 协议可用于所有已支持 Provider。
2. 配置可从 TOML/JSON 和 Rust Builder 生成同一个 Bridge 实例。
3. 非流式和流式文本响应可用。
4. Tool Definition 可以发送给模型。
5. 单个和多个 ToolCall 可被完整解析。
6. ToolCall 参数在流式分片下无损重组。
7. ToolResult 可以作为后续请求消息发送给模型。
8. 下游可以通过 ToolRegistry 执行 ToolCall。
9. Bridge 不执行 Tool，Tool Executor 不调用 Bridge。
10. Usage、finish reason、请求 ID 和错误类别被标准化。
11. MockBridge 和所有真实 Adapter 通过共享合约测试。
12. 除 `armillae-llm-rig` 外没有 crate 依赖或暴露 rig 类型。

## 15. 风险与应对

### 15.1 rig API 变化

风险：rig 处于 0.x，升级可能修改消息、Tool 或流式类型。

应对：固定精确依赖版本；所有转换集中在独立 Adapter；通过合约测试驱动升级；禁止 rig 类型穿透公共 API。

### 15.2 Provider 语义不完全一致

风险：Role、ToolChoice、JSON Schema、finish reason 和流式事件在 Provider 间存在差异。

应对：能力预检；明确的兼容/降级策略；保留 ProviderData 和扩展字段；禁止静默丢失。

### 15.3 流式 ToolCall 重组错误

风险：网络 chunk 不等于语义事件边界，Tool JSON 和 UTF-8 都可能跨 chunk。

应对：Adapter 按 Provider 协议增量解析；按 call/index 维护独立缓冲；完成后再解析 JSON；使用随机分片和属性测试。

### 15.4 抽象过度

风险：在缺少真实 Provider 反馈前设计过多未来能力。

应对：第一阶段只支持文本与 Tool Calling；不实现 Turn、Agent、Memory、Embedding、Vector
Store、RAG 和调度策略；多模态和插件机制在有明确需求后扩展。

### 15.5 Secret 和敏感内容泄漏

风险：Provider Client、Debug、错误或测试 fixture 可能包含认证和用户内容。

应对：SecretRef 与已解析 Secret 分离；自定义脱敏 Debug；默认不记录正文；fixture 提交前扫描敏感字段。

## 16. 后续演进

LLM Bridge 和 Tool Executor 稳定后，可以在不修改其核心协议的前提下新增：

- `armillae-turn`：自动 Driver 与可逐步推进的 Turn 状态机；
- 多 ToolCall 的串行、并行或 Executor-defined 调度；
- 人工审批、权限与副作用策略；
- MCP 或远程 ToolExecutor；
- 录制与回放 Executor；
- 多模态 Message Content；
- Provider 路由、回退和负载均衡；
- Conversation Memory 与叙事上下文；
- `armillae-embedding`：以 `EmbeddingBridge` 封装一次或批量 Embedding 调用；
- `armillae-vector-store`：以 `VectorStore` 封装数据库无关的向量写入、过滤和检索；
- `armillae-rag`：组合 LLM、Embedding、Vector Store、可选重排和上下文组装；
- 更高层 Agent 和世界运行时。

未来 Turn 的组合关系应保持为：

```text
armillae-turn
    ├── LlmBridge
    └── ToolExecutor
```

而不是让 Bridge 依赖 Turn 或让 Tool Executor 持有 Bridge。

未来 RAG 的组合关系应保持为：

```text
armillae-rag
    ├── armillae-llm          → LlmBridge
    ├── armillae-embedding    → EmbeddingBridge
    └── armillae-vector-store → VectorStore
```

具体模型或数据库集成分别放在能力对应的 Adapter crate 中，例如 `armillae-embedding-rig`、
`armillae-vector-qdrant` 和 `armillae-vector-pgvector`。RAG 是这些能力的上层编排，不作为
数据库抽象，也不反向进入任一底层 Bridge。

## 17. 总结

第一阶段的架构边界是：

> LLM Bridge 负责一次 Provider 无关的模型调用和 Tool Calling 协议传输；Tool Executor 负责一次 ToolCall 的类型安全执行；是否继续调用模型由下游显式决定。

这一边界既能满足当前 ToolCall 与内容输出需求，也为未来一次完整 Turn 的自动驱动留下稳定组合点。rig-rs 被用于降低 Provider 接入成本，但被严格隔离为可替换 Adapter，Armillae 自己掌握公共协议和长期兼容性。

第一阶段的 LLM Bridge 由 `armillae-llm` 提供；未来的 Embedding Bridge、Vector Store 和 RAG
分别由独立 crate 提供，避免把不同请求、错误和生命周期语义压入一个通用 Bridge。

## 18. 调研参考

- [rig-core crate 文档](https://docs.rs/rig-core/latest/rig_core/)
- [Rig CompletionModel](https://docs.rs/rig-core/latest/rig_core/completion/request/trait.CompletionModel.html)
- [Rig Completion 协议](https://docs.rs/rig-core/latest/rig_core/completion/index.html)
- [Rig Streaming 协议](https://docs.rs/rig-core/latest/rig_core/streaming/index.html)
- [Rig GitHub 仓库](https://github.com/0xPlaygrounds/rig)

外部依赖的版本、能力和行为以实现 Spike 及锁定版本的源码为准，不能仅依赖本文链接所指向的 latest 文档。
