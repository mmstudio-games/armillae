# 项目与发布实施清单

> 状态：Active
> 技术事实来源：[Armillae 设计索引](../DESIGN.md)与
> [LLM Bridge 发布规范](../specs/llm-bridge.md#53-版本与发布)
> 最后核对：2026-08-27

本清单记录跨子系统的文档、仓库治理、版本与发布差异，不承载子系统内部功能需求。

## 开源与文档

- [x] 添加英文/中文 README、贡献指南和完整 `AGPL-3.0-only` 许可证正文。
- [x] 将 `.agents/DESIGN.md` 调整为生态入口，并将已生效规范与待决 RFC 分离。
- [x] 将 `.agents/TODO.md` 调整为索引，按子系统拆分实施清单。
- [x] 将 Agent 工程文档迁入 `.agents/`，为 `docs/` 保留稳定版用户文档职责；公共接口冻结并
      明确授权推进稳定版前不创建或维护独立用户指南。
- [x] 移除 alpha 阶段提前创建的 `docs/llm-bridge.md` 及 README 引用，协议事实继续由 Active
      Spec、RFC、Harness 和实现示例承载。
- [x] 同步设计索引、Spec、RFC、实施清单、README、贡献指南和当前实现的链接与范围。

## Crate 发布准备

- [x] 各 crate 分别在自己的 manifest 中锁定 package version，不继承统一 workspace version。
- [x] 为新建的 `armillae-tools-macros` 补齐 Semifold package 配置和与既有 crate 一致的发布
      元数据，并通过 `cargo package` 验证；正式 publish dry-run 证据仍由下方统一发布门禁跟踪。
- [x] 撤销未消费的 stable promotion changeset，并使用 Semifold CLI 将既有四个 crate 恢复为
      `alpha` 通道；`semifold status` 不得再把下一版本计划为无 prerelease 后缀的稳定版。
- [ ] 冻结 0.1 范围并明确 Router 是否纳入，清零重大链路缺陷，完成共享/Mock/安全门禁、
      代表性 Live 矩阵、所有 crate publish dry-run 和至少一个真实下游验证后，再决定是否进入
      `beta`；stable 必须经过至少一个 beta 稳定周期并单独授权。
- [ ] 分别通过所有 crate 的 `cargo publish --dry-run`，且不执行实际发布或版本提升。
      当前 `armillae-core` 已通过；其余既有 crate 已完成无元数据警告的打包阶段。新增
      `armillae-tools-macros` 已通过 `cargo package`（含 tarball 编译验证），其正式 dry-run 与其余
      未完成项一起等待对应上游版本按授权发布到 registry 后重跑。
- [ ] 公共接口冻结且稳定版推进获得明确授权后，再为稳定契约编写 `docs/` 用户指南；alpha 与
      beta 阶段不以不稳定接口维护独立用户文档。
