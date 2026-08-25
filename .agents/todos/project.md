# 项目与发布实施清单

> 状态：Active
> 技术事实来源：[Armillae 设计索引](../DESIGN.md)与
> [LLM Bridge 发布规范](../specs/llm-bridge.md#53-版本与发布)
> 最后核对：2026-08-25

本清单记录跨子系统的文档、仓库治理、版本与发布差异，不承载子系统内部功能需求。

## 开源与文档

- [x] 添加英文/中文 README、贡献指南和完整 `AGPL-3.0-only` 许可证正文。
- [x] 将 `.agents/DESIGN.md` 调整为生态入口，并将已生效规范与待决 RFC 分离。
- [x] 将 `.agents/TODO.md` 调整为索引，按子系统拆分实施清单。
- [x] 将 Agent 工程文档迁入 `.agents/`，为 `docs/` 保留面向使用者的文档职责。
- [x] 同步设计索引、Spec、RFC、实施清单、README、贡献指南和当前实现的链接与范围。

## Crate 发布准备

- [x] 四个 crate 分别在自己的 manifest 中锁定 package version，不继承统一 workspace version。
- [x] 为四个 crate 补齐一致且可继承的发布元数据，并确保打包后的 README 路径有效。
- [ ] 分别通过四个 crate 的 `cargo publish --dry-run`，且不执行实际发布或版本提升。
      当前 `armillae-core` 已通过；其余 crate 已完成无元数据警告的打包阶段，完整验证等待
      `armillae-core 0.1.0` 按授权发布到 registry 后依赖拓扑重跑。
