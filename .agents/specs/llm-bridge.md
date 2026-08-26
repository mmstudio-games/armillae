# Armillae LLM Bridge、Router 与 Tool Executor 规范

> 状态：Active Spec；直接 Bridge canonical 投影离线实现已完成；fallback Router 待实现
> 规范基线：2026-08-27
> 适用范围：`armillae-core`、`armillae-llm`、`armillae-tools`、`armillae-llm-rig`
> 设计入口：[Armillae 设计索引](../DESIGN.md)
> 路由决策：[RFC 0003：LLM canonical 投影与模型 fallback](../rfcs/0003-llm-projection-fallback.md)
> 后续方向：[RFC 0001：Agentic 叙事运行时](../rfcs/0001-agentic-runtime.md)

本文保留第一阶段设计基线、协议决策和验收证据，并按 2026-08-27 接受的 RFC 0003 扩展
canonical history、Provider 双向投影、兼容性事实和模型 fallback 边界。该扩展不引入新的
Driver、全局 Provider Registry、配置 Loader、自动 Tool Loop 或 Agentic 运行时职责。

## 1. 背景

Armillae 的长期目标是成为一个面向 Agentic 叙事的通用运行时，可被上层用于构建叙事引擎、TRPG 运行时以及大世界游戏引擎。长期系统将涉及上下文组织、叙事状态、世界状态、工具调度、持久化、回放以及更高层的 Agent 行为，但这些能力不属于第一阶段的实现范围。

本文聚焦三个基础设施能力：

1. **LLM Bridge**：通过统一协议连接不同 LLM Provider，支持普通内容、流式内容以及完整 Tool Calling 协议。
2. **Tool Executor**：让下游以类型安全的方式实现 Tool，并可根据 LLM 返回的 `ToolCall` 显式执行 Tool。
3. **LLM Router**：组合宿主构造的多个 Bridge Candidate，为一次逻辑 LLM 请求执行能力协商、
   Provider projection 和显式策略控制的 model fallback。

本文覆盖的子系统不实现完整 Agent，也不实现自动的多轮 Tool Continuation Loop。下游可以用
Bridge 和 Tool Executor 自行组织如下流程：

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

Agentic 叙事运行时可以在这些能力之上组织 Turn 或其它执行模型，但运行时设计不得反向改变本
文冻结的一次 Model Call 与一次 Tool Execution 边界。运行时的具体模型由
[RFC 0001：Agentic 叙事运行时](../rfcs/0001-agentic-runtime.md) 单独管理。

Router 可以在一次逻辑请求中执行多个 Candidate attempt，但每个 attempt 最多调用一个 Bridge
一次。它不执行 Tool，也不把 fallback 等同于 Turn、Conversation Memory 或 Agent 调度。

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

### 2.8 Canonical History 与 Provider Projection

Canonical History 是调用方持有的 Armillae Message 序列。Provider Projection 是 Adapter 为
一个确定目标生成 wire request 的派生过程，不修改 canonical 数据。

### 2.9 Model Candidate 与 Attempt

Candidate 是宿主提供的一个具名 Provider/model Bridge；Attempt 是 Router 对该 Candidate 的
预检、投影和至多一次 Bridge 调用。

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
- 保持 Armillae request、response 和 history 为 canonical 数据，并为已知 Provider 私有内容
  建立响应反序列化与同 Provider 请求回放闭环。
- 支持宿主提供多个运行时 Candidate，由 Router 在结构化兼容性和错误事实基础上 fallback。
- 提供 Mock、合约测试和协议转换测试，保证未来更换 Adapter 时上层行为不变。

### 3.2 非目标

第一阶段明确不实现：

- 自动 Tool Loop 或 Turn Runner；
- 完整 Agent、规划器或工作流编排；
- 跨 Turn 的 Conversation Memory；
- Embedding、RAG、向量数据库或上下文检索；
- Tool 批量调度、并发策略、无界或隐式重试、人工审批；
- 世界状态、叙事状态或游戏事务；
- 长期 transcript 持久化和存档；
- 由 Bridge 自动执行 Tool；
- 由 Tool Executor 自动再次调用 LLM。
- 自动发现 Provider/model、全局 Provider Registry、配置 Loader 或动态插件加载。
- 在已经输出流式语义事件后切换 Provider 并拼接另一条响应流。

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

### 4.7 Canonical 数据与 Router 分层

Armillae canonical request/history 始终由调用方拥有。Adapter 只为当前 Provider 派生 wire
projection，并把 Provider response 解码回 canonical response；投影不得原地删除或覆盖历史。
同 Provider 已知 replay data 必须双向转换，跨 Provider 私有数据继续保留在 canonical history
中但不发送给无关目标，并产生结构化兼容性事实。

`LlmBridge` 继续表示一次 Provider Model Call。`LlmRouter` 是 `armillae-llm` 中独立的组合层，
接收宿主已经构造的 Bridge Candidate，可以按策略执行多个 attempt，但不实现
`LlmBridge`、不执行 Tool、不维护 Memory，也不拥有 Turn。具体决策由
[RFC 0003](../rfcs/0003-llm-projection-fallback.md) 约束。

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
│   │       ├── projection.rs
│   │       ├── config.rs
│   │       ├── factory.rs
│   │       ├── router.rs
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
└── crates/armillae-llm-rig/examples/
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
| `armillae-llm-rig` | `armillae-core`、`armillae-llm`、`rig-core`、`futures-util`、`tokio` |

公共 Bridge 和 Tool 接口只暴露标准 `Future`/`Stream` 语义，不把 Tokio 类型放入协议层。首个 rig Adapter 可以使用 Tokio 作为执行环境。rig 依赖以 Spike 验证过的精确版本锁定；本设计调研基线为 `rig-core = 0.41.0`。

Workspace 初始化阶段只添加上述 crate 之间的本地 path 依赖。外部依赖在对应实现开始且实际
需要时通过 Cargo CLI 引入，避免空 crate 提前携带未使用依赖；这不改变本节记录的第一阶段
预期依赖与版本约束。

### 5.3 版本与发布

Workspace 使用 Semifold 管理 changeset、crate 版本和发布流程，配置保存在
`.changes/config.toml`。初始化工具基线为 Semifold 0.3.0。四个 crate 分别在自己的 manifest
中声明精确 package version，不从 `[workspace.package]` 继承版本；Semifold 按 package 独立
计算和写入版本，并同步更新依赖方 manifest 中的内部 crate 版本要求。共享的 edition、license、
repository、homepage 和 readme 等非版本元数据继续从 workspace 继承。

第一阶段的发布约定为：

- 使用 Rust workspace resolver 发现四个 crate；
- 四个 crate 独立锁定和演进 package version；根 manifest 不定义统一 workspace version，
  不因任一 crate 发布而无条件提升其它 crate；
- 项目源代码和四个 crate 统一采用 SPDX 标识 `AGPL-3.0-only`，仓库根目录保存完整
  `LICENSE` 正文；不得将其表述为“AGPL-3.0-or-later”；
- 仓库根目录提供英文 `README.md`、中文 `README.zh.md` 和 `CONTRIBUTING.md`。发布到
  registry 的 crate 必须提供准确的 description、license、repository、homepage、
  documentation、readme、keywords 和 categories 等发现与合规元数据；crate README 可以
  复用根 README，但打包后必须能够解析和展示；
- 每个 crate 在进入发布流程前分别执行 `cargo publish --dry-run -p <package>`，检查 manifest
  元数据、包内容、workspace path 依赖和构建结果；首次发布必须按内部依赖拓扑进行：在上游
  crate 尚不存在于 registry 时，下游 dry-run 可以先完成打包与元数据检查，但完整依赖解析只能
  在上游实际发布后重跑。不得为了让 dry-run 通过而在缺少明确授权时实际发布；dry-run 本身也
  不构成实际发布授权；
- base branch 为 `main`，release branch 为独立的 `release`；两者不得相同，避免 Semifold 将
  版本提交直接强推到 base branch，或尝试创建源分支与目标分支相同的 release PR；
- 四个 crate 在 workspace 初始化时以 `0.1.0-alpha.0` 建立集成基线；第一阶段离线实现完成后
  统一恢复 Semifold 默认稳定通道，下一次版本计划直接移除 prerelease 后缀并进入 `0.1.0`，
  不额外经过 beta 或 rc 通道；
- 使用 Semifold 默认 changelog 标签；
- GitHub Actions 使用手工渲染自 Semifold 0.3.0 内置 Jinja 模板的
  `semifold-ci.yaml` 和 `semifold-status.yaml`，模板变量固定为 base branch `main`、Rust
  resolver；Semifold step ID、job output、权限和 registry token 契约保持与上游模板一致；
- 配置发布通道不等于授权执行版本提升或向 registry 发布。

crate 的稳定发布通道只表达 SemVer 发布策略，不改变 `armillae.llm/v1alpha1` 配置协议版本，
也不把尚未执行的 Live Provider 矩阵视为已通过。首个 `0.1.0` 可以继续明确标注“Live 未验证”，
但在真实矩阵留下脱敏证据前不得宣称全量支持冻结的 OpenAI 协议 Provider/模型组合。

仓库同时提供一个可复用的 Rust CI workflow。Pull Request 直接调用该 workflow；`main` 的
Semifold CD workflow 必须先复用并通过同一质量门禁，再运行 `semifold ci`，避免未验证提交进入
版本提升或发布路径。质量门禁固定执行：

- `cargo fmt --all -- --check`；
- `cargo check --workspace --all-targets --all-features --locked`；
- `cargo test --workspace --all-targets --all-features --locked`；
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`；
- 在 `RUSTDOCFLAGS=-D warnings` 下执行
  `cargo doc --workspace --all-features --no-deps --locked`。

CI 使用 stable Rust，并安装 `rustfmt` 和 `clippy`；Cargo 构建缓存不得包含 Secret。标准 CI 只
运行完全离线的单元测试、协议测试、Schema 快照、Mock HTTP 和共享合约测试，不运行 ignored
Live Provider 测试，也不向 workflow 注入 Provider API Key。Semifold PR status workflow 只读取
发布计划并维护 PR 状态评论；CD 仅使用 GitHub Token、OIDC 权限和仓库配置的
`CARGO_REGISTRY_TOKEN`，不得把 token 输出到日志。

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
#[serde(transparent)]
pub struct ToolCallId(String);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: ToolCallId,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: ToolCallId,
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

`ToolCallId` 是非空、透明序列化的字符串新类型，统一用于 `ToolCall.id`、
`ToolResult.call_id` 和流式 ToolCall 事件。它在 JSON 和 Schema 中仍表示字符串，构造和
反序列化必须拒绝空字符串，不得通过默认值制造无效 ID。Provider 返回可用 ToolCall ID 时，
Adapter 必须原样保留；只有 Provider 确实没有提供 ID 时才允许生成本次消息历史内唯一的关联
ID，且不得用生成值覆盖或冒充另一个 Provider 身份。

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
    pub finish_reason: Option<FinishReason>,
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

`finish_reason = None` 表示 Provider 没有报告结束原因；
`Some(FinishReason::Unknown(value))` 表示 Provider 明确报告了 Armillae 尚不认识的值。
Adapter 不得根据响应内容推断或合成缺失的结束原因。JSON 反序列化时字段缺失或显式 `null`
均映射为 `None`；序列化时保留 `finish_reason` 字段并以 `null` 表达 `None`，保持唯一的输出形态。

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
- 后续同 Provider 请求可能需要回放的 reasoning、签名和 ToolCall metadata；
- 需要保留但不参与通用逻辑的数据。

ProviderData 不能用于绕过已经存在的标准字段。它属于 canonical content，
`CompletionResponse::as_assistant_message()` 必须原样保留。Adapter 按 `(provider, kind)` 明确
声明 replay 规则：目标与 `provider` 相同且 kind 已知时验证并还原；目标不同或 kind 未知时
不注入目标 wire request，但保持原始 Armillae history 不变并产生 `not_forwarded` compatibility
fact。已知 replay data 结构损坏时 Candidate projection 失败，不得丢弃后继续冒充精确回放。

Usage、response ID、system fingerprint 和只用于诊断的 metadata 不应伪装成 replay content；
它们继续进入各自标准字段或受控 `provider_metadata`。是否需要在未来给 ProviderData 增加显式
replay/observation 分类，必须以实际 Adapter 实现证据驱动，不在本次设计中提前扩张 Schema。

## 7. `armillae-llm`：Bridge、Router 与模型调用

### 7.1 Bridge 接口

```rust
pub type BoxFuture<'a, T> =
    Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub type CompletionStream =
    Pin<Box<dyn Stream<Item = Result<CompletionEvent, BridgeError>> + Send>>;

pub trait LlmBridge: Send + Sync {
    fn capabilities(&self) -> BridgeCapabilities;

    fn project(
        &self,
        request: &CompletionRequest,
    ) -> Result<ProjectionReport, BridgeError>;

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

`project` 是同步、无网络副作用的 Provider 无关预检：它验证直接 Bridge 的能力和目标 Adapter
投影，返回 compatibility facts，但不暴露、缓存或交出 Provider wire request。调用方可以先用
它审计手动跨 Provider 调度；`complete`/`stream` 不要求调用方预先调用 `project`，仍会在内部
执行相同投影。Router 后续只编排 Candidate，并复用该方法；实际 projection 仍由 Candidate
对应的 Adapter 完成，不能形成另一套能力判断或编码路径。

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
- 请求要求 Specific Tool，而 Provider 不支持：直接 Bridge 返回错误；Router 将其作为候选不
  匹配事实，不自动改成 Auto。
- Provider 不支持 Developer role：只有规范已命名且宿主策略显式允许时才能转换，否则明确
  拒绝；不得默认改写成 System。
- 不支持 Streaming：`stream` 直接返回能力错误，而不是伪造流。

`tool_choice` 存在时请求必须同时提供至少一个 Tool Definition；`Specific { name }` 指定的
名称必须出现在本次请求的 Tool Definition 中，否则属于 `InvalidRequest`，不能交给 Provider
猜测或修正。

Text 是所有 Bridge 的基础输出能力。`ToolChoiceCapabilities` 分别表达四种 ToolChoice，
`OutputFormatCapabilities` 分别表达 JSON Object 与 JSON Schema，不能用一个总开关掩盖
Provider 的部分支持。`tool_calling = false` 时所有 ToolChoice 能力必须为 false；请求使用的
具体变体不受支持时，直接 Bridge 必须在本地拒绝，Router 则在网络前选择下一 Candidate。
能力预检不能原地改写 canonical request。

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
        id: ToolCallId,
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

    #[error(
        "request content at message {message_index}, content {content_index} is incompatible with {target_provider} projection: {kind}"
    )]
    ProjectionIncompatible {
        target_provider: String,
        message_index: usize,
        content_index: usize,
        kind: String,
    },

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

Bridge 只提供 `retryable`、`retry_after` 等事实，不决定是否再次执行推理请求。单 Candidate
重试仍由宿主策略决定；跨 Candidate fallback 由本规范的 Router 显式执行，不进入具体 Adapter，
也不属于 Tool 或 Turn 调度。

### 7.5 配置

```rust
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BridgeConfig {
    pub api_version: String,
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
5 秒、请求超时为 60 秒，两者必须大于零；不设置跨 Provider 的任意最大值。同一 Candidate 的
自动重试不属于 Transport，仍由宿主根据 `BridgeError` 中的事实决定；Router 只执行 RFC 0003
定义的跨 Candidate fallback，不能把 fallback 隐式变成同一 Candidate 重试。

`BridgeConfig` 的 `api_version` 必须等于 `armillae.llm/v1alpha1`，`provider` 和 `model` 必须
非空，`provider_options` 必须是 JSON Object。通用层只验证跨 Provider 的结构；
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

文件解析和运行时 Builder 最终生成同一个 `BridgeConfig`。`BridgeConfig` 只描述一次 Bridge
构造所需的 Provider、模型、凭证、传输与扩展配置，不包含 Adapter Driver 选择。宿主可以在
自己的外层配置中保留 `driver` 或其他路由字段，并在运行时据此选择 `RigBridgeFactory` 或未来
的其他 Factory；Armillae 不约束宿主外层配置的格式。配置生命周期为：

```text
TOML / JSON / Rust Builder
           │
           ▼
      BridgeConfig
           │ 默认解析或 BridgeResolveContext
           ▼
  ResolvedBridgeConfig
           │ 宿主在运行时选择的 Adapter Factory
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

pub struct BridgeResolveContext<'a> {
    secret_resolver: Option<&'a dyn SecretResolver>,
    endpoint_policy: Option<&'a dyn EndpointPolicy>,
}

impl BridgeConfig {
    pub fn resolve(
        &self,
    ) -> BoxFuture<'_, Result<ResolvedBridgeConfig, BridgeError>>;

    pub fn resolve_with<'a>(
        &'a self,
        context: BridgeResolveContext<'a>,
    ) -> BoxFuture<'a, Result<ResolvedBridgeConfig, BridgeError>>;
}

pub struct ResolvedBridgeConfig {
    config: BridgeConfig,
    credential: Option<SecretString>,
}
```

常规 Environment、File 或无凭证配置使用零参数 `BridgeConfig::resolve()`。只有
`CredentialRef::Resolver` 或宿主需要限制显式 endpoint 时才构造 `BridgeResolveContext`，通过
链式 `secret_resolver(...)`、`endpoint_policy(...)` 设置对应宿主能力，并调用
`resolve_with(...)`。Context 字段保持私有，避免未来增加解析阶段宿主能力时破坏外部结构体字面量；
不得为 Resolver 与 EndpointPolicy 的不同组合扩张多个专用 resolve 方法。

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

`BridgeFactory::driver()` 是 Factory 自身的稳定标识，供宿主发现、组织或选择 Factory；它不
属于 `BridgeConfig`，也不要求宿主在编译时固定 Factory。第一阶段由宿主在读取自己的运行时
配置后直接选择并实例化 `RigBridgeFactory`。Armillae 第一阶段不提供 Factory Registry、配置
Loader 或动态插件加载；后续若出现多个 Adapter 实现与共享注册机制的真实需求，再单独设计。

### 7.7 Provider Projection 与兼容性事实

P7 直接 Bridge projection 的最小公共合约冻结为：

```rust
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageContentLocation {
    pub message_index: usize,
    pub content_index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompatibilityAction {
    NotForwarded,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompatibilityFact {
    pub location: MessageContentLocation,
    pub source_provider: String,
    pub target_provider: String,
    pub kind: String,
    pub action: CompatibilityAction,
    pub lossy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionReport {
    pub target_provider: String,
    pub facts: Vec<CompatibilityFact>,
}
```

这些类型是本地运行时报告，不是持久化配置或 Provider wire contract，本轮不为其派生 Serde 或
JSON Schema。若 Router route report 后续需要稳定线格式，必须在实际持久化需求明确后单独冻结，
不能让当前报告反向污染 `armillae-core` canonical Schema。

Bridge Adapter 在发送前必须从 canonical request 生成目标 Provider projection。Projection
至少区分：

- 标准字段的精确转换；
- 同 Provider 已知 replay data 的验证与还原；
- 外部/未知 ProviderData 的 `not_forwarded`；
- 已由规范命名、且宿主策略允许的兼容转换；
- 无法安全表达标准语义或已知 replay data 的 Candidate projection failure。

Compatibility fact 至少包含 canonical message/content 位置（适用时）、source Provider、target
Candidate、kind、采取的 action 及是否存在语义损失。Fact 不能包含消息正文、reasoning 内容、
Tool 参数、ToolResult、Secret、header 或原始 Provider 响应。它必须能进入 Router route report
和结构化可观测性；不得只写入无法由调用方判断的自由文本日志。

外部或未知 ProviderData 产生的 `not_forwarded` fact 默认不使 Candidate 失败：只要标准 Role、
ToolCall、ToolResult、ToolChoice、Schema 和安全语义仍可表达，目标调用可以继续。标准语义无法
保持、同 Provider 已知 replay data 损坏或安全策略失败时才属于 projection failure。

直接 Bridge 调用继续接受 canonical request，并在内部执行相同 projection。调用方通过
`LlmBridge::project(&request)` 在发送前取得报告；该调用不得消费 Mock 脚本、记录一次 Model Call
或执行网络 I/O。没有 fact 的精确投影返回空 `facts`。`ProjectionIncompatible` 只用于标准语义
无法保持或同 Provider 已知 replay data 结构损坏；外部/未知 ProviderData 使用
`NotForwarded` fact，不得借此错误阻断直接 Bridge。

### 7.8 LLM Router 与 Model Fallback

Router 接收有序、宿主构造的 Candidate 列表。Candidate 至少具有宿主稳定 ID 和
`Arc<dyn LlmBridge>`；Router 不负责读取宿主配置、解析 Secret、选择 Factory 或创建 Bridge。
每个 attempt 都从同一 canonical request 重新投影，不复用前一 Candidate 的 wire request。
Candidate 的 Provider 无关预检/投影边界由 Adapter 提供，Router 不解析 ProviderData value、
不构造 Rig 消息，也不复制 Provider codec。

Route report 必须区分只完成能力/投影预检的 Candidate 与实际发送过 Provider 请求的 Candidate，
并包含最终选择、脱敏错误类别和 compatibility facts。默认允许 fallback 的事实为
`UnsupportedCapability`、Candidate projection 不兼容、RateLimited、Timeout 和 retryable
Transport；`InvalidRequest`、Cancelled、安全策略、Authentication、PermissionDenied 默认终止。
ProviderRejected、非 retryable Transport 和 InvalidProviderResponse 只有在显式 policy 按
Provider/code/status 允许时才 fallback，不能解析错误字符串猜测。

非流式 Route 在首个成功 attempt 后立即返回。Streaming 只允许在创建流失败或第一个语义事件
之前 fallback；发出任何 `ResponseStarted`、内容、Usage 或 ProviderEvent 后必须固定 Candidate，
后续失败返回 `StreamInterrupted`。Drop Route Future/Stream 必须取消当前 attempt 且不能启动
新的 Candidate。

Router 不实现 `LlmBridge`，因为一次逻辑 Route 可能包含多个 Provider Model Call。它不执行
Tool、不自动继续 ToolCall、不维护 history，也不提供负载均衡、成本优化或模型自动发现。

### 7.9 Mock Bridge

`MockBridge` 是 Bridge 合约的一等实现，用于离线测试下游调度：

```rust
let call_id = ToolCallId::new("call-1").expect("static mock ToolCall ID is non-empty");
let bridge = MockBridge::scripted([
    MockResponse::tool_call(call_id, "get_weather", json!({ "city": "上海" })),
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

`RigRequestMapper` 和 `RigResponseNormalizer` 继续是两个单向边界，不合并为同时负责请求、
响应或传输的宽泛 Provider 对象；但它们必须共享同一个 Provider 的 replay 规则或窄化 codec
helper，保证 Normalizer 产生的已知 ProviderData 能由 Request Mapper 验证并还原。实际网络
调用始终只由 rig `CompletionModel` 执行。

不得根据“是否出现 ToolCall”等内容猜测 Provider 缺失或已明确返回的结束原因。Provider
没有报告结束原因时保持 `None`，返回未知结束值时转换为
`Some(FinishReason::Unknown(value))`；协议允许缺失的 ID/model 保持 `None`，协议要求存在但
实际缺失时返回 `InvalidProviderResponse`。第一阶段 OpenAI Normalizer 读取 OpenAI raw
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

额外提供 `deepseek`、`minimax` 和 `moonshot` 三个具名 Provider，只接入它们由 rig
暴露的 OpenAI-compatible Chat Completions 路径。MiniMax 和 Moonshot 同时提供的
Anthropic-compatible 路径、Model Listing、Embedding、多模态及 Provider 高级参数不属于该
增量范围。三者分别使用 rig 的原生 Provider Client，以保留其请求线格式修正和响应类型；
不得把具名 Provider 简化为仅替换 endpoint 的 `openai-compatible`，否则会绕过 rig 已知的
Provider 差异。

三者都要求 Bearer credential。未显式配置 endpoint 时使用 rig 对应 Provider 的默认全局
地址；中国区或其他兼容地址由调用方通过显式 endpoint 选择，并继续经过通用 URL 与宿主
EndpointPolicy 校验。本阶段不为三个具名 Provider 开放任何专属 `provider_options` 或请求
扩展字段，非空配置和具名命名空间下的未知扩展必须在发送前拒绝。`stop`、`seed`、
ToolChoice 和 OutputFormat 等已经进入公共协议的字段仍通过 OpenAI-compatible wire mapper
转换，不能重复放入 Provider 扩展。

具名 Provider 使用固定且保守的能力预设：

| Provider | ToolChoice | OutputFormat | 其他 |
| --- | --- | --- | --- |
| `deepseek` | `auto`、`none` | JSON Object | Tool Calling、并行 ToolCall、System；不支持 Developer |
| `minimax` | `auto`、`none`、`required`、`specific` | JSON Object、JSON Schema | Tool Calling、并行 ToolCall、System；不支持 Developer |
| `moonshot` | `auto`、`none` | JSON Object | Tool Calling、并行 ToolCall、System；不支持 Developer |

DeepSeek 在未显式关闭 thinking 时会抑制强制 ToolChoice；本阶段未开放该 Provider 参数，因而
必须在本地拒绝 `required` 和 `specific`。Moonshot 的 rig 路径会把 `required` 改写为
`auto` 并注入提示消息、对 `specific` 返回 Provider 错误；Armillae 不接受这种语义降级，
必须在能力预检阶段拒绝两者。DeepSeek 和 Moonshot 的 rig profile 都不支持 JSON Schema
response format，Adapter 只声明 JSON Object。P4 非流式实现完成时这些 Provider 声明
`streaming = false`；通过 P5 Streaming 合约后，`openai`、`openai-compatible`、`deepseek`、
`minimax` 和 `moonshot` 统一声明 `streaming = true`，其他能力矩阵保持不变。

MiniMax 和 Moonshot 的 OpenAI-compatible 响应复用 OpenAI raw response normalizer，但错误
metadata 必须保留具名 Provider。DeepSeek 使用自身 raw response normalizer，保留其可选
response ID/model、finish reason、system fingerprint、缓存 token usage 和 reasoning 内容；
reasoning 继续按公共转换规则进入 `ProviderData { provider = "deepseek" }`；DeepSeek Request
Mapper 必须把结构合法的同 Provider reasoning 还原为 Rig Reasoning，使普通多轮和
reasoning + ToolCall + ToolResult continuation 可以回放。不同 Provider、未知 kind 或畸形
DeepSeek reasoning 分别按 7.7 节执行 not-forwarded 或 projection failure，不得静默丢弃。
三个 Provider 的 OpenAI-compatible ToolResult 都沿用本节记录的 `is_error` 省略策略。

Anthropic 使用 rig 原生 Messages Client，默认 endpoint 为 `https://api.anthropic.com`，请求
路径为 `/v1/messages`，credential 通过 `x-api-key` 发送，并由 rig 添加固定
`anthropic-version` header。显式 endpoint 继续经过通用 URL 与宿主 EndpointPolicy 校验。
本阶段不开放 Anthropic `provider_options` 或请求扩展；任何非空配置或扩展都必须在发送前
拒绝，避免把 prompt caching、beta header、thinking 或其它高级能力隐式带入公共契约。

Anthropic 使用固定且保守的能力预设：支持 Streaming、Tool Calling、并行 ToolCall、全部既有
ToolChoice、JSON Schema 和 System role；不支持 Developer role 与 JSON Object。System 只允许
出现在消息历史开头，避免 rig 将中途 System 静默重排到顶层。Anthropic 请求必须由构造期默认值
或单次请求显式提供 `max_output_tokens`；`stop` 映射为 `stop_sequences`，`seed` 因 Provider
不支持而本地拒绝。JSON Schema 必须使用 `strict = true`；Anthropic wire 只接收 schema，不接收
公共协议中的描述性 name，因此 Adapter 校验 name 非空后不下发该字段。rig 0.41 会为 Anthropic
自动补齐 required/additionalProperties、移除数字约束并将 oneOf 改为 anyOf；Adapter 必须先
验证 schema 已属于不会发生语义改写的严格子集：所有 object 属性均 required、
`additionalProperties = false`、无数字约束且无 oneOf。不符合时本地拒绝，不能交给 rig 静默
放宽。ToolResult 的 JSON content 由 rig 按 Anthropic wire 能力序列化为紧凑 JSON 文本。

Anthropic wire 原生支持 `ToolResult.is_error`，但 rig 0.41 的通用 `ToolResult` 不承载该字段，
并固定转换为 `is_error: None`。为避免静默丢失错误事实，Rig Anthropic Adapter 允许
`is_error = false`（wire 缺失等价于 false），对 `is_error = true` 返回 `InvalidRequest`；本阶段
不为这一字段复制 Anthropic 请求类型或建立自有 HTTP 传输层。需要原生错误标记的调用方应选择
能够保留该事实的其它 Driver，而不是依赖 Rig Adapter 改写 ToolResult content。

Anthropic 非流式响应要求非空 ID 和 model；`end_turn`/`stop_sequence` 映射为 Stop，
`max_tokens` 映射为 Length，`tool_use` 映射为 ToolCall，未知 stop reason 进入
`FinishReason::Unknown`。`stop_sequence` 与 cache-creation token usage 只进入受控 metadata。
流式路径复用统一状态机；Anthropic 在 reasoning delta 后给出的完整带签名 Reasoning 必须完成并
替换同一 content index，不能生成重复 ProviderData block。P7 必须审计该 signed reasoning 的
同 Provider 请求回放；签名无法由 Rig 0.41 安全还原时应形成 Candidate projection failure，
由 Router 决定 fallback，不得仅删除签名后继续请求。

rig 0.41 会在 Anthropic Provider parser 内过滤 `StreamingEvent::Unknown` 和未知 delta，且其
公开 terminal stream item 不携带 response ID、model 或 finish reason。Adapter 对已经暴露的
rig Unknown item 继续生成 `ProviderEvent`，但不复制传输层来捕获 rig 未暴露的原始 SSE；相应
终端事实保持 `None`，不得推断。需要原始未知 Anthropic SSE 的使用场景应选择保留该能力的其它
Driver。这是 Rig Provider 边界的显式能力限制，不作为升级 Rig 或引入原生 Adapter 的理由。

Ollama 使用 rig 原生 `/api/chat` Client，默认 endpoint 为 `http://localhost:11434`，允许经过
通用校验和宿主 `EndpointPolicy` 的显式 HTTP/HTTPS endpoint。Ollama 默认不要求 credential；
当配置 credential 时按 rig 契约发送 Bearer token，以支持反向代理或受保护部署。本阶段不开放
Ollama `provider_options` 或请求扩展，避免将 `think`、`keep_alive` 和任意模型参数绕过公共协议
带入 wire。公共 temperature 与 max output tokens 分别由 rig 映射为 `options.temperature` 和
`options.num_predict`；stop 与 seed 显式映射为 `options.stop` 和 `options.seed`。

Ollama 使用保守能力预设：支持 Streaming、System、Tool Calling、并行 ToolCall、JSON Object
和 JSON Schema；不支持 Developer role，也不声明任何 ToolChoice 变体，因为 rig 0.41 会警告后
忽略该字段。JSON Object 通过最小 object schema 下发；JSON Schema 要求非空 name、object
schema 和 `strict = true`，name 只作为 Armillae 描述字段，不进入 Ollama wire。模型是否实际
遵循 schema 或调用 Tool 仍可能因本地模型而异，远端拒绝必须标准化，Adapter 不自动降级。

Ollama 原生 ToolCall 不提供调用 ID，ToolResult 只通过 `tool_name` 关联。Adapter 必须为每个
返回 ToolCall 生成响应内唯一的 Armillae ID；调用方把 Assistant ToolCall 和 ToolResult 放回
后续历史时，Request Mapper 按先前 ToolCall ID 显式恢复工具名。多个同名 ToolCall 依靠原始
顺序关联，Armillae ID 与内容顺序保持稳定；缺少先前 ToolCall 的孤立 ToolResult 必须本地拒绝。
Ollama wire 也不承载 `ToolResult.is_error`，因此沿用 OpenAI 的显式省略策略，保留调用方给出的
content，不拒绝或自动包装错误结果。
Ollama thinking 模型返回的 reasoning 同样属于待审计 replay data；Rig 能够还原时必须闭环，
不能继续使用“响应保留、下一请求统一拒绝”的单向转换。

Ollama 非流式响应不提供 response ID，model 必须非空；`done_reason` 映射为标准或 Unknown
finish reason，评估计数映射为 Usage，受控的 created-at 和 duration 数据进入 metadata。流式
NDJSON 由 rig 负责跨 HTTP/UTF-8 chunk 重组；ToolCall 在 rig 0.41 中以完整结构化参数事件暴露，
因此 Adapter 不伪造不存在的字符串 delta。terminal item 已暴露的 `done_reason`、Usage 和安全
duration metadata 必须进入最终响应；stream model 被 rig 丢弃时保持 `None`。rig 的 Ollama
parser 会忽略未知 JSON 字段且不产生 Unknown item，Adapter 不建立第二套 NDJSON 传输层捕获它们；
需要原始未知事件的用户应选择能够保留该事实的 Driver。

P5 Streaming 复用相同的 Request Mapper、能力预检和 rig `CompletionModel::stream` 传输边界，
由 Provider 无关的私有流式状态机将 rig item 转换为 Armillae 事件。五个当前 Provider 使用同一
Streaming 合约，不为具名 Provider 复制或分叉公共语义。MiniMax 和 Moonshot 仍不接入
Anthropic-compatible API。

rig 0.41 的公开 OpenAI-compatible stream item 不暴露响应 ID、实际 model 或 finish reason，
因此流式 `ResponseStarted` 的 `id`/`model` 和最终 `CompletionResponse` 的
`id`/`model`/`finish_reason` 保持 `None`，不得依据配置、内容或 ToolCall 猜测。终端
`Final` item 报告的 Usage 必须保留；Reasoning 转为 `ProviderData` 内容和
`ProviderEvent`，未知 rig stream item 转为 `ProviderEvent`。

ToolCall 增量以 rig 的 `internal_call_id` 作为交错分片关联键，以 Provider 提供的非空 ID 作为
Armillae `ToolCallId`。名称和参数可以跨任意 item 缓冲，但只有收到 rig 的完整 ToolCall 且
参数为完整 JSON 后才能产生 `ToolCallCompleted`。Provider 未提供非空 ID 时，可以基于该次流
内部稳定关联键生成仅在本响应内唯一的 ID，不得跨响应推断或复用。

成功流必须先后产生唯一 `ResponseStarted` 和唯一终端 `ResponseCompleted`；Usage 在终端响应
前报告。任何 item 级错误、未收到终端 `Final`、或流结束时仍有不完整 ToolCall，均以一个
`StreamInterrupted` 终止，不补发 `ContentCompleted` 或 `ResponseCompleted` 伪造完整响应。
调用方 drop Armillae stream 时必须同步 drop 所持有的 rig stream，以尽快释放/取消底层请求。

### 9.3 转换边界

转换代码必须集中在 `convert` 模块，并单独测试：

```text
Armillae Message          ↔ Rig Message
Armillae ToolDefinition   ↔ Rig ToolDefinition
Armillae CompletionRequest → Rig CompletionRequest
Rig CompletionResponse     → Armillae CompletionResponse
Armillae ProviderData       ↔ Rig Provider replay content
Rig Streaming Item         → Armillae CompletionEvent
Rig Error                  → Armillae BridgeError
```

转换必须满足：

- 不丢失 ToolCall ID。
- 不丢失多个 ToolCall 及其顺序。
- Assistant ToolCall 和对应 ToolResult 能在下一次请求中正确关联。
- Provider 原始未知输出转换为 ProviderData。
- 同 Provider 已知 reasoning、签名和 ToolCall metadata 能从 ProviderData 还原到下一请求。
- 跨 Provider 或未知 ProviderData 不进入目标 wire request，canonical history 保持不变并产生
  compatibility fact。
- Provider 原始响应只进入受控 metadata，不把 Secret 写入日志。
- 不依赖 Rig Agent 的 Tool 注册或执行路径。

`ToolResult.is_error` 是 Armillae 公共协议的一部分，但 Provider 线协议或所选 Driver 的通用
类型未必有对应字段。Adapter 必须为每个 Provider 显式定义该字段的兼容策略：能够承载原生错误
标记时进行语义映射；Provider 不支持时不得因此拒绝 ToolResult，也不得擅自改写或包装调用方
提供的内容；Provider 原生支持但 Driver 无法承载时，必须在转换前显式拒绝 `is_error = true`，
不得静默省略。

第一阶段 OpenAI/OpenAI-compatible 的 tool message 不承载独立的 `is_error` 字段，因此转换时
保留 `call_id`、content 及其顺序，但不把 `is_error` 下发到 Provider。原始 Armillae 请求和
调用方维护的消息历史仍保留该字段；调用方在 `is_error = true` 时必须通过 ToolResult content
向模型表达失败事实。Adapter 不自动添加错误前缀或结构，以免改变调用方定义的模型可见内容。
此行为必须由转换测试覆盖，不能作为未记录的字段丢弃。Anthropic 原生错误标记受 rig 0.41
通用类型限制，采用上一节记录的显式拒绝策略，而不是沿用 OpenAI 的省略策略。

### 9.4 首批 Provider

第一阶段按以下顺序扩展 Provider：

1. OpenAI 或 OpenAI-compatible：验证主协议和自定义 endpoint。
2. MiniMax、Moonshot 和 DeepSeek 的 OpenAI-compatible 路径：先验证具名 Provider 路由、
   保守能力预检和 Provider-specific raw response 归一化，再与 OpenAI/OpenAI-compatible
   一同通过 P5 Streaming 合约。
3. Anthropic：在隔离分支验证原生 Tool 协议、消息差异和统一 Streaming 合约。
4. Ollama：验证本地 Provider、无原生调用 ID 的 Tool 协议和 NDJSON 流式路径。

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
- `ToolCallId` 的非空约束与透明字符串线格式；
- 缺失、未知 finish reason 和 ProviderData 的前向兼容；
- canonical history 在任意 Candidate projection 后保持不变；
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
- `as_assistant_message()` 中同 Provider 已知 replay data 的下一请求闭环；
- 外部/未知 ProviderData 不进入目标 wire request 且产生 compatibility fact；
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
- 覆盖 rig terminal `Final` 带 Usage 与缺失 Usage；带 Usage 时转换为独立 `Usage` 事件并进入
  最终响应；
- 未知 Provider 事件不会丢失；
- drop Stream 后底层调用被取消。

### 11.5 Provider 测试分层

- 转换单元测试：完全离线，不访问 Provider。
- Mock HTTP/cassette 测试：验证请求和响应协议。
- Live 测试：使用真实凭证，默认 ignored，仅用于发布前验证。

测试夹具不得包含 API Key、Authorization header、用户隐私内容或未经脱敏的 Provider 响应。

### 11.6 OpenAI 协议端到端场景门禁

现有转换、Mock HTTP 和共享合约测试证明公共协议及 Adapter 单元行为，但不足以单独支撑
“全量支持 OpenAI 协议主流模型”的生态级声明。在发布该声明前，必须建立显式的 Provider/模型
矩阵，并至少以真实下游流程验证：

- 非流式和流式纯文本；
- System 与多轮消息历史；
- JSON Object 与受支持的 JSON Schema 输出；
- Tool Definition、单 ToolCall 和多 ToolCall；
- 流式 ToolCall 参数重组和 UTF-8 分片；
- `LLM -> ToolCall -> ToolResult -> LLM` 手工闭环；
- reasoning 普通多轮与 reasoning + ToolCall + ToolResult 回放；
- Usage、finish reason、请求 ID 和错误分类；
- 不支持能力的本地预检以及 Provider 远端拒绝。

本阶段支持声明门禁冻结为以下矩阵；它是可重复验证基线，不代表“总是选择最新模型”：

| Provider | 模型 | credential 环境变量 | endpoint |
| --- | --- | --- | --- |
| `openai` | `gpt-4.1-mini` | `OPENAI_API_KEY` | rig 默认 OpenAI endpoint |
| `deepseek` | `deepseek-v4-flash` | `DEEPSEEK_API_KEY` | rig 默认全球 endpoint |
| `minimax` | `MiniMax-M2` | `MINIMAX_API_KEY` | rig 默认全球 endpoint |
| `moonshot` | `kimi-k2` | `MOONSHOT_API_KEY` | rig 默认全球 endpoint |

DeepSeek 基线使用 Rig 0.41 提供的 `deepseek-v4-flash` 正式模型 ID。不得继续使用已于
2026-07-24 弃用的 `deepseek-chat` 或 `deepseek-reasoner` 别名；它们只作为分别映射到 V4 Flash
非思考与思考模式的 Provider 兼容入口，不构成冻结矩阵的可重复模型标识。

Live harness 默认 ignored，在具备明确凭证的发布工作站上串行执行，不进入普通离线 CI。每个
Provider 必须通过上述全部适用场景；能力矩阵明确不支持的场景以本地预检成功拒绝为通过。证据
只记录日期、配置的模型 ID、Provider 返回的实际模型、场景结果和标准化错误类别，不保存 prompt、
响应正文、Tool 参数、ToolResult、header 或 Secret。环境变量允许临时覆盖模型仅用于探索；覆盖
结果不能替代冻结矩阵的发布门禁。当前没有真实凭证时只能提交 harness 和“未执行”状态，不得把
离线测试推断成 Live 通过证据或对外全量支持声明。

### 11.7 Provider Projection 与 Router 测试

- 转换单测覆盖每个 Provider 的已知 replay、未知 kind、畸形 value 和跨 Provider 数据；
- 同 Provider encode/decode 保持 Text、ToolCall、ToolResult、ID、签名和内容顺序；
- Mock Candidate 覆盖能力不匹配、projection failure、rate limit、timeout、retryable transport、
  默认终止错误、显式 ProviderRejected policy 和候选耗尽；
- 每个 attempt 从原始 canonical request 重新投影，前一 wire request 不得泄漏到后一 Candidate；
- route report 区分 preflight-only 与 sent attempt，并对 compatibility fact 和错误完成脱敏；
- Streaming 覆盖首事件前 fallback、首事件后固定 Candidate、中断和 drop 不启动后续 Candidate；
- 默认 ignored Live 测试至少覆盖同 Provider reasoning 回放和两个已授权 Candidate 的 fallback，
  不在 fixture 或证据中保存正文与 Secret。

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
- Router Candidate/attempt ID、是否实际发送请求和最终选择；
- compatibility action、source/target Provider、kind 和是否 lossy。

默认不得记录：

- API Key 或认证 header；
- 完整消息正文；
- 完整 Tool 参数和 ToolResult；
- Provider 原始响应正文。
- ProviderData value、compatibility fact 对应的原始内容或各 Candidate 的 credential。

Rig Adapter 使用 `armillae::llm` target 和 `llm.bridge.call` span 表达上述事实，字段名保持稳定；
成功、标准化错误、流中断和调用方 drop 都必须结束一次观测。非流式首 token 延迟不可观测，保持
缺失；流式首个文本、ToolCall 或 ProviderData 语义事件记录 first-token latency。所有计数和延迟
都来自本地时钟或标准化响应，不读取正文推断。

本阶段不提供内容级调试开关或宿主脱敏器公共 API。所有发给 rig 的请求继续固定
`record_telemetry_content = false`，Armillae 自身 span/event 永不记录正文；这是比引入尚无使用
证据的通用 redactor 更小且更安全的收尾。rig 0.41 的 Ollama 实现在 `rig` DEBUG target 中会输出
原始 NDJSON 行，生产宿主不得启用 `rig`/`rig::completions` 的 DEBUG/TRACE；需要安全的内容级调试
时应先升级或替换 Driver，并通过新的设计变更引入脱敏契约，不能复用本阶段结构化 tracing 冒充。

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
[rig-core 0.41.0 P0 Spike](../spikes/rig-core-0.41.0.md)。

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

- 为 OpenAI、OpenAI-compatible、DeepSeek、MiniMax 和 Moonshot 实现统一的文本、Reasoning、
  ToolCall、Usage 与未知 Provider 语义事件。
- 完成参数重组、多 ToolCall 交错、中断、唯一完成事件和 drop 取消测试。
- 保持 rig 0.41 stream 层未暴露的 ID、model 和 finish reason 为缺失值，不进行推断。

### P6：更多 Provider

- Anthropic：使用 rig 原生 Messages Client，完成非流式、流式、Tool Calling、保守能力预检和
  响应归一化；接受 rig 对原始未知 Anthropic SSE 的过滤边界，不引入自有传输层。
- Ollama。
- 完成统一合约测试和能力矩阵。

### P7：Canonical Projection 与 Model Fallback

- 冻结 projection、compatibility fact、Candidate、policy、route report 和 routing error 的公共
  Rust 合约，不暴露 Rig 类型或宿主配置格式。
- 先修复 DeepSeek reasoning 的同 Provider 普通多轮与 Tool continuation 回放，再审计所有已
  支持 Provider 的 replay/observation/unknown 数据。
- 实现 canonical request 不变的目标 Provider projection 和脱敏 compatibility facts。
- 在 `armillae-llm` 实现 host-owned 非流式 Router，再实现首个语义事件前可 fallback 的
  Streaming Router。
- 完成转换、Mock Candidate、Mock HTTP、共享合约、取消、安全和默认 ignored Live 门禁。

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

### 14.1 P7 验收标准

1. `as_assistant_message()` 产生的已知同 Provider replay data 可以进入下一请求。
2. DeepSeek reasoning、ToolCall 和 ToolResult continuation 不再因 ProviderData 本地失败。
3. 跨 Provider projection 不修改 canonical history，也不向目标发送源 Provider 私有数据。
4. 所有 not-forwarded、兼容转换、候选跳过和 attempt 失败都有结构化脱敏事实。
5. Router 从宿主有序 Candidate 列表选择目标，并遵守默认与显式 fallback policy。
6. 每个 attempt 至多调用一个 Bridge 一次；Bridge 和 ToolExecutor 的既有边界不变。
7. Streaming 在首个语义事件后不切换 Candidate，drop 不启动新的 fallback。
8. 所有 Provider 通过 projection 合约；至少 DeepSeek 完成真实多轮和 Tool continuation 验证。

## 15. 风险与应对

### 15.1 rig API 变化

风险：rig 处于 0.x，升级可能修改消息、Tool 或流式类型。

应对：固定精确依赖版本；所有转换集中在独立 Adapter；通过合约测试驱动升级；禁止 rig 类型穿透公共 API。

### 15.2 Provider 语义不完全一致

风险：Role、ToolChoice、JSON Schema、finish reason 和流式事件在 Provider 间存在差异。

应对：保持 canonical 数据；由目标 Adapter 明确投影；同 Provider replay data 双向转换；跨
Provider 私有数据不外发但记录 compatibility fact；能力不匹配交给 Router 选择下一 Candidate。

### 15.3 流式 ToolCall 重组错误

风险：网络 chunk 不等于语义事件边界，Tool JSON 和 UTF-8 都可能跨 chunk。

应对：Adapter 按 Provider 协议增量解析；按 call/index 维护独立缓冲；完成后再解析 JSON；使用随机分片和属性测试。

### 15.4 抽象过度

风险：在缺少真实 Provider 反馈前设计过多未来能力。

应对：只实现已由真实 DeepSeek 回放问题证明必要的 Provider projection，以及宿主显式候选的
最小 fallback Router；不实现 Turn、Agent、Memory、Embedding、Vector Store、RAG、Tool 调度、
负载均衡、自动模型发现或插件机制。

### 15.5 Secret 和敏感内容泄漏

风险：Provider Client、Debug、错误或测试 fixture 可能包含认证和用户内容。

应对：SecretRef 与已解析 Secret 分离；自定义脱敏 Debug；默认不记录正文；fixture 提交前扫描敏感字段。

### 15.6 Fallback 重复调用与语义拼接

风险：Timeout 或流中断后切换 Provider 可能产生重复计费、重复推理或把不同 Provider 的输出
拼成一条伪响应。

应对：默认 fallback 错误集合保持窄化；route report 区分 sent attempt；ProviderRejected 只按
显式 code/status policy 放行；首个流式语义事件后固定 Candidate；取消后不得启动下一 attempt。

## 16. 与 Agentic 叙事运行时的边界

Agentic 叙事运行时、Turn、Memory、世界状态、持久化、回放和 Agent 调度不再作为本文的
“后续演进”展开，它们由 [RFC 0001：Agentic 叙事运行时](../rfcs/0001-agentic-runtime.md)
独立管理。

本文冻结的跨层不变量是：上层运行时可以直接组合 `LlmBridge` 与 `ToolExecutor`，也可以使用
`LlmRouter` 组合多个 Bridge；Router、Bridge 和 Tool Executor 都不依赖运行时，Router 不执行
Tool，Tool Executor 不持有 Bridge，任何一层都不得静默承担另一层的状态推进职责。

Embedding、Vector Store 和 RAG 也不合并进 `armillae-llm`；是否以及如何被运行时使用，由
运行时设计在场景与状态边界明确后决定。

## 17. 总结

持续生效的架构边界是：

> LLM Bridge 负责一次 Provider 无关的模型调用和 Tool Calling 协议传输；Tool Executor 负责一次 ToolCall 的类型安全执行；是否继续调用模型由下游显式决定。

`LlmRouter` 在该边界之上为一次逻辑请求完成 Candidate projection 与显式 fallback，不改变
Bridge 的单 Provider call 语义，也不接管 Tool continuation。rig-rs 被用于降低 Provider 接入
成本，但被严格隔离为可替换 Adapter；Armillae canonical 协议、history 和兼容性事实不绑定
任何 Driver。

第一阶段的 LLM Bridge 由 `armillae-llm` 提供；未来的 Embedding Bridge、Vector Store 和 RAG
分别由独立 crate 提供，避免把不同请求、错误和生命周期语义压入一个通用 Bridge。

## 18. 调研参考

- [rig-core crate 文档](https://docs.rs/rig-core/latest/rig_core/)
- [Rig CompletionModel](https://docs.rs/rig-core/latest/rig_core/completion/request/trait.CompletionModel.html)
- [Rig Completion 协议](https://docs.rs/rig-core/latest/rig_core/completion/index.html)
- [Rig Streaming 协议](https://docs.rs/rig-core/latest/rig_core/streaming/index.html)
- [Rig GitHub 仓库](https://github.com/0xPlaygrounds/rig)

外部依赖的版本、能力和行为以实现 Spike 及锁定版本的源码为准，不能仅依赖本文链接所指向的 latest 文档。
