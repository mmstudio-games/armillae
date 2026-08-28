# bevy_ecs 0.19.1 P0 可行性 Spike

> 日期：2026-08-28
> 结论：通过
> 依赖：`bevy_ecs = "=0.19.1"`
> MSRV：Rust 1.95.0

## 目标

在创建 `armillae-simulate` 与 `armillae-simulate-bevy` 产品 crate 前，验证 Simulate Active
Spec 第 10 节冻结的 Bevy-native 签名、最小依赖、执行图、显式上下文、故障隔离和目标平台
边界。Spike 使用独立临时 Cargo crate，不修改产品 workspace，也不提前实现 Hosted Loader、
持久化或 Agent Harness。

## 依赖与工具链结论

- `bevy_ecs = "=0.19.1"` 在 `default-features = false`、仅启用 `std` 时编译并通过运行测试；
- Rust 1.95.0 可以使用同一锁定版本编译全部冻结签名和 Spike 测试代码；
- additive `parallel` 可以直接映射到 `bevy_ecs/multi_threaded`，启用后
  `MultiThreadedExecutor` 可构造、初始化并执行 Schedule；
- 未启用 Adapter `parallel` 时可以显式安装 `SingleThreadedExecutor`，不依赖 Cargo feature
  union 得到的默认执行器；
- `std + multi_threaded` 配置可以为 `wasm32-wasip1` 编译；Bevy 在 `wasm32` 目标仍选择
  single-threaded executor，因此 Adapter 不得在该目标报告 parallel capability；
- `bevy_reflect`、`async_executor`、`backtrace`、`serialize` 和完整 Bevy App 均不是首阶段
  Adapter 的必需依赖或 feature。

## 公共签名验证

以下 Active Spec 签名在 Bevy 0.19.1 上编译通过：

- object-safe `BevyModule` 的 `self: Box<Self>` 注册入口；
- `BevyModuleRegistrar::bind_clock`、`add_system` 和精确返回
  `SystemExecutionResult` 的 `add_fallible_system` bounds；
- `ExecuteContext`、`ClockComponent<C>` 与 `AdvanceContext<C>` 的 Resource/Component derive；
- typed Clock CRUD、typed batch Advance，以及 closure-scoped `inspect_world` / `write_world`；
- `Schedule::initialize`、`set_apply_final_deferred(true)`、SystemSet ordering、single-threaded 与
  multi-threaded executor。

Spike 首次按 Spec 复制 typed 协议时发现 `TypedAdvanceOutcome<C>` 的嵌套 derive 无法自动补出
`C::Step: Debug + PartialEq` 约束。经项目方确认，Active Spec 已改为条件手动实现 `Debug` 与
`PartialEq`，没有扩大 `Clock` / `Clock::Step` 的必需约束；修正后的签名在 Rust 1.95.0 编译
通过。

## 运行行为验证

独立 Spike 测试验证了：

- SystemSet 的 `before` / `after` 映射可以形成稳定顺序；
- final deferred application 在 `Schedule::run` 返回前应用 `Commands`；
- `Schedule::initialize` 会在应用初始化前执行 `Local<T>::FromWorld`；
- Execute 与 typed Advance Context 可以只在单次 Schedule 期间插入并在返回后移除；
- `Changed<T>` 只在 Schedule 被显式运行时生效，不会自动触发执行；
- Bevy 明确标记为 skipped 的 `SystemParamValidationError` 不进入 fallback handler；
- `SystemExecutionResult` 可以通过 `IntoSystem::pipe` 进入 Adapter collector，不先被 Bevy
  fallback 擦除；
- 未捕获 Bevy error 可以由不读取、不格式化 payload 的 fallback handler 转成私有无数据
  unwind marker，并在外层 `catch_unwind` 分类；
- descriptor/register staging panic 可以丢弃 staging，执行 panic 可以被边界捕获并转入
  Faulted 状态；
- `World: Send` 的编译期断言成立；在错误线程访问 `NonSend` 会 panic，边界可以捕获该 panic，
  但包含 `NonSend` 的 World 仍必须回到其所有权线程销毁，符合 Spec 的线程亲和性要求；
- `panic = "abort"` 下 `catch_unwind` 不提供进程内恢复，产品文档和错误保证必须保持该限制。

## 执行证据

临时 crate 通过 Cargo CLI 创建并只用 Cargo CLI 添加依赖。关键验证命令与结果为：

```text
rtk cargo +1.95.0 check --locked
    Finished dev profile

rtk cargo test --locked                         # std only
    10 passed

rtk cargo test --locked                         # std + multi_threaded
    11 passed

rtk cargo check --locked --target wasm32-wasip1 # std + multi_threaded
    Finished dev profile
```

产品实现必须把这些行为迁移为 `armillae-simulate-bevy` 的永久共享合约与专项测试；本记录本身
不能代替产品 Backend 合约测试。

## 决策

P0 Spike 通过。首个 Adapter 精确锁定 `bevy_ecs 0.19.1`，最小配置为
`default-features = false, features = ["std"]`；Adapter 的 `parallel` feature additive 映射到
`bevy_ecs/multi_threaded`。可以按 Active Spec 使用 Cargo CLI 创建产品 crate。

Spike 没有推翻 RFC 0002 的 Backend、Clock、Module、同步执行或 Faulted 决策。唯一规范修正是
项目方明确批准的 typed outcome 条件 Trait 实现，不改变 wire shape 或运行语义。
