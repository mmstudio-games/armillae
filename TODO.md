# Armillae TODO 索引

> 状态：Active
> 作用：全项目实施清单入口；不在本文件重复子系统任务
> 最后核对：2026-08-24

任务必须来自已经确认的设计。本索引和 `todos/*.md` 只记录设计与当前实现之间的差异，不是
独立需求来源；Discovery 阶段的待决问题继续保留在设计文档中。

## 实施清单

| 范围 | 清单 | 状态 | 当前重点 |
|---|---|---|---|
| LLM Bridge 与 Tool Executor | [todos/llm-bridge.md](todos/llm-bridge.md) | Maintenance | OpenAI 协议 E2E 支持声明门禁；其它 Provider 扩展暂停 |
| 项目与发布 | [todos/project.md](todos/project.md) | Active | 首次多 crate 发布验证 |
| Agentic 叙事运行时 | 暂无实施清单 | Discovery | 先完成 [运行时设计](docs/AGENTIC_RUNTIME.md)中的场景与核心决策 |

## 使用规则

- 先从 [Armillae 设计索引](docs/DESIGN.md) 定位权威子系统设计，再进入对应实施清单。
- 根索引只记录清单状态和当前重点；具体任务只写入一个 `todos/*.md` 文件。
- 跨子系统任务写入 `todos/project.md`，并链接所有受影响的权威设计。
- 新子系统只有在设计范围和验收标准冻结后才能创建实施清单。
- 完成项必须同时满足设计、实现和必要验证；暂停项保持未勾选并明确标注状态。
