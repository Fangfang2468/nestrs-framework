# nestrs-core 测试目录

所有测试实现统一保存在 crate 根目录的 `tests/`，`src/` 只保存生产实现及必要的
测试模块挂载声明。目录位置与 Rust 模块可见性分开处理，不为迁移测试开放内部 API。

```text
tests/
├── compiler_plan_required.rs   # 真实非 cfg(test) core 的普通 Cargo / 工具链启动契约
├── support/graph/              # 冻结执行计划的只读快照和显示名称，不编入生产 core
├── unit/                       # 不依赖声明工具链的内部单元测试
│   ├── activation/             # 稳定地址、注入令牌、强 lease、迭代释放
│   │   └── construction/       # 完整输入验证、直接消费、类型化投影、工厂借用
│   ├── graph/                  # 生产计划装配、冻结元数据与只读快照
│   ├── runtime/                # 协调器、预热、订阅、缓存、并发、失败传播与关闭
│   ├── contracts.rs            # 真实类型身份与构造输入的安全契约
│   ├── error.rs                # 共享失败记录、深路径显示与释放
│   ├── panic_payload.rs        # panic 载荷回收、原始载荷传播与有界二次失败处理
│   └── facade_api.rs           # 容器公开门面与 owner 借用期
└── compiler/                   # 真实 Nestrs 工具链生成声明的隔离契约
```

## 单元测试如何加载

生产模块只保留下面这样的声明，测试函数、辅助类型和断言均位于 `tests/unit/`：

```rust,ignore
#[cfg(test)]
#[path = "../../tests/unit/activation/instance.rs"]
mod tests;
```

这样测试仍是被测模块的子模块，能通过 Rust 原有的私有访问规则检查内部状态。
测试通过 `super` 使用私有实现，不需要增加 `pub`、测试专用公开 ABI 或额外
编译一套生产实现。`#[path]` 相对声明所在的源文件目录解析，不是相对 shell 工作目录。

`unit/`、`support/` 与 `compiler/` 不设置 `main.rs`，因此不会被 Cargo 误当成独立集成测试目标。
普通单元测试通过已有 lib 测试目标执行：

```sh
cargo test -p nestrs-core
cargo test -p nestrs-core --lib -- --list
```

### 按实现职责选择测试

| 实现约束 | 对应测试 |
| --- | --- |
| 编译计划槽位、真实类型和目标端装配 | `unit/graph/plan.rs` |
| 共享计划不共享 root 实例，服务级 lazy 只控制预热根 | `unit/runtime/initialization.rs` |
| 共享缓存、全 root 构造上限、依赖就绪推进和深链激活 | `unit/runtime/coordinator.rs` |
| 普通查询与预热取消、完成和退订竞争 | `unit/runtime/requests.rs`、`subscriptions.rs` |
| 延迟槽位取消接续、owner 关闭、runtime 退出、深链释放 | `unit/runtime/lazy.rs`、`lazy_delivery.rs` |
| 稳定实例、发布后取址、令牌保活、输入回滚与真实 factory frame | `unit/activation/` 及其中的 `construction/` |
| 输入形态和准确类型、失败后重试、缺席与消费状态 | `unit/activation/construction/inputs.rs` |
| concrete / trait 直接交付零临时分配、投影实例身份 | `unit/activation/construction/projection.rs` |
| factory 取消时借用保活、成功移交 lease、拒绝替换实例 | `unit/activation/construction/factory.rs` |
| 强 lease 的迭代释放、析构 panic 与逃逸令牌 | `unit/activation/instance.rs`、`unit/runtime/lazy.rs`、`lazy_delivery.rs` |
| 零／单订阅内联、大共享集合按键取消、Pending 槽位不合并 | `unit/runtime/compact.rs`、`task.rs`、`subscriptions.rs` |
| 完整 DAG 关闭顺序、缺席中间节点、重复 occurrence、跨 owner 排序工作空间复用 | `unit/runtime/cleanup.rs` |
| 容量维护保留活动记录、整窗口低负载回收与缩容滞后 | `unit/runtime/capacity.rs` |
| panic 载荷析构隔离、协调器存活、跨线程释放与回执归属 | `unit/panic_payload.rs`、`unit/runtime/panic_payloads.rs`、`unit/activation/instance.rs` |

取消与失效订阅的回归分别位于 `unit/runtime/requests.rs` 和
`unit/runtime/subscriptions.rs`。前者实际取消 `resolve` / `warm_up` future，验证
退订命令及完成竞争；后者在共享构造仍未结束时检查等待者和反向依赖记录，覆盖
重复槽位、其他健康消费者及 12,000 层失败传播。不能仅凭最终关闭后任务表为空，
判定慢初始化期间的失效订阅已经及时回收。

core 测试直接提供 `CompiledNode` / `ValidatedGraph`，或向真实 `PlanAssembly` 写入
`ActivationAdapter` / `ProjectionAdapter` 与冻结的选择编号。测试 fixture 明确给出拓扑、
路由和 Scope 要求，不执行候选选择、泛型物化或第二套图验证。协议损坏用例直接调用
生产 sink，不能通过自动补齐输入的辅助层掩盖错误。

候选、key、primary、环和生命周期等图规则由 `cargo-nestrs/tests/di_plan.rs` 测试生产
模型；涉及 rustc 真实类型、宏卫生、泛型和跨 crate 的规则由工具链编译契约覆盖。
`support/graph` 只保留只读快照和名称展示，旧图编译器及声明模型已移除。


`compiler_plan_required.rs` 是有意保留的 Cargo 集成测试入口，链接不含 `cfg(test)`
的真实 core 库。它证明普通 Cargo 构建的应用在两个 build 入口都会收到
`BuildError::CompilerPlanUnavailable`，且不会静默交付空容器。经 `cargo nestrs`
编译时，同一组测试严格验证两个 build 入口都成功；没有 Tokio runtime 时必须返回
`BuildError::RuntimeUnavailable`。测试用 CLI 提供的编译期 `NESTRS_TOOLCHAIN_ID`
区分这两种预期，不依赖只标记 core 本体的 `nestrs_compiler` cfg，也不跳过断言。

## 工具侧测试负责什么

| 层次 | 当前入口 | 验证内容 |
| --- | --- | --- |
| 闭合图纯模型 | [di_plan.rs](../../cargo-nestrs/tests/di_plan.rs) | type/key、primary、optional、重复与孤立 binding、完整 Lazy 边、环、Scope、槽位、深链和确定性 |
| Rust 类型与生成来源 | [compiler_contracts.rs](../../cargo-nestrs/tests/compiler_contracts.rs)、[registry_abi.rs](../../cargo-nestrs/tests/registry_abi.rs) | 真实执行计划、宏来源、泛型闭合、自动/显式投影、精确 factory 优先；v2 driver 拒绝旧 core |
| 非法最终图 | [aot_graph.rs](../../cargo-nestrs/tests/aot_graph.rs) | 即使 main 不调用 build，结构错误仍在 check/build 拒绝 |
| 源码诊断 | [diagnostics.rs](../../cargo-nestrs/tests/diagnostics.rs) | 原生 Cargo JSON 的业务标题、完整类型或 key 高亮、关联声明、独立错误与末尾 cause |
| 跨 crate | [cross-crate-binding](../../cargo-nestrs/tests/fixtures/README.md#关键契约的阅读位置) 与 [验证脚本](../../tools/verify-cross-crate-binding.py) | 私有 concrete、同名类型身份、上游/兄弟需求、泛型投影、实例共享、关闭和导图 |
| 声明与业务行为 | [DI fixtures](../../cargo-nestrs/tests/fixtures/di/tests)、constructor / query / provider-lazy 契约 | 真实应用中的属性语法、借用、字段映射、普通查询和延迟行为 |
| 审阅与展示产物 | [reflection_artifacts.rs](../../cargo-nestrs/tests/reflection_artifacts.rs)、graph 测试 | 版本化字段、编号、转义、数据完整性；不把 JSON 当运行时输入 |

“编译失败”和“正确定位业务源码”是两项独立契约。纯模型断言结构化的消费者、
槽位、候选与路径；codegen 检查原 token Span；真实 driver 检查 Cargo JSON。
仅匹配非零退出码或内部错误名称不足以证明诊断正确。来源不可精确恢复时保留可信
边界，不能用猜测行号通过测试；展示规则见[诊断指南](../../docs/NESTRS_DIAGNOSTICS_DESIGN.md)。

安全契约按当前对象验证，不保留另一份已经移除的 preparer 或注册 oracle：完整输入
失败应释放全部 lease，错误读取不消费原槽位，普通与 lazy 的 None 检查准确类型，
投影不能替换为同类型的另一个实例。factory 必须借用从原输入派生的 frame，取消
和成功移交均保持真实保活关系。上面的 construction 测试直接覆盖这些契约。

## 编译器契约如何加载

`compiler/` 的每组用例由 `cargo-nestrs/tests/compiler_contracts.rs` 创建隔离测试工程，
包含真实 core 源码，并通过 Nestrs driver 编译运行。它们直接核验 `CompiledApplication::load` 得到的
最终执行计划与类型化 adapter，而非调用旧注册收集器。泛型通过普通查询贡献闭合
根；primary 通过真实竞争实现的 trait 路由核验；factory 形态还会实际调用并检查
错误文本。每个工程有独立的反射执行入口。

`compiler/implicit_drop_roots.rs` 还核对 core 本体以 `cfg(test)` 编译时的隐式析构
查询根，包含参数、局部值、聚合字段与未执行分支，以及 `forget`、`ManuallyDrop`
和引用不贡献析构根的对照。生产 core rlib 仍不参与业务查询摘要收集。

`reflection_artifacts` 还验证生成清单不会求值用户 factory/value、未使用 provider
仍受检查，以及投影裁剪之后所有编号同步更新。`registry_abi` 验证同名手写模块、
marker 或泛型协议不能伪造来源，并在引用 core 的空图 check 中核对版本化 sink。

这些用例需要真实工具链，普通 core 测试不会自动运行它们。先按
[工具链说明](../../docs/NESTRS_CARGO_TOOLCHAIN.md)安装匹配的编译器及 `rustc-dev`，
再从仓库根目录构建同一套 CLI、driver 和私有 bridge。Linux / WSL 可执行：

```sh
python3 tools/build-toolchain.py
LD_LIBRARY_PATH="$(rustc --print sysroot)/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
RUSTC_BOOTSTRAP=nestrs_driver \
cargo test -p cargo-nestrs --features compiler-driver --test compiler_contracts
```

`RUSTC_BOOTSTRAP` 只授权编译工具自己的 driver crate，不能改为 `1` 或用来放宽应用
编译。示例中的 `rustc` 必须与构建工具时使用的固定编译器一致；上面的动态库设置只
适用于 Linux / WSL。跨平台维护脚本通过 `tools/toolchain_support.py` 配置对应
host 的动态库路径，harness 会为应用子进程移除 bootstrap，并使用真实 driver。

普通 `cargo test -p nestrs-core` 会执行内部单元测试，但启动集成测试只覆盖缺少计划
的路径。通过已构建的 CLI
执行下面的命令，可以同时验证 Nestrs 编译计划存在时的启动路径：

```sh
target/debug/cargo-nestrs test -p nestrs-core
```

涉及业务声明的黑盒集成和 UI 编译失败用例属于 `cargo-nestrs/tests/fixtures/di/`；
constructor、查询方法和跨 crate 能力还各有独立 fixture 与工具侧 harness。测试它们
应运行对应 `cargo-nestrs` 编译契约，不能把普通 core 测试成功当成声明功能已验收。
消费者优先 cleanup、逐 owner 串行清理和错误聚合还通过 DI fixture 中的
`runtime_lazy.rs`、`runtime_cancellation.rs`、`runtime_concurrent_disposal.rs` 验证。
这些测试位于工具侧，不让 core 反向依赖工具 crate。新增 core 测试应继续放到本目录；不能重新在 `src/`
创建测试文件或内联测试主体，也不能通过削弱私有边界来迁就文件布局。

输入生成的业务回归位于 DI fixture 的 `runtime_direct_inputs.rs`：覆盖自动字段、
显式 constructor、泛型、用户函数/常量名称冲突（含同步/异步/延迟 factory 参数与
Result 分支），以及 `Default` / `value` 的顺序和字段类型推导。factory adapter 复用
原 factory 函数名绑定输入、参数元组和 Result 结果，以 `self::函数名` 调用业务函数；
回归同时声明旧的 context、逐参数和结果临时名对应的业务常量，以及名称相同的
业务 factory，验证不会出现常量模式误解析或局部绑定遮蔽函数调用。
输入交付的计数分配器测试只证明 `take` / projection 本身不创建临时
堆载荷，不代表整个容器或一次业务查询零分配。真实业务查询的累计请求字节、存活
峰值与 scope 关闭后释放检查见[性能与内存](../../docs/NESTRS_PERFORMANCE.md)。
