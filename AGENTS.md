# Armillae Agent 协作约定

本文件适用于 Armillae 仓库中的全部目录和任务。所有后续 Agent 在分析、设计、实现、测试
和交付工作时都必须遵守以下约定。

## 1. 权威文档与实施清单

- 第一阶段技术设计：[docs/DESIGN.md](docs/DESIGN.md)
- 设计与实现差异清单：[TODO.md](TODO.md)

`docs/DESIGN.md` 是第一阶段需求、架构、协议、安全边界和验收标准的技术事实来源。
`TODO.md` 只记录设计与当前实现之间的差异，不是独立需求来源。

- 开始实现前，先确认需求已经由 `docs/DESIGN.md` 表达。
- 设计领先于代码时，在 `TODO.md` 中保留对应未完成项。
- 只有代码和必要验证均已满足设计要求后，才能勾选对应 TODO。
- 不得让 TODO、代码或测试与设计文档冲突。
- 不得将未来阶段的设想直接当作第一阶段需求。Turn Runner、完整 Agent、Memory、RAG、
  自动 Tool Loop 和多 ToolCall 调度策略等能力不属于当前实施范围。

## 2. 修改授权

- 在用户明确要求实施或修改之前，不得直接修改代码、配置或文档。
- 讨论方案、分析问题、审查代码或提出建议不等同于获得实施授权。
- 未获得实施授权时，可以进行必要的只读检查，但必须保持工作区内容不变。
- 用户只授权修改文档时，不得顺带修改代码、配置、依赖或生成产物。
- 不得因为某项修改看起来简单、合理或属于自然后续步骤，就推断用户已经授权。

## 3. 设计优先与同步顺序

如果实现前或实现过程中出现新的架构决定、公共协议变化、Provider 兼容策略、安全边界、
依赖选择或第一阶段范围变化，必须按以下顺序处理：

1. 更新 `docs/DESIGN.md`；
2. 更新 `TODO.md`；
3. 修改代码、配置、测试和示例。

- 不得先实现技术方案变化，再补写设计文档。
- 不得把重要设计决定只隐藏在代码、测试或依赖中。
- 如果变化方向或影响尚不明确，停止相关实现并向用户确认。
- 每次代码任务交付前，检查公共 API、配置契约、示例、安全说明和架构文档是否需要同步；
  若不需要更新文档，也必须在最终说明中明确原因。

## 4. 第一阶段架构边界

- `LlmBridge` 只负责一次 Provider 无关的 Model Call，不执行 Tool，也不自动继续调用模型。
- `ToolExecutor` 只负责一次 `ToolCall -> ToolResult`，不持有或调用 Bridge。
- 多 ToolCall 的顺序、并发、审批、重试和错误反馈策略由下游负责。
- Armillae 拥有公共协议；`rig-core` 只能出现在 `armillae-bridge-rig` 中，其类型不得穿透
  其他 crate 的公共 API、配置或持久化数据。
- `armillae-core` 不依赖异步运行时、HTTP Client 或 LLM SDK。
- `armillae-bridge` 与 `armillae-tools` 不互相依赖。
- Provider 差异必须通过能力预检、显式转换、`ProviderData` 或命名空间扩展表达；不得静默
  丢弃 Role、ToolCall、ToolResult、Usage、错误或未知 Provider 事件。
- ToolCall ID、内容顺序和流式内容索引必须在协议转换中保持稳定。

## 5. Rust 错误处理与 panic 约束

- 生产 Rust 代码不得使用可能 panic 的 `unwrap()`。
- 文件、网络、流式传输、解析、配置、Secret 解析、用户输入和 Provider 响应等可恢复失败，
  必须返回或转换为调用方可处理的结构化错误。
- 只有类型系统、构造器或同一函数内的穷尽分支已经证明内部不变量时，才允许在生产代码中
  使用 `expect()`；消息必须具体说明该不变量。
- 不得使用 `expect()` 掩盖外部输入、运行环境、并发、I/O 或第三方服务失败。
- 测试代码可以为断言和前置条件使用 `unwrap()`、`expect()`，但不得借此规避生产错误处理。
- `BridgeError` 和 `ToolExecutionError` 的分类应保持设计语义，不得用无结构字符串替代调用方
  需要判断的错误事实。

## 6. Cargo manifest 与依赖管理

- 除非用户明确要求直接编辑，否则不得手动编辑任何 `Cargo.toml`。
- 新增或移除依赖、crate、workspace member 或 crate 元数据时，必须使用能够表达该操作的
  Cargo CLI，例如 `cargo add`、`cargo remove`、`cargo new` 或 `cargo init`。
- 不得使用文本补丁、脚本或重定向绕过 Cargo CLI。
- 如果 Cargo CLI 无法表达所需变更、命令失败或预期结果不明确，立即停止并向用户确认。
- 执行 Cargo CLI 后必须检查 manifest、lockfile 和 workspace 的实际变化，确认其符合
  `docs/DESIGN.md` 和任务授权。
- `rig-core` 必须使用经过 P0 Spike 验证的精确版本；升级前先通过转换与 Bridge 合约测试
  验证兼容性，不得仅依据 latest 文档升级。
- 当前受限环境中新增依赖需要网络时，应直接请求以受审批方式运行限定到目标 package 的
  `cargo add -p <package>`，不得因网络不可用而改为手动编辑 manifest。

## 7. 安全、Secret 与日志

- Secret 值不得进入可序列化配置、`Debug`、错误 Display、tracing、测试 fixture 或快照。
- 默认不得记录完整消息正文、Tool 参数、ToolResult、Authorization header 或 Provider 原始
  响应正文。
- 自定义 endpoint 和动态 Provider 配置必须在构造阶段完成 URL、类型、已知字段及宿主策略
  校验；不得默认扩大 SSRF 可达范围。
- Provider 原始错误和 metadata 在保留诊断价值前提下必须脱敏。
- Live Provider 测试默认 ignored，仅在具备明确凭证与授权时运行；夹具不得包含真实凭证、
  用户隐私内容或未经脱敏的响应。

## 8. 测试与验证

- 公共协议变更必须覆盖 Serde round-trip、顺序与 ID 关联、前向兼容和必要的 Schema 快照。
- Tool 变更必须覆盖参数校验、Schema、执行错误、输出序列化、重复注册和 Context 透传。
- Bridge 与 Adapter 变更必须复用共享合约测试，不得为每个 Provider 定义互相冲突的外部语义。
- Streaming 变更必须覆盖任意分片、UTF-8 边界、多 ToolCall 交错、流中断、唯一完成事件和
  drop 取消。
- 测试应按转换单元测试、Mock HTTP/cassette 测试和默认 ignored 的 Live 测试分层。
- 交付前至少运行与改动范围相称的格式检查、Clippy 和测试；无法执行的验证必须明确说明。

## 9. Shell 命令

- 在本仓库执行 shell 命令时，始终使用 `rtk` 前缀。
- 搜索文件或文本时优先使用 `rtk rg --files` 和 `rtk rg`。

## 10. 任务完成与交付说明

每次完成任务时，最终回复除总结业务和技术产出外，还必须明确说明本次技术方案相对于任务
开始时既有设计发生了什么变化。

- 如果技术方案发生变化，说明变化内容、原因、影响，以及如何同步到 `docs/DESIGN.md` 和
  `TODO.md`。
- 如果技术方案没有变化，明确写明“本次技术方案相对既有设计无变动”。
- 列出实际完成的验证，以及任何尚未完成、被阻塞或需要用户决定的事项。

