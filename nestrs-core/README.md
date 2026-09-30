# nestrs-core

`nestrs-core` 是 Nestrs 的依赖注入容器与运行期基础库。它把服务声明编译成一张经过验证的静态依赖图，再由 Tokio 调度器按图构造、共享和清理实例。

核心流程是：**编译时收集类型化声明 → 容器构建时验证完整依赖图 → 冻结构造计划 → 按需或提前实例化 → 等待异步关闭**。依赖图分析、构造任务展开、失败传播和最终实例释放都采用非递归算法。

本文介绍当前实现的设计和边界。业务声明语法的完整说明见[宏使用指南](../docs/NESTRS_MACROS.md)，完整业务项目见[结账示例](../example/di-checkout/README.md)。

## 1. 库的定位与工具链边界

| 组成 | 职责 |
| --- | --- |
| `nestrs-core` | 服务描述、图验证、生命周期、实例构造调度、查询与关闭 |
| `cargo-nestrs` | 应用构建、声明代码生成、基于真实 Rust 类型的自动 binding、编译器注册清单、编辑器支持与 HTML 依赖图 |
| 工具内部的 `nestrs-tool-bridge` | 标准过程宏薄桥接，复用工具中的代码生成逻辑；不是应用需要声明的依赖 |
| 未来的 `nestrs-bootstrap` | 在 core 之上组合应用、配置和生态库；当前尚未提供 `NestrsFactory` |

core 不依赖 CLI、代码生成库或 rustc 内部库，应用运行时也不链接这些工具实现。当前直接依赖为 Tokio、thiserror 和 serde_json。

应用开发仍需要 Nestrs 工具链：`#[injectable]`、`#[factory]` 的展开、trait 自动绑定、查询根收集和跨 crate 注册汇总均由 `cargo nestrs` 管理。应用的 `Cargo.toml` 依赖 core 与业务库；`use nestrs::{injectable, factory};` 中的 `nestrs` 是工具注入的 extern 名称，无需增加公开宏 package。

core 自身可以用普通 Cargo 检查和测试；使用 Nestrs 声明的应用通过 `cargo nestrs check/build/run/test` 构建运行。当前没有公开的手动注册门面、运行时扩图接口或单独的 `register!` 宏。

## 2. 三个不同层级的保证

| 阶段 | 检查内容 | 失败表现 |
| --- | --- | --- |
| Rust 编译 | 类型、trait 投影、借用、构造 adapter、`Send` / `Sync` 等约束 | 编译错误 |
| `ServiceProvider::build().await` | 全部注册及可物化闭合类型的缺失依赖、歧义、循环、输入协议与生命周期 | 在任何服务实例化之前 panic |
| 实际初始化 | constructor / factory 执行，以及数据库连接等外部资源操作 | 返回初始化错误；构造 panic 被 worker 捕获并转为错误 |

因此，`cargo nestrs check` 或 `cargo nestrs build` 成功不代表已经执行容器的全图校验；图结构合法也不代表外部资源一定能初始化成功。默认 Lazy 只把第三个阶段推迟到查询时，**不会推迟或跳过第二个阶段**。

全图验证只处理服务描述，不执行 constructor、factory、`Default`、`#[value]` 表达式或 cleanup。一个从未被查询的已注册服务存在图错误，也会使容器构建失败。

## 3. 从声明到使用

下面是一个最小的订单服务。仓储由异步 factory 创建，业务服务通过 trait 接收它；普通 `impl OrderStore for MemoryOrderStore` 由工具链按实际需求参与自动绑定。

```rust
use nestrs::{factory, injectable};
use nestrs_core::{ResolveError, ServiceProvider, get_required_service};

trait OrderStore: Send + Sync {
    fn count(&self) -> usize;
}

struct MemoryOrderStore;

impl OrderStore for MemoryOrderStore {
    fn count(&self) -> usize {
        3
    }
}

#[factory(lifetime = Singleton, cleanup = "cleanup_store")]
async fn create_order_store() -> MemoryOrderStore {
    // 实际项目可以在这里异步建立数据库连接。
    tokio::task::yield_now().await;
    MemoryOrderStore
}

async fn cleanup_store() {
    println!("订单仓储 cleanup 完成");
}

#[injectable(lifetime = Scoped)]
struct CheckoutService {
    #[inject]
    orders: dyn OrderStore,
}

impl CheckoutService {
    fn order_count(&self) -> usize {
        self.orders.count()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::build().await?;
    let scope = provider.create_scope();

    // 保存操作结果，再执行关闭，让查询失败路径也能等待异步清理。
    let outcome: Result<usize, ResolveError> = async {
        let service =
            get_required_service!(scope.service_provider(), CheckoutService).await?;
        Ok(service.order_count())
    }
    .await;

    let scope_close = scope.dispose_async().await;
    let root_close = provider.dispose_async().await;

    println!("订单数量：{}", outcome?);
    scope_close?;
    root_close?;
    Ok(())
}
```

该程序需要在应用依赖中启用 Tokio 的 `macros` 和 `rt-multi-thread` feature，并通过 `cargo nestrs run` 执行。cleanup 保留无参数异步函数形式：每个成功发布的实例调用一次所配置的 hook，不会自动把实例作为参数传入。

上例会等待两级关闭，再传播错误；若操作和关闭同时失败，它只返回首先传播的错误。真实项目可以像[结账应用的生命周期入口](../example/di-checkout/src/application.rs)一样聚合操作错误与关闭错误。

### 公开门面

| API | 语义 |
| --- | --- |
| `ServiceProvider::build().await` | 使用入口项目的启动配置，验证并冻结图，返回 `Result<ServiceProvider, BuildError>`；未配置时为 Lazy / 32 |
| `ServiceProvider::build_with_options(options).await` | 用代码中的完整选项覆盖项目启动配置 |
| `provider.create_scope()` | 同步创建 scope，本身不实例化服务 |
| `scope.service_provider()` | 返回轻量的 `ServiceProviderRef<'_>` 查询视图 |
| `scope.warm_up().await` | 预热当前 scope 的全部 Scoped 服务及必要依赖 |
| `LazyInjection<T>::get().await` | 获取注入字段的固定延迟目标，返回借用句柄的 `Result<&T, ResolveError>` |
| `provider.dispose_async().await` | 消费 root，等待已接受工作、scope 关闭和 root 清理 |
| `scope.dispose_async().await` | 消费 scope，等待该 scope 的工作与清理 |

服务查询只通过以下四个宏进行；接收者可以是 root 或 scope 的查询视图：

| 查询宏 | 返回类型 |
| --- | --- |
| `get_required_service!(provider, T).await` | `Result<&T, ResolveError>` |
| `get_service!(provider, T).await` | `Result<Option<&T>, ResolveError>` |
| `get_required_keyed_service!(provider, T, key).await` | `Result<&T, ResolveError>` |
| `get_keyed_service!(provider, T, key).await` | `Result<Option<&T>, ResolveError>` |

`T` 可以是 concrete 类型或可绑定的 `dyn Trait`。key 表达式使用 `ServiceKey::Named(String)` 或 `ServiceKey::Indexed(usize)`。可选查询只有在精确类型与 key 未注册时返回 `None`；初始化失败、scope 限制、owner 已关闭等仍然返回错误。

返回引用借用实际 root / scope owner，不借用临时的 `ServiceProviderRef`，因此支持示例中的链式调用。服务引用仍将被使用时，Rust 借用检查会拒绝消费对应 owner 来执行 disposal。

## 4. 服务描述与编译器注册清单

运行时需要的描述由 [registration](src/registration/mod.rs) 定义，主要包括：

| 描述 | 用途 |
| --- | --- |
| `Provider::Class` | 通过生成的 class adapter 构造服务 |
| `Provider::Factory` | 通过同步或异步 factory adapter 构造服务 |
| `TraitBinding` | 把已有 concrete 实例投影为 trait 视图，不额外创建实例 |
| 查询根 | 记录具体查询类型，以及可用的闭合类型描述回调 |
| 自动 binding 与闭合蓝图目录 | 保存可被实际根或依赖需求启用的能力，不直接等同于显式注册 |

编译器在每个 binary / test 入口汇总本 crate 与依赖 metadata 中的真实描述回调，生成唯一的版本化注册入口。core 每次构图从中取得自己拥有的 `RegistrySnapshot`。

这里不依赖 linkme、inventory、链接段扫描、全局构造器或可变的全局注册表。不同入口各自拥有注册集合，库的描述不会自动构成一个独立运行的容器。

typed adapter 引用 core 实际所属的私有模块。driver 根据生成来源授权访问，普通业务源码不能借此访问内部协议；core 不导出 `__private` 或换名后的公开内部 ABI。生成的类型投影和借用继续接受 Rust 的正常检查，不伪造 vtable 或绕过业务类型的可见性。

跨 crate 收集与自动绑定的具体覆盖范围见[跨 crate DI 说明](../docs/NESTRS_CROSS_CRATE_DI.md)，编译器侧实现见[编译器适配说明](../docs/NESTRS_COMPILER_ADAPTER.md)。

### 为什么查询使用宏

图在 `build` 成功后冻结，运行时查询不能再物化新类型。查询宏除了发起查询，还在具体类型调用处贡献静态根，使 `Repository<User>` 这样的已知闭合泛型在构图阶段就能进入图。

这意味着：

- 已编译但尚未执行的查询分支仍然贡献根；被 `cfg` 排除的调用点不贡献根。
- 多个调用点或依赖触达同一个闭合类型是幂等的，不会因此重复注册。
- 泛型展开只处理已知的有限需求，不枚举任意泛型实参组合。
- 查询类型不能捕获外层泛型参数、const 参数或 impl 的 `Self`；这些位置需要使用具体类型或闭合类型别名。
- provider 和动态 key 表达式各在查询时求值一次；动态 key 只选择冻结路由，不新增注册或改写声明中的 key。

方法体内查询也只贡献查询根，不会自动把方法调用关系变成该服务的构造依赖。全图验证针对声明的激活依赖，不分析任意业务方法的运行时行为；建议在应用边界获取服务，由字段或 factory 参数表达业务协作关系。

## 5. 依赖图如何编译和冻结

[GraphCompiler](src/graph/compiler.rs) 按四个阶段处理快照，成功后交付不可变的 `ValidatedGraph`：

```text
RegistrySnapshot
    │
    ├─ expand：工作队列展开闭合类型，按需求启用描述能力
    ├─ routes：选择精确类型 / key 对应的 concrete 与 trait 路由
    ├─ inputs：确定每个输入槽位的目标、可选缺席状态与 typed preparer
    └─ topology：拓扑顺序、真实环路径与生命周期能力
    │
    ▼
ValidatedGraph：provider ID、输入计划、反向依赖、构造入口与查询路由
```

图编译器校验全部有效注册，而非只检查某次查询经过的路径。候选选择有明确的边界：

- 同一 concrete 类型、同一 key 出现多个显式 provider 是错误，`primary` 不能覆盖它。
- trait 候选按请求 key 精确匹配；一个候选直接选中，多个候选要求恰好一个 `primary`。
- 默认 key 不回退到 named / indexed key；同类型不同 key 分别处理。
- 精确 type / key 已有显式 provider 时，闭合蓝图不会替换它。
- 重复显式 binding、展开后仍无对应 concrete provider 的 binding 都是错误。
- optional 无候选时固化为缺席输入；已有候选仍要通过歧义、循环及生命周期检查。
- 输入槽位必须连续，描述组合必须合法；字段和 factory 参数都参与依赖检查。

拓扑分析使用 Kahn 算法生成依赖优先顺序；若有残余节点，用显式栈定位实际环，诊断包含字段 / 参数、key 与源码位置。排序和诊断不依赖注册收集顺序。

拓扑计数可以合并重复边，**实际输入槽位必须保留**。例如一个消费者的两个字段都注入同一种 Transient，运行时仍要为两个槽位分别构造实例。

冻结后不再读取注册清单、不物化泛型、不修改路由。未进入图的类型或 key 按未注册处理。

## 6. 生命周期与预热

| 生命周期 | 共享边界 | 实例语义 |
| --- | --- | --- |
| `Singleton` | root / provider | 同一 root 的所有 scope 共享；始终在 root 上下文构造 |
| `Scoped` | scope / provider | 同一 scope 共享，不同 scope 隔离；root 不能直接解析 |
| `Transient` | 每次消费 occurrence | 每次查询、每个注入槽位独立构造，不进入共享实例缓存 |

Singleton 允许依赖 Transient，但整个依赖闭包不能包含 Scoped，包括经过 Transient 的间接依赖、延迟字段以及 factory 参数。Transient 可以依赖 Scoped，此时它被标记为需要 scope，root 查询会返回生命周期错误。

Transient 不进入共享缓存，不意味着查询返回后立即释放。所有成功发布实例都会进入所属 owner 的 journal，至少保留到 owner 清理；逃逸的强 lease 还可能延长最终内存存活期。

### 在 Cargo.toml 设置启动默认值

在应用 package 自己的 `Cargo.toml` 中使用顶层 `[nestrs-cli]` 配置节：

```toml
[nestrs-cli]
initialization = "lazy"           # "lazy" 或 "eager"，缺省为 "lazy"
max-concurrent-activations = 32    # 正整数，缺省为 32
```

之后直接调用 `ServiceProvider::build().await`，即可使用项目配置。该节不放在
`[package]` 或 `[package.metadata]` 下面。Cargo 原生不识别这个自定义顶层节，会提示
`unused manifest key: nestrs-cli`；实际读取和校验由 `cargo nestrs` 管理。

配置在编译期间读取，并随当前 binary / test 的注册入口固化。core 运行时不解析 TOML，
可执行文件离开源码目录后仍使用同一份配置。修改配置后需要重新构建；工具链把 manifest
加入编译依赖跟踪，避免仅修改配置后继续复用旧产物。

每个入口使用所属 package 的配置，同一 package 的多个 binary 共用该配置。不自动继承
workspace 根或依赖 package 的配置；即使 `build()` 写在业务依赖库中，实际默认值也来自
最终应用入口。未知配置键、无效初始化模式、零或非整数并发数在编译阶段报错，不静默回退。
配置不会改变 `cargo nestrs graph` 只验证结构、不执行构造的契约。

### 代码显式覆盖

需要程序化控制时，仍可使用既有选项：

```rust
use std::num::NonZeroUsize;
use nestrs_core::{InitializationMode, ServiceProvider, ServiceProviderOptions};

let provider = ServiceProvider::build_with_options(ServiceProviderOptions {
    initialization: InitializationMode::Eager,
    max_concurrent_activations: NonZeroUsize::new(16).unwrap(),
})
.await?;
```

优先级为 **`build_with_options` 的完整显式选项 > `[nestrs-cli]` 项目配置 > Lazy / 32 基线**。
`ServiceProviderOptions::default()` 始终表示库的 Lazy / 32 基线，不读取项目配置。
例如 `build_with_options(ServiceProviderOptions::default())` 会显式覆盖项目中设置的 Eager；
希望使用项目配置时应调用 `build()`。

- 默认 `Lazy`：完整验证图，查询时才构造对应依赖闭包。
- `Eager`：在 build 返回前初始化全部 Singleton 及必要依赖，不是实例化所有服务。
- `scope.warm_up().await`：初始化该 scope 的全部 Scoped 及必要依赖；创建 scope 本身不构造。

两种初始化模式共用同一张图和同一个调度器，预热不会改变共享边界或把 Transient 变为缓存实例。

### 字段级延迟注入

全局 Lazy 决定何时开始构造某个服务；字段上的 `#[lazy]` 则允许消费者先完成构造，
把一个依赖留到实际业务分支使用时再获取：

```rust
#[injectable]
struct OrderService {
    #[inject]
    #[lazy]
    reports: ReportService,
}

impl OrderService {
    async fn export_report(&self) -> Result<String, nestrs_core::ResolveError> {
        let reports = self.reports.get().await?;
        Ok(reports.generate())
    }
}
```

工具把字段改写为 `LazyInjection<ReportService>`；普通注入仍为 `Injection<T>`。
`LazyInjection<T>` 支持具体类型、`dyn Trait`、key 和闭合泛型；optional 形式改写为
`Option<LazyInjection<T>>`，注册不存在时直接是 `None`。它不提供同步 `Deref`，
不会把异步初始化隐藏在普通同步方法调用里。

图保存目标选择、输入投影和延迟属性。所有边仍参与 build 时的缺失依赖、歧义、循环与
生命周期检查，只有激活任务展开会跳过延迟边。调用 `get()` 使用已冻结的目标计划，
不重新读取注册或展开类型。全局 Eager 与 scope 预热仍会选择各自生命周期的全部目标；
延迟字段不豁免目标自身的预热，也不阻止其他普通注入提前创建它。

每个字段拥有固定的获取状态：并发首次访问合并，成功实例与初始化错误都保留在该
句柄中，取消等待不会重复提交。Singleton / Scoped 目标仍复用所属 owner 的缓存；
Transient 只在该字段内部复用，不改变不同消费槽位相互独立的语义。

当前标记只用于 `#[injectable]` 的注入字段，不支持 provider 级初始化策略或 factory
参数上的 `#[lazy]`。构造 worker 的同任务调用链不能首次初始化延迟字段，以免构造名额
被互相等待的任务耗尽；此时返回 `ResolveError`，业务阶段后续访问仍可正常初始化。
这个任务阶段标记不会自动传递到用户新建的 `tokio::spawn` / `spawn_blocking` 任务，
因此 factory 也不能通过派生任务等待未初始化的延迟字段。

## 7. 非递归的 Tokio 调度

每个 root 启动一个中央协调任务，管理所有 scope、共享初始化状态、就绪队列、活跃任务和 worker。门面通过命令通道提交请求，worker 只构造一个依赖已经就绪的节点，不递归调用 resolver。

一次查询大致经过以下步骤：

1. 查找冻结路由与对应 owner 的缓存。Singleton / Scoped 已成功或失败时直接复用结果，正在构造时共享同一任务。
2. 通过显式工作队列展开必要的构造 occurrence；Transient 为每次消费创建独立任务。
3. 把依赖就绪的任务放入就绪队列，在构造并发上限内启动 Tokio worker。
4. worker 返回后，协调器先把成功实例的强 lease 写入 owner journal，再更新缓存、通知等待者并推进消费者。
5. 完成的任务移出活跃任务表；长期成功 / 失败结果留在独立的生命周期缓存中。

```text
活跃任务：Unexpanded → Waiting → Queued → Running → 完成并移除
共享缓存：无缓存 → Building(TaskId) → Ready(lease) 或 Failed(error)
```

默认整个 root 最多运行 **32 个构造任务**，所有 scope 和并发查询共享上限；通过 `max_concurrent_activations: NonZeroUsize` 配置。等待依赖的任务不占名额，节点完成就立即推进可运行的后继，没有等待整层完成的屏障。该上限不控制业务方法自身的执行并发。

core 使用应用当前的 Tokio runtime，不创建嵌套 runtime。当前线程 runtime 支持异步并发推进，多线程 runtime 可以并行执行 worker。短同步构造直接在 worker 内执行；阻塞初始化需要用户工厂显式安排，例如使用 `spawn_blocking`，框架不会自动迁移所有同步构造。

## 8. 稳定地址、强 lease 与真实借用

DI 容器返回引用且支持异步初始化，安全性需要同时回答：实例放在哪里、谁保活它、依赖何时释放、factory 的参数实际借用了谁。

### 实例与注入令牌

实例存放在稳定地址的 Box 中，发布后不移出载荷；发布前核对构造结果的实际类型与图声明。`Injection<T>` 以只读 `Deref` 提供业务访问，内部持有真实强 lease，没有公开构造、`Clone`、`Copy`、可变访问或裸指针导出。

各持有者的责任不同：

| 持有者 | 责任 |
| --- | --- |
| owner journal | 保活查询返回的引用，记录成功发布时间和 cleanup 顺序 |
| Singleton / Scoped 缓存 | 合并初始化，保留成功实例或共享失败 |
| `Injection` / `ErasedServiceRef` / 已完成的 `LazyInjection` | 保活令牌指向的实例，即使安全代码将令牌移出消费者 |
| factory frame | 为异步 factory 参数提供跨 await 的真实借用来源 |
| 实例保存的依赖 leases | 保证消费者析构期间，其必要依赖仍然存活 |

journal 位于门面与协调器共享的 owner 数据中，存活不依赖协调任务是否还在运行。只有确认实例已被 journal 收纳，门面才能把经过类型检查的内部指针恢复成借用 owner 的 `&T`。trait 查询复用真正的 typed coercion，并核对投影仍指向同一个实例。

当前可调度服务需要满足 `Send + Sync + 'static` 约束。服务类型不得携带非 `'static` 的外部借用；这不表示实例永远不释放。

### 构造输入与 factory frame

`ActivationPreparation` 统一执行“准备完整令牌 → 写入空槽位 → 收纳实际令牌的 lease”。准备或写入失败时回滚，类型与 required / optional 形态不匹配时不会提前取走原槽位。

class adapter 把带 lease 的注入令牌移入字段。factory adapter 则借用 worker 内创建的真实 `FactoryLeaseFrame`，在该 frame 存活期间等待 factory future；不通过延长外部引用或伪造 `'static` 借用完成异步调用。

延迟字段交付的是句柄，其目标尚不存在时没有伪造地址。第一次获取成功后，typed
projection 与强 lease 一起固定，返回引用借用实际句柄。句柄只弱引用 owner 和请求通道，
避免形成“owner journal → 消费者 → 延迟字段 → owner”的强引用环。

## 9. 失败、取消与关闭

### 失败如何传播

| 情况 | 行为 |
| --- | --- |
| 图结构非法 | 公开 build 入口执行时 panic；此前不执行服务构造 |
| 缺少当前 Tokio runtime | 返回 `BuildError::RuntimeUnavailable` |
| Eager 初始化失败 | 关闭尚未交付的 root，处理已接受任务并清理成功实例，再返回 `BuildError`；可同时保留关闭错误 |
| 查询、生命周期或构造失败 | 返回 `ResolveError`；初始化诊断保留 provider、key、源码与依赖路径 |
| cleanup panic | 记录失败、继续清理其他实例，最终聚合为 `DisposeError` |

Singleton / Scoped 的初始化失败缓存到所属 owner 关闭，后续查询不会自动重试。Transient 的失败只属于本次 occurrence，下次请求可以创建新的 occurrence；一个 `LazyInjection` 字段固定持有同一次 occurrence，因此该字段反复 `get()` 不会重试失败。普通前置依赖失败时，消费者不会开始构造；延迟目标失败发生在消费者已构造之后，由调用 `get()` 的业务代码处理。其他请求可能使用的成功共享实例不会被连带清理。

factory 返回 `Result<T, E>` 时要求 `E: Debug`，同步和异步 adapter 都保存错误诊断文本，不承诺保留完整的 `Error::source` 链。constructor / factory panic 通过受跟踪的 worker 结果进入初始化错误，协调器继续运行。

### 取消等待与取消初始化分开

查询 future 被丢弃，只取消等待。协调器已经接受的初始化继续执行，成功实例仍由 owner 收纳。关闭等待被取消，也不会取消已经启动的关闭流程。

这个契约让共享初始化不受单个请求取消影响，也使 cleanup 有明确归属。

### 关闭顺序

每个 owner 的状态单向推进：

```text
Open → Draining → Cleaning → Closed
```

1. 拒绝新解析，继续处理已接受的工作。
2. 工作排空后，按冻结图中的依赖关系安排实例清理，消费者先于依赖。
3. 每个 owner 同时最多运行一个 cleanup worker，上一项 hook 和该次触发的释放处理完成后才推进下一项。
4. root 等待全部 scope 关闭，再完成自己的清理。

普通依赖先于消费者发布，而延迟目标可能晚于消费者发布。因此关闭不能只依赖发布时间
倒序，还要遵守图中的消费者优先关系。没有延迟边时仍使用逆发布时间；存在延迟边时，
使用全图的反向 Kahn 调度（没有实例的 provider 也要传递依赖约束），在当前允许清理的
实例中优先选择最晚发布的一项。配置的 hook 对每个成功发布实例调用一次；
未实例化的延迟目标不执行 cleanup，未成功发布的值不按成功实例执行该 hook。
普通 Rust 值的析构仍遵循其所有权。

`dispose_async(self)` 消费 owner 并等待上述流程。普通 `Drop` 只向已有协调器发送幂等关闭请求，不阻塞、不临时启动 runtime、不调用 `block_on`。有活跃 runtime 时尽力执行异步清理；runtime 已退出时只保证同步安全释放，不能保证异步 cleanup 完成。因此，需要确认 hook 已完成的应用应显式 await disposal。

### 逻辑关闭与最终内存释放

逃逸的注入令牌不会阻止逻辑关闭，但会延长实例及必要依赖的内存存活期。cleanup 已执行后，不再承诺这些服务仍具备业务可用性。

owner 开始关闭后，尚未初始化的 `LazyInjection` 拒绝新的初始化；此前已接受的初始化
继续排空。已成功获取的延迟句柄仍以强 lease 维护内存，即使消费者在 `Drop` 中将它
移出，关闭后也不会得到悬垂引用。

最后一个实例 lease 释放时，载荷进入独立于 Tokio 的 `ReleaseDomain` 队列，由循环销毁。释放消费者导致更多依赖 lease 归零时，继续入队处理，避免深层依赖沿嵌套 `Drop` 再次形成递归。队列锁不包围用户析构代码。

当前没有自动超时或强制终止策略。不返回的 factory / cleanup 会使显式关闭持续等待；构造并发上限不提供终止保证。

## 10. 源码阅读路线

下面是职责划分；内部模块不是业务公开 API。建议先从门面与图编译读起，再进入调度和内存安全细节。

| 位置 | 阅读重点 |
| --- | --- |
| [src/lib.rs](src/lib.rs) | 公开导出、模块边界 |
| [src/options.rs](src/options.rs) | 启动选项类型与库的基线默认值 |
| [src/facade.rs](src/facade.rs)、[src/query.rs](src/query.rs) | owner 借用、build / scope / disposal、查询根与查询宏 |
| [src/registration/catalog.rs](src/registration/catalog.rs)、[provider.rs](src/registration/provider.rs) | 构图快照、Provider 与描述协议 |
| [src/graph/compiler.rs](src/graph/compiler.rs) | 四阶段编排、诊断汇总与冻结 |
| [expand.rs](src/graph/compiler/expand.rs)、[routes.rs](src/graph/compiler/routes.rs)、[inputs.rs](src/graph/compiler/inputs.rs)、[topology.rs](src/graph/compiler/topology.rs) | 声明展开、候选选择、输入计划、非递归拓扑与生命周期 |
| [src/runtime/handle.rs](src/runtime/handle.rs)、[coordinator.rs](src/runtime/coordinator.rs) | 请求提交、唯一可变调度状态、依赖推进与发布 |
| [owner.rs](src/runtime/owner.rs)、[task.rs](src/runtime/task.rs)、[worker.rs](src/runtime/worker.rs) | owner 状态、缓存与活跃任务、单节点构造 / 清理 |
| [src/activation/construction/preparation.rs](src/activation/construction/preparation.rs)、[factory.rs](src/activation/construction/factory.rs) | 输入提交边界、工厂 frame 与真实借用 |
| [src/activation/instance.rs](src/activation/instance.rs)、[injection.rs](src/activation/injection.rs)、[release.rs](src/activation/release.rs) | 稳定实例、强 lease、迭代析构 |
| [src/error.rs](src/error.rs) | 公开错误、可共享的依赖失败路径及非递归显示 / 释放 |

底层 `activation` 提供构造输入、实例与 lease 能力；`registration` 用这些能力描述构造协议；`graph` 消费描述生成计划；`runtime` 组合图与 activation；`facade` 组合运行时和用户借用。activation 不反向依赖 registration，core 不反向依赖工具或未来的 bootstrap。

更聚焦于状态转换和维护约束的说明见[内部实现阅读指南](../docs/NESTRS_CORE_INTERNALS.md)。

## 11. 测试与依赖图诊断

所有 core 测试实现放在 crate 根目录的 [`tests/`](tests/README.md)：

- `tests/unit/`：图分析、调度、输入与 lease、深链释放、错误及门面借用等内部测试。生产模块只通过 `#[cfg(test)]` / `#[path]` 挂载，保留 Rust 模块原有私有边界。
- `tests/compiler/`：真实工具链生成声明的隔离契约，由 cargo-nestrs 的测试 harness 编译运行。
- 业务宏的黑盒集成和编译失败回归位于工具 package 的测试目录，core 不为测试反向依赖宏或 CLI。

从仓库根目录可以执行普通 core 检查：

```sh
cargo check -p nestrs-core --all-targets
cargo test -p nestrs-core
cargo clippy -p nestrs-core --all-targets -- -D warnings
cargo fmt --check
```

真实编译器契约还需要配套 driver 与工具环境，具体执行方式见[测试目录说明](tests/README.md)。已有重构验收的测试与平台范围见[验收记录](../docs/NESTRS_CORE_REFACTOR_VALIDATION.md)，不能把普通 core 单元测试视为完整工具链验证。

HTML 依赖图由 CLI 生成，例如在本仓库中执行：

```sh
cargo nestrs graph -p nestrs-di-example
```

core 只提供内部只读图描述和既有校验语义，HTML / CSS / JavaScript 及文件输出均归工具。graph 命令使用真实选定入口的注册集合，不执行业务 main 或服务构造；多入口保持各自节点和依赖边，不虚构跨入口的共享容器。使用方式见[工具链指南](../docs/NESTRS_CARGO_TOOLCHAIN.md)。
