# nestrs-core 内部实现阅读指南

本文面向维护容器实现的人。业务使用方式见 [宏使用指南](NESTRS_MACROS.md)；编译器如何
汇总注册清单见 [编译器适配说明](NESTRS_COMPILER_ADAPTER.md)。这里说明 core 内部职责、
状态和所有权，避免读代码时需要同时理解全部实现。
本轮实际执行的测试与平台范围见[重构验收记录](NESTRS_CORE_REFACTOR_VALIDATION.md)。

## 1. 从一条主线开始

```text
编译器生成的注册入口
        ↓
RegistrySnapshot：本次构图的声明快照
        ↓
GraphCompiler：展开 → 路由 → 输入 → 拓扑与生命周期
        ↓
ValidatedGraph：不可变执行计划
        ↓
facade：用户借用与请求入口
        ↓
runtime：一个中央协调器接受请求、推进任务、发布实例和关闭 owner
        ↓
activation：准备输入、调用 typed adapter、持有实例、迭代释放
```

编译器负责收集类型化声明；core 在 `build` 时验证完整图。编译 Rust 成功不表示已经
运行图验证，图验证成功也不保证数据库连接等外部资源初始化成功。

建议先读 [lib.rs](../nestrs-core/src/lib.rs) 的模块说明，再沿下表进入实现。

| 模块 | 负责什么 | 阅读入口 |
| --- | --- | --- |
| registration | 描述 provider、依赖、binding、查询根和闭合蓝图，接收编译器注册快照 | [mod.rs](../nestrs-core/src/registration/mod.rs) |
| graph | 选择精确类型和 key 的路由，校验输入、循环和生命周期，冻结计划 | [compiler.rs](../nestrs-core/src/graph/compiler.rs) |
| facade | 构建、scope、预热、关闭和绑定 owner 借用期的查询引用 | [facade.rs](../nestrs-core/src/facade.rs) |
| runtime | 一份可变协调状态、任务图、缓存和关闭状态机 | [mod.rs](../nestrs-core/src/runtime/mod.rs) |
| activation | 类型化输入、真实工厂借用、稳定地址和强 lease | [mod.rs](../nestrs-core/src/activation/mod.rs) |
| error | 公开错误及可共享、非递归的依赖失败路径 | [error.rs](../nestrs-core/src/error.rs) |

## 2. 图编译的四个阶段

[compiler.rs](../nestrs-core/src/graph/compiler.rs) 只编排阶段并汇总诊断。每个阶段的输入、
输出和局部状态都在对应文件内说明。

1. [expand.rs](../nestrs-core/src/graph/compiler/expand.rs) 求有限声明闭包。
   它区分显式注册与闭合蓝图，以队列处理需求；自动 binding 和蓝图是能力目录，只有
   实际需求才会启用。精确 type/key 的显式 provider 优先，不能跨 key 回退。
2. [routes.rs](../nestrs-core/src/graph/compiler/routes.rs) 选择 concrete 与 trait 路由。
   `primary` 只解决同 key 的 trait 多候选，不能覆盖重复 concrete 注册。
3. [inputs.rs](../nestrs-core/src/graph/compiler/inputs.rs) 编译每个输入槽位。
   optional 无候选固化为缺席；有候选仍接受全部检查。目标和 typed preparer 在这里确定。
4. [topology.rs](../nestrs-core/src/graph/compiler/topology.rs) 使用 Kahn 算法生成顺序，
   用显式栈定位循环，再沿依赖优先顺序传播 scope 要求。

拓扑算法可以合并重复边，但输入槽位必须完整保留。例如同一服务的两个字段都注入
Transient，运行时需要构造两个实例。把“用于计数的边”和“实际消费槽位”混成一组，
会在图看似正确的情况下改变生命周期语义。

所有阶段只处理描述，不调用 constructor、factory、Default、value 或 cleanup。诊断
在冻结前汇总；验证失败时不会交付部分图，也不会提前开始实例化。

## 3. 运行时只维护一套调度状态

[handle.rs](../nestrs-core/src/runtime/handle.rs) 负责发命令和等待结果。
[coordinator.rs](../nestrs-core/src/runtime/coordinator.rs) 是唯一修改 owner、缓存、任务、
就绪队列及 worker 集合的位置。[worker.rs](../nestrs-core/src/runtime/worker.rs) 只接收
一个节点的完整输入并构造它，或者清理一个实例，不递归解析其他服务。

需要区分三种身份：

| 身份 | 含义 | 保存期限 |
| --- | --- | --- |
| ProviderId | 冻结图中的一份服务声明 | 整个容器存活期 |
| TaskId | 一次实际构造 occurrence | 接受构造到成功或失败 |
| DependencyLease | 一个实例及其必要依赖的强所有权 | 最后一个持有者释放之前 |

Singleton 按 root/provider 合并；Scoped 按 scope/provider 合并；Transient 每个消费
槽位创建独立任务。Singleton 首次从哪个 scope 请求，都在 root 上下文中构造。

活跃任务的状态由 [task.rs](../nestrs-core/src/runtime/task.rs) 定义：

```text
Unexpanded → Waiting → Queued → Running → 结束并移出任务表
                 └─ 依赖失败 ──────────→ 结束并移出任务表
```

共享缓存独立保存在 [owner.rs](../nestrs-core/src/runtime/owner.rs)：

```text
无缓存 → Building(TaskId) → Ready(lease) 或 Failed(error)
```

任务表仅用于推进未完成的工作，不再用已经完成的任务充当长期缓存。命中 Ready 或
Failed 时直接返回结果，命中 Building 时共享该活跃任务。Transient 不进入共享缓存。

依赖就绪立即把消费者入队，不等待整个拓扑层完成；所有 scope 共同使用 root 的构造
名额。依赖未就绪的任务不占用名额。父任务失败后，之前接受的子任务仍会继续处理。

## 4. 实例为什么有多处强持有

这些持有者承担不同责任，不能为了减少字段而相互替代：

- owner 的 journal：保存每个成功发布实例，保活公开查询返回的 `&T`，并记录 cleanup 顺序。
- 共享缓存：合并 Singleton/Scoped 初始化，提供成功结果或共享失败。
- Injection：让注入令牌即使被安全代码移出服务，仍然保活对应实例。
- factory frame：给跨 await 的参数提供真实借用来源。
- 实例的依赖 leases：保证消费者析构时其必要依赖仍存活。

发布顺序必须是 **写入 journal → 更新缓存 → 通知等待者和消费者**。journal 位于
门面与协调器共享的 OwnerData；即使 Tokio 协调任务已经退出，仍被用户借用的 owner
也能保活已经返回的引用。门面恢复裸指针前还核对实际类型及 trait 投影所属的实例。

[preparation.rs](../nestrs-core/src/activation/construction/preparation.rs) 集中执行
“完整准备令牌 → 写入空槽位 → 收纳实际返回令牌的 lease”。输入检查失败不提交保活
记录；类型或 required/optional 形态不匹配时，消费接口也不取走原有槽位。

class adapter 取得带 lease 的 Injection；factory adapter 借用 worker 内真实的
FactoryLeaseFrame。两者继续使用不同的交付方式，不伪造 `'static` 引用。

字段的 `#[lazy]` 由 [lazy.rs](../nestrs-core/src/activation/lazy.rs) 类型化为
`LazyInjection<T>`。activation 只依赖 `LazyResolver` 协议，不反向依赖 runtime；
[runtime/lazy.rs](../nestrs-core/src/runtime/lazy.rs) 实现请求合并与弱 owner 归属。
运行时 watch 保存一次 occurrence 的结果，使等待取消后仍能重新连接同一初始化；
句柄内 OnceCell 保存准确类型的 Injection，保证返回引用的存活期。两者不能合并成
“OnceCell 初始化闭包直接重新 resolve”，否则 Transient 在取消重入后可能创建两次。

`get()` 不通过 Deref 隐藏异步等待。构造 worker 的 task-local 阻止同任务内等待未完成
延迟字段，但不传播到业务自行 spawn 的任务；factory 不能借派生任务绕过此限制。
未来若支持构造重入，需要独立设计挂起、名额归还和等待环诊断协议。

## 5. 关闭和最终内存释放

协调器中的 owner 阶段是：

```text
Open → Draining → Cleaning { running: false/true } → Closed
```

Draining 拒绝新解析并排空已经接受的任务。进入 Cleaning 后清除缓存中的重复持有，
图没有延迟输入时从 journal 逆成功发布时间取实例。含延迟输入时，消费者可能先于
依赖发布，所以先在完整冻结 DAG 上反向执行 Kahn 算法：消费者的实例全部清理后才
允许清理依赖，同步满足条件的实例优先取较晚发布者。没有实例的节点也参与约束传播，
不会漏掉间接依赖。每个 owner 同时只有一个 cleanup worker，上一项 hook 和当前
触发的释放完成后才推进下一项。root 必须先等待所有 scope 关闭。

外部原子状态只用于跨线程判断是否还能接受请求；详细关闭阶段只由协调器维护。
显式关闭、取消关闭等待和 Owner 的非阻塞 Drop 都提交同一种幂等 Close 命令。

逻辑关闭与最终内存释放不同。逃逸的 Injection 不阻塞关闭，却会继续保活必要内存。
[instance.rs](../nestrs-core/src/activation/instance.rs) 管理实例与 lease；
[release.rs](../nestrs-core/src/activation/release.rs) 在最后一个 lease 释放时用队列
循环销毁载荷，避免深链 Arc 析构再次形成递归。队列锁绝不包围用户析构代码。

ReleaseDomain 独立于 Tokio，runtime 停止后仍能同步安全释放；异步 cleanup 则需要
活跃 runtime。不会为 Drop 临时启动 runtime，也不会对不返回的 hook 强制超时。

## 6. 维护时必须保留的边界

本次调整面向可读性和状态模型，未改变公开 API 或业务声明语法。没有新增 runtime
crate，没有恢复公开内部 ABI，也没有第二个 resolver 或协调器。仍保留必要的运行时
类型、槽位、lease 身份及关闭检查；`Delivery::Direct` 的既有校验分支没有顺带删除。

中文源码注释集中解释模块职责、状态转换、先后次序和 unsafe 成立条件。底层协议的
测试实现统一位于 crate 根目录的 `tests/unit/` 和 `tests/compiler/`；`src/` 仅通过
`#[cfg(test)]` 与 `#[path]` 挂载内部单元测试，保持原有模块私有边界。布局和运行方法见
[测试目录说明](../nestrs-core/tests/README.md)。迁移保留原断言；后续修改应以这些约束
审查代码，而不只核对最终是否能取到一个服务。本文没有给出吞吐量或耗时改善承诺，
性能需要独立量测。
