# Nestrs 激活运行时：v0–v6 生命周期闭环、Tokio 并行与应用 API 收口

> 状态：v0 已在 `nestrs-core` 落地 Root 定向的同步 Singleton 激活闭环；v1–v5 依次完成
> 同任务异步、类型化 Scoped、显式 cleanup、消费者持有的 Transient 与静态多层 Scope。
> v6 以 Tokio 替换本地 future 集合：在不改动 `Inject<T>`、宏 ABI 或容器公共线程边界的前提下，
> 让独立就绪节点可在 Tokio multi-thread runtime 上安全并行。它仍只在 `nestrs-core` 实现，
> 不引入 `nestrs-bootstrap`、动态 service locator 或新的生命周期枚举。
> 当前公开 API 进一步收敛：根路径只保留面向应用的 `ServiceProvider` 与 `BuildError`；
> 静态 Scope 位于 `nestrs_core::scope`，注册元数据、Compiler 与 Arena 是内部实现。
>
> 相关文档：
>
> - `NESTRS_CORE_DI_DESIGN.md`：面向应用开发者的当前使用方式、公开边界与设计要求
> - `AGENTS.md`：项目架构共识与 Git/Commit 规范
> - `NESTRS_REFLECTION_DESIGN_QA.md`：Provider 模型、Registry 与 Compiler 路线
> - `NESTRS_DI_QA_HANDOFF.md`：构造 ABI、`ConstructionContext` 与 `Inject<T>` 的决策

---

## 一、面向应用开发者的入口与边界

普通应用只需声明 provider、构建一个类型化根，并读取已经构建好的服务：

```rust
use nestrs_core::ServiceProvider;

let provider = ServiceProvider::<AppRoot>::build()?;
let root = provider.root();
let users = provider.get::<UserService>();
```

- `AppRoot` 本身必须是默认 key 的 Singleton。`build()` 只允许同步 class/sync-factory；
  `build_async().await` 在活动 Tokio runtime 中也允许 async factory 与 cleanup。
- `root()` 是应用的主要类型化入口。`get<T>() -> Option<&T>` 仅查询当前容器已经 eager
  激活、默认 key 的 concrete 持久服务；它不会注册、编译、物化泛型或创建新实例。
  trait、keyed provider 与 Transient 不能经由 `get` 查询；不在 Root 可达闭包内的服务返回
  `None`。因此 `get` 是只读观察，不是 `resolve` 或动态 service locator。
- `ServiceProvider` 的公共操作只有 `build`、`build_async`、`root`、`get` 与消费式
  `shutdown`。`BuildError` 是面向用户的高层错误出口，通过 `Display` / `Debug` 给出
  provider 声明上下文；Registry、Compiler、Arena 及其细粒度错误不属于普通应用 API。
- 服务继续由 `#[injectable]`、`#[factory]` 与 `#[bind]` 属性声明和 linkme 自动注册。
  `Inject<T>`、`Option<Inject<T>>` 与 `FactoryParameter<'frame>` ABI 不变；宏所需的
  `nestrs_core::__private` 是隐藏的跨 crate ABI，不是应用代码应手写的扩展点。
- 容器仍不承诺 `Send` 或 `Sync`。所有 `*_async()` 激活入口要求当前线程有 Tokio runtime：
  current-thread runtime 安全地并发推进，multi-thread runtime 才可能物理并行；调用方直接
  `.await` 容器 future，不把容器或 scope 交给 detached spawn。

静态 Scope 是显式导入的高级能力，不出现在根路径：

```rust
use nestrs_core::scope::{ScopeLayer, ScopeProvider};

type AppScopes = ScopeLayer<
    RequestRoot,
    ScopeLayer<TransactionRoot, ScopeLayer<CommandRoot>>,
>;

let provider = ScopeProvider::<AppRoot, AppScopes>::build()?;
let request = provider.create_scope()?;
let transaction = request.create_child_scope()?;
let command = transaction.create_child_scope()?;
```

异步入口使用同一条静态链：

```rust
let provider = ScopeProvider::<AppRoot, AppScopes>::build_async().await?;
let request = provider.create_scope_async().await?;
let transaction = request.create_child_scope_async().await?;
let command = transaction.create_child_scope_async().await?;
```

- `ScopeLayer<Root, Child = ScopeEnd>` 只描述一条编译期固定的线性 Scope 链；链头和每个
  child root 都必须是默认 key 的 Scoped provider，且必须在自己的层首次获得所有权。
  Transient 不能作为公开 root。
- `ScopeProvider<AppRoot, Chain>` 暴露应用 `root()`、仅默认 key concrete Singleton 的
  `get<T>()`、链头
  `create_scope*()` 与 `shutdown()`。`Scope<'parent, Root, Tail>` 暴露自己的 `root()`、
  当前层/精确祖先层/Singleton 的已提交默认 key concrete 服务 `get<T>()`、`shutdown()`，以及在
  `Tail` 存在时的 `create_child_scope*()`。这些查询同样不会触发动态解析，且不会返回 Transient。
- 同步 Scope 构建拒绝其可达图中的 async factory 与 cleanup；异步 Scope 构建使用 v6 Tokio
  激活器并接受持久消费者拥有的 Transient cleanup。scope 必须由内向外 shutdown，借用关系
  保证 child 不能超过 parent 或 Singleton provider 的寿命。

---

## 二、编译与可达图

Registry 按稳定顺序收集 linkme 的原始 `Provider::{Class, Factory}` 与
`TraitBinding`；Compiler 从默认 key 的 `Root` token 深度遍历并生成不可变编译图。
无关注册不参与校验。

- 精确 token 遵循“唯一候选 / 唯一 primary / 歧义”选择规则；optional direct 依赖可编译为
  absent 输入。
- trait 请求经 binding 派生 concrete token，并保存 typed projector；闭合泛型保留
  “显式候选优先，零候选才物化并缓存”的语义。
- 依赖循环、必选缺失、歧义及当前入口不支持的生命周期能力都在编译阶段报告。
- 编译器使用私有激活能力模式：同步入口禁止 async factory 与 cleanup，异步入口允许两者；
  其余能力边界在两个入口中一致。
- v2/v4/v5 在同一 Compiler 会话内处理 AppRoot 与全部按外到内声明的 ScopeRoot，避免
  closed-generic fallback 在多个根之间重复物化。`ServiceIdentifier` 继续负责 provider 选择、
  key、primary、trait binding 与泛型 provider 蓝图缓存；私有 `ActivationNodeId` 负责一次
  实际实例化。
- Singleton 与 Scoped activation node 按 token 去重；每条解析到 Transient 的静态注入边都会
  生成独立 node。因此同一 consumer 的两个 `Inject<T>` 以及不同 consumer 的同一 `T` 都不
  复用 instance。这是“每条注入边创建一次”，不是每次 `Inject<T>` 解引用时重新构造。
- 每个已解析输入都会记录其实际来源：父 Singleton Arena、当前或精确祖先 `ScopeFrameId`
  对应的 Scoped Arena、某个 transient occurrence，或已证明缺席的 optional 输入；运行时不会
  重新解析 token，也不会以“本层缺失就向父层回退”的方式猜测来源。
- 允许 `Scoped -> Singleton`，以及同层或子层读取已拥有的祖先 Scoped 服务；必须拒绝
  `Singleton -> Scoped`，也必须拒绝外层 Scope（含其 transient 子树）捕获已声明的后代
  Scoped 服务。这些规则同样覆盖 optional 已解析依赖、trait binding、key 和泛型 fallback
  导出的边。
- `EffectiveOwner::{Singleton, Scoped(frame)}` 沿 Transient 边传播：Singleton 路径可消费
  Singleton/Transient，某个 ScopeFrame 的路径可消费 Singleton、该层或祖先 Scoped、以及
  继承该 frame 的 Transient。直接 `Singleton -> Scoped` 继续给出 `LifetimeInversion`；
  `Singleton -> Transient -> Scoped` 给出携带持久 owner 和完整链路的
  `TransientLifetimeInversion`。`Scoped -> Transient -> Scoped` 仅在目标不属于后代 frame 时
  合法。
- trait binding 的生命周期继承所选 concrete provider，不能独立改变。cleanup 在同步入口报错；
  async build/建 scope 中，字段消费者持有的 transient cleanup 会保留到显式 shutdown。
- `#[factory]` 的 transient 参数只在 activation frame 内临时持有：frame、context/future 先被
  销毁，随后才 Rust Drop 这棵临时 transient 子树。若这棵临时子树任一 transient provider
  声明 cleanup，即使是 async 入口也以 `TransientFactoryParameterCleanup` 拒绝，绝不静默跳过 hook。

---

## 三、v6 Tokio 安全跨线程并行调度

异步入口在 Compiler 成功后通过 `tokio::runtime::Handle::try_current()` 取得当前 runtime。
`ServiceProvider::build_async()`、`ScopeProvider::build_async()`、`ScopeProvider::create_scope_async()`
与 `Scope::create_child_scope_async()` 都使用同一套 Tokio 调度器；缺失 runtime 时返回带有
runtime 缺失上下文的 `BuildError`。Tokio 依赖固定为最小的 `rt-multi-thread` 与 `sync`
feature：current-thread runtime 仍可安全运行，multi-thread runtime 则可将独立任务分配给不同
worker。worker 分配和独立兄弟的完成顺序都不是公共合同。

1. 协调器独占不可变图的 ready queue、输入准备、结果提交、Transient child ownership、失败排序
   与 rollback；它只把依赖已经成功提交的 activation node 交给 `JoinSet`。
2. 每个 worker 接收 `Send + 'static` 的 provider 调用数据、准备好的 `ConstructionContext` 与
   Arena lease set。class、同步 factory 和 async factory 都在 worker wrapper 内执行；factory 的
   `FactoryActivationFrame` 也在该 wrapper 内创建和销毁，所以 `FactoryParameter<'frame>` 的
   不可逃逸约束保持不变。
3. `Arena` 的私有 backing 用 `Arc` 保留稳定的 `Box` 服务值、索引、逆序析构 journal、Transient
   child ownership 与 cleanup 元数据；公开的 Arena façade 和容器仍是线程封闭的。`ArenaLease`
   在 worker 还可能读取 token 时保活整个可见 Arena 闭包，不能靠新增 `unsafe impl Send/Sync`
   把原 Arena 直接跨线程移动。lease 以外到内采集、以内到外释放，所以取消 worker 后当前
   scope/Transient backing 会先于祖先 Scoped 或 Singleton backing 析构。
4. worker 成功后，协调器才提交结果并解锁消费者。持久服务进入其 Singleton/Scoped Arena；
   transient occurrence 先由协调器暂存，随后随直接消费者提交转入 child Arena。仍只保证
   “依赖先提交、消费者后提交”。

`shutdown()` 在执行 cleanup 前等待相关 lease 清空，再保持既有的“hook → 服务 Rust Drop →
Transient child shutdown”逆序。这样用户不需要把 `Inject<T>` 改成 `Arc<T>` 或修改宏生成代码，
但 Tokio `abort` 后仍不会让尚在退出的 worker 持有悬垂 token。

---

## 四、v2 类型化 Scoped Arena

v2 将一次应用构建与每次 scope 构建分开：

1. `ScopeProvider::build*` 先按依赖后序激活并提交可达 Singleton 到父 Arena。它们由
   AppRoot 和静态 Scope 链所有 ScopeRoot 的并集共同决定，因此不会要求 AppRoot 人为注入
   所有未来 scope 所需的单例。
2. 每次 `create_scope*` 创建独立 Scoped Arena，只激活 ScopeRoot 可达的 Scoped 节点及其
   Scoped-owner Transient occurrence。构造输入按编译图标记，从父 Singleton Arena、当前
   Scoped Arena 或对应 transient arena 准备。
3. Scoped 服务可安全持有 Singleton、同 scope Scoped 或由自身持有的 Transient `Inject<T>`；
   父 Singleton 绝不能持有子 scope 服务，也不能经由其 Transient 子树捕获它。scope 释放时，
   Scoped Arena 按提交逆序析构，而父 Arena 仍因借用关系存活。

异步 scope 激活复用 v6 的 Tokio ready queue：独立 factory 可以在 multi-thread runtime 上并行；
current-thread runtime 则安全地退化为同线程并发。每个 worker 只持有预绑定
`ConstructionContext`、provider 调用项与 lease；局部 `FactoryActivationFrame` 在 worker 内创建，
两个 Arena 仍只由协调器在 prepare、commit 与 rollback 边界管理。

---

## 五、v5 静态多层 Scope 链

`ScopeProvider<AppRoot, Chain>::build*()` 将单层模型推广到在类型中从外到内写出的线性链，
但不改变 Scope 的静态边界：

1. Compiler 在同一会话中依次编译 AppRoot、链头 root 与所有 child root；全部可达
   Singleton 仍只在 provider 构建时激活一次。已物化的闭合泛型结果也在这次会话中复用。
2. Scoped token 在首次可达的 `ScopeFrameId` 固定 owner。后代层的输入直接记录该祖先 frame，
   因此同一父 scope 创建的多个 child 会复用父 Scoped 实例；每个 child 自己首次拥有的
   Scoped token 则在每次 child scope 创建时各自构造。
3. 每一层 `Scope` 自己拥有一个 Arena，只借用 Singleton Arena 与全部祖先 Scope Arena。
   child 的借用链让它必须先结束，父 scope 才能 shutdown 或析构；同一父 scope 可以创建多个
   sibling child，它们共享祖先、隔离各自层拥有的实例。Scope Arena 不复用 transient child
   Arena 机制。
4. ScopeRoot 必须在自己的层首次获得所有权。链中重复 root、外层 Scope 的闭包提前拥有后代
   root，或外层经 transient 子树捕获已声明为后代 ScopeRoot 的 Scoped 服务，都会在编译时
   拒绝；不会在创建 child 时隐式重新选择 provider 或补建祖先实例。
5. 同步 `ScopeProvider::build()` 会拒绝整条链可达的 async factory 与 cleanup。异步构建与
   异步建 scope 允许二者；由异步 provider 建成后，某层同步 `create_scope()` /
   `create_child_scope()` 只检查该层的调度图，而不是重新拒绝链中别层的 async factory。

Transient 的持久 owner 也绑定到精确 ScopeFrame：后代可以消费祖先 Scoped 与继承本层 owner
的 Transient，祖先及其 transient 子树不能捕获已声明为后代 ScopeRoot 的 Scoped 服务。
factory 参数 transient 的临时 Arena 与 cleanup 拒绝规则不变。

---

## 六、v3 cleanup 与 v4 Transient 所有权

cleanup hook 维持宏既有 ABI：零参数 `fn() -> CleanupFuture`，其 future 输出为 `()`。
它不是普通 `Drop` 的替代品，只有调用方显式消费容器并 `.await shutdown()` 才会被驱动。

1. Arena 按提交逆序逐项处理：先 await 当前服务的 hook（如有），再析构该服务，最后释放
   存储。因此消费者会先于其依赖 cleanup；独立兄弟的相对顺序不构成合同。
2. 每个 transient occurrence 都有私有 child Arena。普通 Drop 时先析构 consumer，再 Rust
   Drop 其 transient 子树；显式 shutdown 时按“consumer hook → consumer Drop → transient
   child shutdown”的顺序递归处理。由 factory 参数临时持有的 transient 不属于此 shutdown 链。
3. `Scope::shutdown` 只处理当前 Scoped Arena；在线性链中必须按 child → parent →
   Singleton provider 的顺序结束。父 scope 或父 Singleton 的 hook 不会由 child shutdown
   隐式运行。`ScopeProvider` 被 shutdown 时只在不存在活跃 scope 的借用前提下清理其
   Singleton Arena。
4. 若 shutdown future 被取消，当前及剩余 hook 都不保证执行；Arena 仍必须按普通 Rust
   析构逆序释放全部服务，不能留下悬垂 token 或泄漏存储。

---

## 七、失败、取消与 Arena 生命周期

- 观察到构造失败后，协调器停止加入新的就绪节点，并收口已经开始的 Tokio worker；完成收口后按
  稳定编译图 rank 选择一个带 provider 声明上下文的 `BuildError`。worker panic、意外取消与
  factory 失败都参与同一稳定选择，但不是调用方可匹配的公开 error variant。
- factory 返回的用户 `Err(E)` 不会作为 `BuildError` source 向外泄漏。
- 任一激活失败时，已提交实例由 `Arena` 按提交逆序执行 Rust 析构；已完成但尚未移交的
  transient occurrence 与未提交结果也只做 Rust Drop。失败回滚不运行 cleanup hook。
- 若调用方在完成前丢弃 `build_async()` future，session 会停止调度并 abort 在飞 worker。
  Tokio abort 不等待 worker 实际退出，因此 lease 会继续保活其可见 Arena backing，直到最后一个
  worker 退出后才执行普通 Rust 回滚析构；取消不运行 cleanup，也不会留下悬垂 `Inject` token
  或部分构建的 `ServiceProvider`。
- scope 或 child scope 构建失败，或 `create_scope_async()` / `create_child_scope_async()` 被取消时，
  只回滚当前层的 Scoped Arena；已经成功构建的祖先 Scope 与父 Singleton 均保留，可供下一次
  同层 scope 创建使用。取消 activation 或普通 `Drop` 同样只执行 Rust 析构，绝不隐式启动
  cleanup future。

这些规则使协调器独占 Arena 的 prepare、commit 与 rollback，lease 只延长 worker 所见 storage
的存活期，并保留“依赖先于消费者提交”所需的析构关系。

---

## 八、验证矩阵

| 目标 | 覆盖方式 |
| --- | --- |
| 异步入口 | async-factory Root、混合同步依赖、trait/key 投影、optional 与泛型 fallback |
| Tokio runtime | 缺失 runtime 的结构化错误、current-thread 安全退化，以及 multi-thread worker 的无时序假设重叠 |
| 调度关系 | 两个无依赖构造可在不同 Tokio worker 执行；消费者仅在所有直接依赖提交后启动 |
| 失败收口 | 多个构造失败、worker panic/取消均按稳定图 rank 选错；已提交实例逆序析构 |
| 取消与 lease | 丢弃未完成的 root/scope future 会 abort worker；lease 保活 backing 至其退出，且 shutdown 等待 lease 清空 |
| 同步回归 | `build()` 仍拒绝 async factory；现有 factory-parameter 逃逸 UI 用例仍编译失败 |
| 类型化 scope | Singleton AppRoot + Scoped ScopeRoot；重复创建 scope 时共享单例、隔离 Scoped 实例 |
| 静态 Scope 链 | 三层 `ScopeLayer` 中全局 Singleton 共享、父 Scoped 被 child 复用、同父 sibling 与不同父分支正确隔离 |
| 链根所有权 | 重复 root、祖先闭包已拥有后代 root、祖先经 Transient 捕获已声明后代 ScopeRoot 的 Scoped 服务都给出结构化错误 |
| Transient occurrence | 同一/不同 consumer 的同 token 注入各自实例化；key、trait、optional 与泛型选择保持原语义 |
| 生命周期边界 | Scoped 可注入 Singleton/Scoped/Transient；Singleton 注入 Scoped 或经 Transient 捕获 Scoped 明确失败 |
| scope 回滚 | scope 失败或取消只回滚其 Scoped Arena，父单例仍可用于下一次 scope |
| async scope | independent async factory 可跨 Tokio worker 并行；消费者在所有输入 commit 后启动 |
| async child scope | 每层 sync/async factory 与 Tokio 就绪调度均覆盖；失败或取消仅回滚当前 child Arena |
| 应用 API | `ServiceProvider` 的 `root()` / `get<T>()` 只读取已建闭包；Scope 查询遵循当前层、精确祖先层、Singleton 的静态可见性 |
| 封装边界 | 下游不能导入 `registration`、`lifetime` 或低层错误；宏展开与手写 Provider ABI 只走 `__private` |
| 显式 shutdown | async root/scope 接受 cleanup；hook 与析构均按反向 commit 顺序执行 |
| cleanup 隔离与取消 | scope shutdown 不影响父 Arena；普通 Drop、激活失败、activation/shutdown 取消不保证运行 hook，且仍析构全部服务 |
| transient cleanup | 字段持有的 transient 仅 async 入口接受并在 owner shutdown 时执行；factory 参数 transient 子树含 cleanup 明确编译失败 |

验证命令保持 workspace tests、示例运行、fmt、clippy 与 `git diff --check`。

---

## 九、v6 仍明确不做

- Transient Root、lazy `Provider<T>`、每次解引用创建、cleanup 重试/超时/后台补偿、公开 lease API、预热和动态注册；
- 任意 ChildRoot 的运行时查询、分支型 Scope 树、通用 service locator 或 scope 间服务传递；
- `spawn_blocking`、跨容器动态调度、容器或 Scope 的 `Send`/`Sync` 生命周期承诺；
- `nestrs-bootstrap` 或任何反向依赖到组合层的边界泄漏。
