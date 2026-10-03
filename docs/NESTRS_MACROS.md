# Nestrs 服务声明与使用指南

Nestrs 用属性声明服务、用字段或函数参数声明依赖，再通过容器的普通异步方法取得服务。
你只需要描述“这个服务怎么创建、依赖谁、活多久”，编译器验证依赖关系，容器管理实例。

本文面向编写 Nestrs 应用的开发者，从一个能运行的订单通知程序开始，介绍当前支持的
宏写法。工具链安装见[工具链说明](NESTRS_CARGO_TOOLCHAIN.md)，编辑器配置见
[项目初始化与 rust-analyzer 接入](NESTRS_IDE.md)。本文只维护当前用法与支持边界；
多轮缺陷、修复过程及各轮验证范围集中在[修复记录](NESTRS_FIXES.md)。

## 1. 先认识声明属性与查询入口

| 写法 | 用途 | 从哪里使用 |
| --- | --- | --- |
| `#[injectable]` | 声明服务，默认根据字段创建；也可由 `constructor` 指定构造函数 | `use nestrs::injectable;` |
| `#[factory]` | 用同步或异步函数创建服务 | `use nestrs::factory;` |
| `#[primary]` | 同一个接口有多个候选时，指定优先实现 | `use nestrs::primary;` |
| `#[inject]` | 结构体字段需要另一个服务；也可指定 factory / constructor 参数的 key | 声明内的辅助属性，不单独导入 |
| 服务上的 `#[lazy]` / `#[lazy(true)]` / `#[lazy(false)]` | 覆盖该服务的自主预热策略 | `use nestrs::lazy;`，与 `injectable` / `factory` 配合 |
| 字段上的 `#[inject] #[lazy]` | 将依赖改为首次 `get().await` 才获取目标的延迟句柄 | `injectable` 字段内的辅助属性，无需单独导入 |
| factory / constructor 参数上的 `#[lazy]` | 按值接收延迟句柄，可存入返回服务 | 参数内的辅助属性，无需单独导入 |
| `#[constructor]` | 为 injectable 服务选择同步构造函数，依赖写在参数上 | `use nestrs::constructor;`，标注 inherent impl 内的关联函数 |
| `#[value(表达式)]` | 为结构体中的普通字段提供初始值 | 声明内的辅助属性，不单独导入 |
| `get_required_service::<T>()` 等四个方法 | 从 root 或 scope 取得服务 | `ServiceProvider` 或 `ServiceProviderRef` 的普通方法 |

应用的 `Cargo.toml` 直接依赖 `nestrs-core`，无需添加 `nestrs-macro`、`nestrs-codegen`
或 `linkme`。`nestrs` 是 `cargo nestrs` 为应用提供的属性命名空间，不需要再添加一个
名为 `nestrs` 的 Cargo 依赖。

声明展开和最终入口编译会自动生成 `nestrs-reflect`，其中包括服务的私有类型化适配
代码与已验证执行计划。这是工具生成的应用产物，无需在 Cargo.toml 添加同名依赖、
导入额外模块或手工维护注册代码。日常开发仍使用本文的声明属性和普通查询方法；
想了解生成文件和职责边界时，可阅读[rustc 集成指南](NESTRS_RUSTC_EXTENSION_GUIDE.md)。

下面两种写法等价：

```rust
use nestrs::injectable;

#[injectable]
struct AppConfig;
```

```rust
#[nestrs::injectable]
struct AppConfig;
```

后文使用导入后的短名称。辅助属性也支持 `#[nestrs::inject]`、`#[nestrs::lazy]` 和
`#[nestrs::value(...)]`。字段 helper 不需要导入；不要单独导入 `inject` 或 `value`。
服务声明上的 `lazy` 是工具提供的属性宏，可以导入；它仍须与 `injectable` 或 `factory`
一起使用，不会独自把普通结构体或函数注册为服务。

服务类型、工厂函数和显式 constructor 方法支持 Rust raw identifier，例如
`struct r#type`、`fn r#type()`，业务引用保留正常的 Rust 名称解析语义。
自动 trait 绑定中的关联类型约束也保留合法拼写，例如
`dyn Port<r#type = [u8; 3]>`。

生成的局部绑定、内部模块和辅助项与业务名称隔离，自动字段构造、constructor、
factory 及自动 trait 绑定采用同一原则。业务无需避让 `error`、`service` 等常量名，
也无需为工具辅助项改名；`#[value(...)]` 和 constructor 函数体仍按原业务作用域、
类型上下文及求值顺序执行。实现细节见[生成名称与宏卫生](NESTRS_RUSTC_EXTENSION_GUIDE.md#generated-hygiene)。

## 2. 跑通第一个程序

假设你已经准备好匹配的 `cargo nestrs` 工具链，手动创建了一个 Rust binary 项目。
为它添加依赖；下面的 core 路径应按本机仓库位置调整：

```toml
[dependencies]
nestrs-core = { path = "../nestrs-framework/nestrs-core" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

把下面的完整程序保存到 `src/main.rs`：

```rust
use nestrs::{factory, injectable};
use nestrs_core::ServiceProvider;

#[injectable]
struct AppConfig {
    #[value("订单服务")]
    sender: String,
}

trait Notifier: Send + Sync {
    fn notify(&self, order_id: u64) -> String;
}

struct MailClient {
    sender: String,
}

// 参数由容器提供；输出的 MailClient 就是这个工厂注册的服务类型。
#[factory]
async fn mail_client(config: AppConfig) -> Result<MailClient, &'static str> {
    // 模拟异步初始化，不连接外部邮件服务。
    tokio::task::yield_now().await;
    Ok(MailClient {
        sender: config.sender.clone(),
    })
}

// 普通 Rust impl 即可，不需要 bind 标注。
impl Notifier for MailClient {
    fn notify(&self, order_id: u64) -> String {
        format!("{}：已通知订单 #{order_id}", self.sender)
    }
}

#[injectable(lifetime = Scoped)]
struct OrderService {
    #[inject]
    notifier: dyn Notifier,
}

impl OrderService {
    fn submit(&self, order_id: u64) -> String {
        self.notifier.notify(order_id)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::build(None).await?;
    let scope = provider.create_scope(None).await?;

    let orders = scope
        .service_provider()
        .get_required_service::<OrderService>()
        .await?;
    println!("{}", orders.submit(1001));

    scope.dispose_async().await?;
    provider.dispose_async().await?;
    Ok(())
}
```

在这个项目目录执行：

```sh
cargo nestrs doctor
cargo nestrs init
cargo nestrs check
cargo nestrs run
cargo nestrs graph
```

程序输出：

```text
订单服务：已通知订单 #1001
```

`init` 为手动组装的已有项目生成 rust-analyzer 项目模型和配置；使用 VS Code 时可
改为 `cargo nestrs init --vscode`，其他客户端按自己的方式加载生成配置。未来的
`cargo nestrs create` 会在创建项目时完成初始化，用户无需再执行 `init`；`create`
目前尚未实现。

初始化之后，编译、运行和测试仍使用 `cargo nestrs check/build/run/test`。
普通 `cargo check` 不提供 Nestrs 属性和自动绑定所需的环境。

这个程序的依赖顺序是：`OrderService → dyn Notifier → MailClient → AppConfig`。
这个未配置启动策略的程序使用 Lazy：编译器预先验证完整图，`build` 加载执行计划，
第一次取得 `OrderService` 时才创建所需实例。`graph` 会验证并导出静态依赖图，
不执行邮件工厂或业务入口。

### 2.1 在项目中设置启动默认值

可以在入口项目的 `Cargo.toml` 顶层添加：

```toml
[nestrs-cli]
initialization = "eager"
scope-initialization = "lazy"
max-concurrent-activations = 8
```

`initialization` 控制 root，`scope-initialization` 控制随后创建的 scope；二者各自接受
`"lazy"` 或 `"eager"`，缺省均为 Lazy，互不继承。构造并发上限必须为
正整数，未配置时为 32，整个 root 及其所有 scope 共用此上限。配置由工具在编译时
读取并写入当前 binary / test 的计划，运行时不读取 TOML，也不继承依赖库或 workspace
的同名配置。这里使用顶层 `[nestrs-cli]`，不是 `[package.nestrs-cli]` 或 metadata 节；
Cargo 可能提示未使用这个自定义节，Nestrs 仍会读取、校验它。

`ServiceProvider::build(None)` 使用这些项目默认值。需要根据运行参数控制容器时，使用
`ServiceProvider::build(Some(ServiceProviderOptions { ... }))`；显式选项完整覆盖
项目默认值。`ServiceProviderOptions::default()` 自身仍是库的 root Lazy / scope Lazy / 32 基线。
`provider.create_scope(None).await?` 使用当前 root 的 `scope_initialization`；
`create_scope(Some(ServiceScopeOptions { initialization: ... })).await?`
只覆盖本次 scope。None 使用项目或容器默认；Some 完整显式覆盖，
`Some(Default::default())` 明确选择库基线，不能当作 None。scope 创建也等待初始化
完成并可能返回 `ScopeBuildError`；
完整代码与失败清理见[core 初始化说明](../nestrs-core/README.md#6-生命周期与预热)。
这些都是全局默认，服务声明上的 `#[lazy]` / `#[lazy(false)]` 策略仍优先，具体见
[服务级预热策略](#34-用-lazy-控制某个服务是否自主预热)。

## 3. 用 injectable 声明服务

### 3.1 每个字段选择一种初始化方式

没有显式 constructor 时，`#[injectable]` 支持具名字段结构体、元组结构体和单元结构体。
本节介绍这种自动字段模式，显式构造模式见[constructor](#35-用-constructor-明确初始化业务状态)。服务声明必须放在
模块作用域，不能放进函数体或局部代码块；业务方法写在普通 `impl` 中。

| 字段写法 | 容器如何处理 |
| --- | --- |
| `#[inject] dependency: SomeService` | 解析必选服务 |
| `#[inject] port: dyn SomeTrait` | 根据接口和 key 选择实现 |
| `#[inject("mail")] port: dyn SomeTrait` | 解析字符串 key 为 `"mail"` 的服务 |
| `#[inject(123)] dependency: SomeService` | 解析整数 key 为 `123` 的服务 |
| `#[inject] optional: Option<SomeService>` | 没有注册时注入 `None` |
| `#[inject] #[lazy] dependency: SomeService` | 注入 `LazyInjection<SomeService>`，首次 `get().await` 获取目标 |
| `#[value(表达式)] field: T` | 创建实例时执行表达式并初始化字段 |
| `field: T`，没有上述属性 | 创建实例时调用 `T::default()`，不会自动注入 |

例如，下面的声明可以加到第一个程序中：

```rust
#[injectable]
struct RetryPolicy {
    #[value(3)]
    max_attempts: usize,
    #[value("order-notification")]
    name: String,
    #[value(format!("{}-retry", "mail"))]
    label: String,
    enabled: bool, // 使用 bool::default()，初始值为 false。
}
```

`#[value("文本")]` 可以初始化 `String`；字符串字面量与路径表达式会按字段类型调用
`Into`，例如常量路径也可转换为 `String`。数字、函数调用和代码块在字段类型上下文
中求值，结果须与字段类型匹配；不会为任意表达式都自动添加 `Into`。它不是依赖查询，
也不在编译期或图验证时求值。需要异步初始化或返回初始化错误时，使用 `#[factory]`；
根据依赖计算普通业务值也可使用同步 `#[constructor]`。

一个字段只能选择一种策略，不能同时写 `#[inject]` 和 `#[value(...)]`。
不带属性的字段必须实现 `Default`；若希望它来自容器，请明确写 `#[inject]`。

普通注入字段按只读共享引用使用，可以直接调用 `self.notifier.notify(...)`。
需要传给接收 `&T` 的函数时，可以显式写 `&*self.dependency`。这些字段由宏改写为
容器管理的只读注入类型，不应当作可直接移动或可变借用的普通 `T`；业务可变状态
应使用 `Mutex`、`RwLock` 或原子类型等同步机制。

注入字段与 constructor / factory 参数共用以下类型语法：目标写成准确类型路径或
`dyn Trait`，允许最外层一层显式 `Option<T>`。不要写 `&T`、`Arc<T>`、嵌套
`Option<Option<T>>`、`impl Trait`、`Self` 或 `<T as Trait>::Item` 这类限定关联类型。
普通服务类型别名可以使用；可选性由源码中的 `Option<T>` 形状识别，不能把自定义
可选类型别名当成相同的声明语法。括号包裹的 `(T)` 与 `T` 等价。


### 3.2 声明生命周期

`injectable` 和 `factory` 使用相同的参数：

| 参数 | 示例 | 默认值与含义 |
| --- | --- | --- |
| `lifetime` | `lifetime = Scoped` | 默认 `Singleton` |
| `key` | `key = "mail"`、`key = 7` | 默认无 key |
| `cleanup` | `cleanup = "cleanup_mail"` | 默认没有异步清理回调 |

生命周期有三种：

| 生命周期 | 复用范围 | 常见用途 |
| --- | --- | --- |
| `Singleton` | 一个 root 容器内共享 | 配置、线程安全的连接池或客户端 |
| `Scoped` | 同一个 scope 内共享，不同 scope 隔离 | 请求上下文、一次业务处理的服务 |
| `Transient` | 每次查询或每个注入槽位独立创建 | 轻量、需要独立实例的协作者 |

推荐写 `lifetime = Singleton`、`Scoped` 或 `Transient`；也支持
`lifetime = "singleton"`、`"scoped"`、`"transient"`。这里的标识符是属性参数，
无需为了使用 `Scoped` 而导入同名类型。

生命周期会沿完整依赖链验证。Singleton 可以依赖 Transient，但不能直接或间接
依赖 Scoped；工厂参数同样受这个规则约束。依赖 Scoped 的 Transient 必须从 scope
查询，不能从 root 查询。

所有服务类型都需要满足 `Send + Sync + 'static`。`'static` 表示服务不能借用短期
外部数据，不表示每个服务都要存活到程序结束。接口通常声明为
`trait MyService: Send + Sync { ... }`，且必须能作为 `dyn MyService` 使用。

### 3.3 用 lazy 推迟某个字段的依赖初始化

某些服务只在少数业务分支使用，例如导出订单报表的客户端。给注入字段增加
`#[lazy]` 后，创建消费者不再要求这个目标及其依赖已经完成构造：

```rust
use nestrs::{factory, injectable};
use nestrs_core::ResolveError;

struct ReportService;

impl ReportService {
    fn generate(&self) -> String {
        "订单报表".to_owned()
    }
}

#[factory]
async fn report_service() -> ReportService {
    // 可以在这里建立异步连接；只在目标确实需要初始化时执行。
    tokio::task::yield_now().await;
    ReportService
}

#[injectable]
struct OrderService {
    #[inject]
    #[lazy]
    reports: ReportService,
}

impl OrderService {
    async fn export_report(&self) -> Result<String, ResolveError> {
        let reports = self.reports.get().await?;
        Ok(reports.generate())
    }
}
```

源码仍声明业务类型 `ReportService`，工具将该字段改写为
`nestrs_core::LazyInjection<ReportService>`。`get().await` 返回借用句柄的 `&ReportService`；
目标就绪后，普通同步业务方法仍可直接调用。句柄没有同步 `Deref`，不能省略首次获取
所需的异步等待而写成 `self.reports.generate()`。

字段还可以组合 key、接口、闭合泛型及 optional：

```rust
#[inject("sales")]
#[lazy]
reports: dyn ReportPort,

#[inject]
#[lazy]
cache: ReportCache<Customer>,

#[inject]
#[lazy]
optional_reports: Option<ReportService>,
```

最后一种形式生成 `Option<LazyInjection<ReportService>>`。注册不存在时是 `None`；
存在时先取句柄，再调用 `get().await`。初始化失败仍是错误，不会变成 `None`。

生命周期决定目标的共享范围：Singleton 在 root 内共享，Scoped 在其所属 scope 内共享；
Transient 每个延迟字段拥有独立的一次构造。同一字段的多次和并发 `get()` 共享这次结果，
包括初始化失败；需要新的 Transient 尝试时，须通过新的消费槽位获取。
取消一次等待不会取消已接受的初始化，也不会让下一次等待重复构造。

图在最终 binary / test 编译时完整验证并冻结。延迟字段的缺失依赖、key 歧义、循环和生命周期冲突
都会提前报告，不会因为尚未访问而被跳过。特别是 Singleton 不能借助 `#[lazy]` 依赖
Scoped；依赖 Scoped 的 Transient 仍须从 scope 查询。

字段上的延迟标记不改变目标自身的初始化选择。如果目标是 Singleton 且 root 为
Eager，它仍可在 `build(options).await` 中自主初始化；如果目标是 Scoped 且本次
scope 为 Eager，它仍可在 `create_scope(options).await` 中自主初始化。root 与
scope 的模式互相独立；若希望目标本身也按需初始化，可以组合下一节的服务级 `#[lazy]`。
其他普通注入依赖需要它时仍会提前构造。字段只接受裸 `#[lazy]`，不接受带括号的
`#[lazy()]`、`#[lazy(true)]` 或 `#[lazy(false)]`；factory 参数也支持相同的裸标记，
具体用法见[把延迟依赖传入工厂](#43-把延迟依赖传入工厂)。

在容器的构造 worker 中，当前句柄尚未缓存类型化结果时，调用 `get()` 会返回明确的
`ResolveError`；即使该 Singleton 已由别处构造，也不能绕过当前句柄的首次交付检查。该保护覆盖同步构造、`#[value]` / `Default` 和同一任务内的 factory
异步调用链，避免构造任务占着名额等待另一构造任务而死锁。构造所需的前置依赖应使用
普通依赖字段或 factory 参数声明；业务方法执行期间再使用延迟获取。
用户另外启动的 `tokio::spawn` / `spawn_blocking` 任务不会继承此阶段标记，框架无法识别
任意用户任务间的因果关系；factory 也不能通过派生任务等待尚未初始化的延迟字段，
这种写法仍可能耗尽构造名额。

owner 开始关闭后，未初始化句柄拒绝新建目标。已成功取得的目标由强 lease 保活；
即使安全代码把句柄移出消费者，内存仍保持有效，但 cleanup 后不承诺业务资源可用。
关闭会先完成消费者 cleanup，再清理它的目标，即使目标晚于消费者发布。

### 3.4 用 lazy 控制某个服务是否自主预热

服务声明上的 `#[lazy]` 决定它是否成为预热根。例如项目默认 Eager，但报表客户端
不需要在启动时连接远端，可以给工厂标注 `#[lazy]`；相反，即使项目默认 Lazy，
启动必需的配置检查也可以用 `#[lazy(false)]` 要求在 `build(options).await` 返回前完成：

```rust
use nestrs::{factory, injectable, lazy};

#[lazy(false)]
#[injectable]
struct StartupChecks;

struct ReportClient;

#[factory]
#[lazy] // 等价于 #[lazy(true)]。
async fn report_client() -> ReportClient {
    tokio::task::yield_now().await;
    ReportClient
}
```

属性可以位于 `injectable` / `factory` 上方或下方，也可与 `primary` 组合。
支持同步工厂、异步工厂、key、trait 绑定和闭合泛型；泛型服务沿用蓝图上的策略。
`#[lazy()]` 在**服务声明**上与 `#[lazy]` 等价。一个声明最多写一个服务级 `lazy`，
参数只能是单个布尔字面量，不能写字符串、表达式或 `lazy = true`。

| 服务声明 | 所属 owner 为 Lazy 时 | 所属 owner 为 Eager 时 |
| --- | --- | --- |
| 不写 `lazy` | 按需构造 | 自主初始化 |
| `#[lazy]` / `#[lazy(true)]` | 不作为自主初始化入口 | 不作为自主初始化入口 |
| `#[lazy(false)]` | 自主初始化 | 自主初始化 |

Singleton 使用 root 模式，选中时在 `build(options).await` 返回前初始化；Scoped
使用本次 scope 模式，选中时在 `create_scope(options).await` 返回前初始化。
即使 scope 默认 Lazy，`#[lazy(false)]` 的 Scoped 也会在创建时构造。root 为 Eager
不改变 scope 的独立默认值。Transient 不作为初始化入口，任何标记都不改变“每次
消费独立构造”的规则。公开的 `warm_up()` 已移除，预先初始化通过创建配置控制。

这里的“按需”包含普通依赖：若一个自主预热的服务普通注入了 `ReportClient`，
客户端仍会在该消费者构造前完成初始化。要让消费者先就绪、报表客户端留待业务分支，
应同时给客户端声明标注服务级 `#[lazy]`，并给消费者相应字段写 `#[inject] #[lazy]`。
服务级标记不生成代理，也不把普通 `Injection<T>` 改成 `LazyInjection<T>`。

服务级延迟不绕过编译期完整图验证，未使用的服务仍检查缺失依赖、循环和生命周期冲突。
创建时选中的服务构造失败，root 返回 `BuildError::Initialization`，scope 返回
`ScopeBuildError`，无论对应模式为 Lazy 还是 Eager；未交付 owner 已成功构造的实例会
完成清理。查询时才发生的失败仍返回 `ResolveError`，原有缓存、取消等待和 cleanup
顺序不变。

### 3.5 用 constructor 明确初始化业务状态

服务需要根据依赖初始化业务状态时，可以在 `impl` 内选择一个同步构造函数。结构体
字段仍写业务类型，依赖及 key、optional、lazy 都写在构造参数上：

```rust
use nestrs::{constructor, injectable};

#[injectable(lifetime = Scoped)]
pub struct OrderService {
    orders: dyn OrderStore,
    reports: ReportService,
    audit: Option<dyn AuditSink>,
    order_prefix: String,
}

impl OrderService {
    #[constructor]
    fn new(
        #[inject("primary")] orders: dyn OrderStore,
        #[lazy] reports: ReportService,
        audit: Option<dyn AuditSink>,
    ) -> Self {
        Self {
            orders,
            reports,
            audit,
            order_prefix: "ORD".to_owned(),
        }
    }
}
```

示例假定 `OrderStore`、`ReportService` 和 `AuditSink` 已定义，相应必选服务已声明。
可直接运行的业务版本见 [CheckoutService](../example/di-checkout/src/checkout/service.rs)。

`#[injectable]` 仍然决定 lifetime、key、primary、服务级 lazy 和 cleanup，该服务只有
一个创建声明。`#[constructor]` 不接受配置参数；方法不必叫 `new`，也不必公开。
参数默认是必选、默认 key 的依赖；`#[inject("name")]`、`#[inject(123)]`、`Option<T>`
以及裸 `#[lazy]` 的含义与其他注入位置一致。

三种位置的参数和字段有不同的持有方式，选择时注意区分：

| 声明位置 | 普通依赖 | 延迟依赖 |
| --- | --- | --- |
| 自动字段模式的 `#[inject]` 字段 | `Injection<T>`，由服务长期持有 | `LazyInjection<T>`，由服务长期持有 |
| `#[constructor]` 参数 | 按值 `Injection<T>`，可移入对应字段 | 按值 `LazyInjection<T>`，可移入对应字段 |
| `#[factory]` 参数 | 构造 frame 提供的 `&T`，不能作为长期借用逃逸 | 按值 `LazyInjection<T>`，可存入返回对象 |

源码都写业务类型 `T`；optional 形式相应加一层 `Option`。注入字段由工具改写；工厂
返回的是普通 Rust struct 时，存储延迟句柄的字段需手写 `LazyInjection<T>`。

工具将普通参数改成按值的 `Injection<T>`，将延迟参数改成 `LazyInjection<T>`；可选
参数相应成为 `Option<...>`。因此构造函数可以直接把参数移入字段，业务代码无需
手写包装类型。普通依赖可通过自动解引用调用服务方法，延迟依赖在业务阶段通过
`get().await` 获取。依赖若由异步 factory 创建，容器先等待它就绪，再执行同步构造函数。

字段和参数的类型括号不改变这条规则：`(Option<T>)`、`Option<(T)>` 及嵌套括号
都按对应的 `Option<T>` 处理，普通和延迟输入一致；`std::option::Option` 与
`core::option::Option` 也保留原路径。可选输入缺席时仍交付 `None`。

字段是否包装取决于实际赋值来源：`Self { report_client: reports, ... }` 和
`let client = reports; Self { report_client: client, ... }` 都支持；
`Self { order_prefix: config.prefix.clone(), ... }` 保持普通 `String`。
参数即使只参与校验、计算业务值或未保存到字段，也仍然贡献一个完整图输入。
框架不根据字段与参数同名、同类型推测注入关系。

字段来源在 Rust 完成 `cfg` 筛选、宏展开和名称解析之后分析。字段及其初始化项都被
`cfg` 排除时，不再改写该字段，但仍保留构造参数的依赖声明。`macro_rules!` 生成的
构造函数也按真实变量绑定区分来源：宏内部的局部变量即使与调用端传入的参数同名，
也不会仅因名字相同而覆盖参数的注入关系。

构造函数可以返回 `Result<Self, E>`，要求 `E: Debug`，成功分支写为 `Ok(Self { ... })`。
返回类型写裸 `Result`、`std::result::Result` 或 `core::result::Result`，不使用自定义别名。
失败和 panic 进入现有初始化错误通道；Singleton / Scoped 失败缓存，Transient 的
失败只影响本次实例。未成功发布的实例不执行 cleanup，已经创建的依赖仍按各自
owner 的规则保活和关闭。

显式构造模式要求完整初始化 `Self`，不再补做字段注入或 `Default`；不能混用字段
`#[inject]`、`#[value]`、`#[lazy]`。没有 `#[constructor]` 的服务继续使用原有字段模式。

泛型服务的 constructor impl 必须覆盖整个服务蓝图：`impl<U> Service<U>` 可以把
参数改名，多参数可以重排，类型和 const 参数可以加不改变含义的括号，但 Self 的每个
泛型位置必须一一使用 impl 自己的参数。`impl Service<u32>`、`Service<Vec<U>>`、
重复参数或固定 const 的专门化构造不受支持，会给出构造声明诊断。这里也不在 HIR 前
展开 `Alias<U>` 等类型别名来证明等价；需要直接写已声明的泛型参数。额外 trait bounds
仍由 Rust 类型检查验证。这条约束避免把只适用于部分类型的构造函数连接到整个蓝图。

每个服务在 cfg 筛选后只允许一个构造函数，支持泛型 inherent impl、外部模块、宏生成
声明和上游业务 crate。参数仍受真实 Rust 类型、借用、可见性检查约束。
当前也不通过类型别名寻找 constructor impl 所属的服务结构体。

当前字段来源分析支持具名结构体、单元结构体的 `Self`、直接字段赋值、简单 `let` 别名，以及映射一致的
`if` / `match` / 提前 `return`。不接受 `..` 更新、隐藏在其他函数返回值中的整个
`Self` 或依赖令牌的复杂重新赋值；宏展开后的表达式也须满足同样的来源规则。循环
业务计算可提取为普通辅助函数，通过借用参数完成。无法确认字段来源时会给出编译
诊断，不猜测字段类型。

构造函数必须同步，不支持 `async fn`、间接返回 Future、receiver、trait impl、
unsafe / extern 或方法自己的泛型参数。异步初始化仍使用 `#[factory]`；构造 worker
内不能首次获取尚未缓存类型化结果的延迟句柄，即使目标实例已就绪也会拒绝。
这与 factory 的延迟参数限制相同。

编辑器使用 `cargo nestrs init` 生成的编译单元模型复用相同字段选择。增加或调整 DI
声明后保存文件，配置的检查命令会刷新模型；也可执行 `cargo nestrs init check`。
字段来源有歧义或模型与编辑内容不一致时会明确提示刷新，不把错误诊断隐藏。

## 4. 用 factory 控制创建过程

在以下情况使用工厂：第三方类型不能修改、需要异步 I/O、创建时可能失败，或者需要
根据配置构造服务。工厂注册的是成功返回的服务类型，函数名称不是查询名称。

| 函数形式 | 注册的服务 |
| --- | --- |
| `fn make() -> Client` | `Client` |
| `fn make() -> Result<Client, E>` | 成功时的 `Client` |
| `async fn make() -> Client` | 等待完成后的 `Client` |
| `async fn make() -> Result<Client, E>` | 等待成功后的 `Client` |
| `fn make() -> impl Future<Output = Client>` | 等待完成后的 `Client`；也支持 `Output = Result<Client, E>` |

`async fn` 应直接返回服务或 `Result`，不能再返回一层 Future。
返回 `Result` 时，错误类型 `E` 需要实现 `Debug`。错误会成为服务初始化错误；
服务不会以失败的值发布。异步工厂返回的 future 需要满足 `Send`，跨 `await` 时
不要持有非 `Send` 的值或普通 Mutex guard。

### 4.1 工厂参数就是依赖

第一个例子的参数 `config: AppConfig` 不需要写 `#[inject]`。每个工厂参数默认就是
必选依赖；`Option<T>` 表示可选依赖，`dyn Trait` 表示接口依赖。需要 key 时才添加
辅助属性，例如：

```rust
#[factory]
fn notifier_label(#[inject("mail")] notifier: dyn Notifier) -> String {
    notifier.notify(0)
}
```

这是参数写法示例：使用它前需要注册 key 为 `"mail"` 的 `Notifier` 实现，不能直接
把它加入只有默认 key 的第一个程序后就期望图验证成功。下一节给出完整的 keyed 例子。

普通参数按 `config: AppConfig` 写，容器会把它作为**此次构造期间有效的共享借用**
传入，跨 `await` 也有效。不要手写成 `&AppConfig`，不要把这个借用保存进返回的服务，
或传给 `tokio::spawn` 这样要求 `'static` 的独立任务。需要长期持有配置内容时，
复制所需数据，正如例子中的 `config.sender.clone()`。

工厂参数只接受 `inject` 与裸 `lazy` helper；不支持 `#[value]` 参数，普通常量或计算值直接写在函数体中。也不需要给工厂
返回的 `Client` 再加 `#[injectable]`：同一具体类型、同一 key 同时有两个 provider
会导致图验证失败。通常由容器调用工厂，业务入口通过查询方法取得服务。

### 4.2 当前函数约束

工厂必须是模块作用域的普通函数，不是 `impl` 方法或函数体内的局部函数；可以是
同步或异步函数，但不能是 `unsafe fn`、`extern fn` 或带泛型参数的函数。
参数使用简单名称，避免解构参数。返回值必须是具体服务，不能是 `()`。

工厂函数会被宏调整为所在模块私有，即使源码写了 `pub` 也不会成为跨模块的公开
构造接口。对外暴露服务类型及业务方法，调用方通过容器查询取得服务。

为了让宏识别成功类型，使用直接的 `Result<T, E>`、`std::result::Result<T, E>` 或
`core::result::Result<T, E>`；不要用自定义 Result 别名期待相同的错误解包行为。
需要泛型服务时，优先使用泛型 `#[injectable]` 或返回某个明确闭合类型的非泛型工厂。

### 4.3 把延迟依赖传入工厂

若工厂返回的对象需要在业务阶段才使用某个依赖，给该参数添加 `#[lazy]`。
参数原本就默认注入，因此这里不要求同时标注 `#[inject]`。例如报表入口启动时只准备
业务对象，报表客户端留到用户实际导出时再获取：

```rust
use nestrs::{factory, injectable, lazy};
use nestrs_core::{LazyInjection, ResolveError, ServiceProvider};

#[injectable]
struct ReportConfig {
    #[value("sales")]
    prefix: String,
}

#[injectable]
#[lazy] // 服务本身也不主动预热；与参数上的延迟标记各自独立。
struct ReportClient;

impl ReportClient {
    fn export(&self, prefix: &str) -> String {
        format!("{prefix}-report.csv")
    }
}

struct ReportExporter {
    prefix: String,
    client: LazyInjection<ReportClient>,
}

#[factory]
async fn report_exporter(
    config: ReportConfig,
    #[lazy] client: ReportClient,
) -> ReportExporter {
    tokio::task::yield_now().await;
    // config 是由真实 frame 保活的借用；client 是可以移动并长期保存的句柄。
    ReportExporter { prefix: config.prefix.clone(), client }
}

impl ReportExporter {
    async fn export(&self) -> Result<String, ResolveError> {
        let client = self.client.get().await?;
        Ok(client.export(&self.prefix))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::build(None).await?;
    let exporter = provider.get_required_service::<ReportExporter>().await?;
    println!("{}", exporter.export().await?);
    provider.dispose_async().await?;
    Ok(())
}
```

工具将 `#[lazy] client: ReportClient` 改写成按值的 `LazyInjection<ReportClient>`，
`#[lazy] client: Option<ReportClient>` 改写成 `Option<LazyInjection<ReportClient>>`。
也支持 `#[inject("sales")] #[lazy] client: dyn ReportPort`、整数字面量 key 和闭合泛型。
源码参数继续写目标业务类型，不手写 `LazyInjection<T>`；返回对象若是普通 Rust struct，
其存储字段则需明确声明 `LazyInjection<T>`，如上例所示。

同步工厂、`async fn` 及返回显式 `impl Future<Output = T>` 的工厂都支持延迟参数；
可以与普通借用参数混用。每个延迟参数独立消费一次输入槽位，读取参数本身不构造目标。
可选依赖的 `None` 在编译计划中确定；存在目标时，初始化失败仍返回错误。

延迟参数只接受裸 `#[lazy]`（也可写 `#[nestrs::lazy]`），不接受布尔参数、空括号或
重复标记。参数属性不需要导入独立宏，由 `factory` 消费；函数本身的服务级 `#[lazy]`
则是另一个位置的策略，决定工厂产出的服务是否在所属 owner 创建时自主初始化。

**应将句柄交给业务阶段使用。** 工厂构造 worker 内对尚未缓存类型化结果的句柄调用
`get().await` 仍会返回 `ResolveError`，即使目标已被其他请求构造；已有句柄结果可复用。
此限制避免占据构造名额等待其他构造任务。需要先取得依赖才能完成
工厂的场景，请声明普通参数；不要通过自行 spawn 的任务绕过该等待限制。
延迟参数保留字段级句柄的缓存、取消、owner 关闭和 cleanup 规则，不会绕过编译期的
缺失依赖、歧义、循环或生命周期检查。

## 5. 接口、key 和 primary 怎么配合

### 5.1 接口绑定由普通 impl 提供

当注入字段、constructor / factory 参数或查询方法请求 `dyn Notifier` 时，工具链会根据已有服务声明和普通
`impl Notifier for MailClient` 生成绑定。业务代码不写 `#[bind]`，也不需要调用
注册函数。仅有 `impl` 不会把任意类型自动变成服务：实现类型仍需来自
`#[injectable]`、工厂输出，或已经确定的泛型服务类型。

接口和具体类型查询选中同一个 provider 时，遵循同一生命周期规则；接口绑定本身
不额外创建实例。Singleton/Scoped 在各自范围内共享，Transient 仍按每次消费创建。

### 5.2 用 key 区分同类型服务

下面是另一个完整程序，可以独立替换 `src/main.rs`。它为同一个通知客户端注册
`mail`、`sms` 两个渠道，展示字段注入与直接查询两种方式：

```rust
use nestrs::{factory, injectable};
use nestrs_core::{ServiceKey, ServiceProvider};

trait Notifier: Send + Sync {
    fn channel(&self) -> &'static str;
}

struct NotificationClient(&'static str);

impl Notifier for NotificationClient {
    fn channel(&self) -> &'static str {
        self.0
    }
}

#[factory(key = "mail")]
fn mail() -> NotificationClient {
    NotificationClient("mail")
}

#[factory(key = "sms")]
fn sms() -> NotificationClient {
    NotificationClient("sms")
}

#[injectable]
struct NotificationService {
    #[inject("mail")]
    mail: dyn Notifier,
    #[inject("sms")]
    sms: dyn Notifier,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::build(None).await?;
    let service = provider
        .get_required_service::<NotificationService>()
        .await?;
    assert_eq!(service.mail.channel(), "mail");
    assert_eq!(service.sms.channel(), "sms");

    let channel = "mail".to_owned();
    let notifier = provider
        .get_required_keyed_service::<dyn Notifier>(ServiceKey::Named(channel))
        .await?;
    assert_eq!(notifier.channel(), "mail");
    provider.dispose_async().await?;
    Ok(())
}
```

注意服务注册、依赖注入与查询的 key 写法不同：

| 位置 | 字符串 key | 整数 key |
| --- | --- | --- |
| `injectable` / `factory` 注册属性 | `key = "mail"` | `key = 7` |
| 字段、factory / constructor 参数的注入属性 | `#[inject("mail")]` | `#[inject(7)]` |
| keyed 查询方法的 key 参数 | `ServiceKey::Named("mail".to_owned())` | `ServiceKey::Indexed(7)` |

属性只接受非空字符串或可表示为 `usize` 的非负整数字面量，不能写运行期变量。
字段、factory / constructor 参数的注入属性只接受裸标记 `#[inject]` 或单个 key 字面量，如
`#[inject("mail")]`、`#[inject(7)]`。`#[inject(key = "mail")]`、
`#[inject(key = 7)]` 会导致编译错误；`injectable` / `factory` 自身的服务注册配置
仍使用 `key = ...`。
keyed 查询方法可以接收运行期计算出的 `ServiceKey`，但不会创建新的注册。

无 key、字符串 `"7"`、整数 `7` 是三种不同选择；默认查询不会回退到 named/indexed
key。`primary` 也不会跨 key 选择。

### 5.3 用 primary 选择同 key 的接口实现

同一个 trait、同一个 key 只有一个候选时直接使用它；有多个候选时，需要恰好一个
候选被标为 `#[primary]`。以下也是可独立运行的程序：

```rust
use nestrs::{injectable, primary};
use nestrs_core::ServiceProvider;

trait Notifier: Send + Sync {
    fn channel(&self) -> &'static str;
}

#[injectable]
#[primary]
struct MailNotifier;

impl Notifier for MailNotifier {
    fn channel(&self) -> &'static str { "mail" }
}

#[injectable]
struct SmsNotifier;

impl Notifier for SmsNotifier {
    fn channel(&self) -> &'static str { "sms" }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::build(None).await?;
    let notifier = provider.get_required_service::<dyn Notifier>().await?;
    assert_eq!(notifier.channel(), "mail");
    provider.dispose_async().await?;
    Ok(())
}
```

`#[primary]` 也可以和 `#[factory]` 配合，写在声明属性的上方或下方均可；不接收
参数，`#[primary()]` 与裸标记等价；同一服务不能重复标注。它只解决接口的多实现选择，
不覆盖同一 concrete 类型与 key 的重复声明，
也不应单独作为服务注册使用。

## 6. 可选依赖：不存在才是 None

以下片段可以加入第一个程序。没有注册 `AuditSink` 实现时，`audit` 是 `None`：

```rust
trait AuditSink: Send + Sync {
    fn record(&self, order_id: u64);
}

#[injectable(lifetime = Scoped)]
struct AuditedOrderService {
    #[inject]
    audit: Option<dyn AuditSink>,
}

impl AuditedOrderService {
    fn record_if_enabled(&self, order_id: u64) {
        if let Some(audit) = &self.audit {
            audit.record(order_id);
        }
    }
}
```

工厂参数也可写 `audit: Option<dyn AuditSink>`，可选查询则使用 `get_service` 或
`get_keyed_service` 方法。可选只表示“允许没有候选”：存在候选时，依赖歧义、循环、
生命周期冲突和初始化失败仍然是错误，不会被吞掉转为 `None`。

## 7. 查询服务：四个普通方法都需要 await

| 调用 | 返回结果 |
| --- | --- |
| `provider.get_required_service::<T>().await` | `Result<&T, ResolveError>` |
| `provider.get_service::<T>().await` | `Result<Option<&T>, ResolveError>` |
| `provider.get_required_keyed_service::<T>(key).await` | `Result<&T, ResolveError>` |
| `provider.get_keyed_service::<T>(key).await` | `Result<Option<&T>, ResolveError>` |

`T` 可以是具体类型、闭合泛型类型或 `dyn Trait`。在 root provider 或
`scope.service_provider()` 返回的视图上调用方法；scope 本身不直接提供查询方法。
接收者和 key 表达式都只求值一次，查询不会消费 owner。

以下片段放在第一个程序的 `scope.dispose_async()` 之前即可，复用已有 `scope`：

```rust
let view = scope.service_provider();
let orders = view.get_required_service::<OrderService>().await?;
let notifier = view.get_service::<dyn Notifier>().await?;
assert!(notifier.is_some());
assert!(orders.submit(1002).contains("1002"));
```

返回引用借用实际 root 或 scope。使用引用期间不能消费对应 owner 进行关闭；
最后一次使用服务后，再调用 `scope.dispose_async().await?` 和
`provider.dispose_async().await?`。

查询使用普通方法，不需要注册宏；`build`、`create_scope`、
`service_provider` 和 `dispose_async` 也都是普通方法。编译器识别这些查询方法的真实定义，
运行时只选择冻结路由。

## 8. 泛型服务：从具体调用恢复闭合类型

`#[injectable]` 支持泛型结构体。例如以下完整程序按需取得订单仓库：

```rust
use std::sync::Mutex;
use nestrs::injectable;
use nestrs_core::ServiceProvider;

struct Order {
    id: u64,
}

#[injectable]
struct Repository<T: Send + Sync + 'static> {
    records: Mutex<Vec<T>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::build(None).await?;
    let orders = provider
        .get_required_service::<Repository<Order>>()
        .await?;
    orders.records.lock().unwrap().push(Order { id: 1001 });
    assert_eq!(orders.records.lock().unwrap()[0].id, 1001);
    provider.dispose_async().await?;
    Ok(())
}
```

注入依赖或查询方法中出现的 `Repository<Order>` 会让这个闭合类型在最终入口编译时
进入执行计划；即使查询写在 `build` 后面，也不需要预先调用注册函数。
`Repository<Order>` 和 `Repository<User>` 是两个不同的服务类型，各自遵循声明的
生命周期。框架不会枚举所有可能的 `T`。

注入字段和工厂参数支持等价的类型别名，包括 `type Store = dyn StorePort`。
工厂返回的闭合泛型也可以直接注入，不需要再为该类型添加 `#[injectable]`。
当泛型服务的字段写为 `#[inject] service: T` 时，例如查询
`Wrapper<Repository<Order>>`，工具会按实际闭合类型展开 `T` 及其必要依赖；
不会因为字段源码中没有写出 `Repository<Order>` 而漏掉它的声明。
这些类型均由工具链在编译时收集，运行时查询不扩展依赖图。

普通方法支持在泛型辅助函数中查询 `T`，由其闭合调用恢复实际类型：

```rust
async fn load<T: Send + Sync + 'static>(
    provider: &ServiceProvider,
) -> Result<&T, nestrs_core::ResolveError> {
    provider.get_required_service::<T>().await
}

// 放在上例 main 内：闭合调用使 Repository<Order> 进入编译计划。
let orders = load::<Repository<Order>>(&provider).await?;
```

闭合 impl 中的 `Self` 和跨 crate 辅助函数也按编译器查询摘要收集。业务类型实现标准
trait 时也遵循同一规则，例如通过 `Iterator::next`、`Add::add` 或 `+` 调用进入的
业务实现；编译器按实际闭合 impl 分析，不以方法最初属于标准库为由丢弃它的查询。
隐式自动解引用进入的 `Deref`／`DerefMut`，以及已知具体类型经 Rust 析构逻辑进入的
`Drop` 实现也遵循这一规则，跨 crate 的泛型参数在下游闭合后继续展开。
元组或闭包的 `.clone()` 由编译器生成字段克隆调用；其中进入的业务 `Clone`
实现也会贡献已闭合查询。仅创建元组或保存闭包而没有克隆，不会因此请求字段的
`Clone` 方法中的服务。克隆引用或 `Arc<T>` 不等于克隆其内部的 `T`；反过来，
`T: Copy` 也不保证元组克隆跳过其自定义 `Clone`。是否存在调用以 rustc 的真实实现为准。
已经通过依赖纳入计划的闭合服务类型，其可确定的方法查询也参与收集。

普通泛型 helper 库即使不依赖 `nestrs-core`，也可以通过 trait 方法或闭包转发查询。
编译器在相关调用闭合后读取其真实调用关系，包括库内已编译但未执行的分支、类型
擦除和关联常量；无需给 helper 添加无业务用途的 core 依赖。具体调用仍须能静态
闭合，不包含任意运行期修改函数指针或动态加载代码的目标推断。
`Family::Target` 等关联类型也可以传递查询对象，包括 helper 内部才构造该对象的
情况；编译器保留声明约束与实际闭合类型之间的关系。

普通业务对象也可以通过 `dyn Trait` 调用包含查询的方法。编译器保留真实的 concrete
到 trait object 转换，在泛型 helper 或跨 crate 调用闭合后恢复对应实现；业务对象本身
无需声明成 provider。匹配仍检查接口参数、关联类型和合法的父接口转换；例如
`Child<T>: Run<T::Target>` 的父接口参数会在具体类型闭合后归一化。单独转换
一个未调用的对象不会自动请求它所有方法中的服务。

范围仍是编译器能恢复的有限闭合调用，不是运行期反射或无限开放泛型注册。方法自己
尚未确定的额外类型或 const 参数不会被猜测。持续增长的类型表达式和超过 100,000
的闭合类型/查询展开会被拒绝；单个类型树上限为 `max(8 × recursion_limit, 1024)`，
这些是类型空间限制，与普通依赖链的深度不同。方法中的查询不会自动成为该服务的
构造依赖；业务协作关系仍应通过注入字段、constructor 或 factory 参数表达。
DI 展开预算只约束实际服务与查询分析；与 DI 无关的普通 Rust 类型及其 trait impl
不会仅因类型较大而触发此限制，仍由 Rust 编译器检查。同一 trait 的其他实现含有
查询，也不会让自身不查询的实现被套用服务类型预算；只查询固定小类型的 helper
可以携带无关的大型泛型参数，实际请求大型服务类型时仍会接受原预算检查。
关联投影按归一化后的服务类型核对；投影输入很大而结果是小类型时，不会仅因输入
类型大小被误认为请求了大型服务。
查找查询来源时，有限的大型业务对象也可以正常创建或转换为 trait object；若同一
真实函数或常量沿分析链不断生成更大的泛型类型，仍会受有限展开保护并明确报错。
这部分筛选是保守的：关联类型约束把 helper 纳入分析后，反复扩大其泛型实参仍可能
触发保护，即使最终选中的叶子实现没有查询。不同 impl 仅共享 trait 方法声明不属于
同一实现重入。具体分析规则见[查询根与预算边界](NESTRS_RUSTC_EXTENSION_GUIDE.md#query-budgets)。

已编译但没有执行的分支（包括 `if false`）里的查询方法也会贡献类型声明；被 `#[cfg]` 排除的
代码不会贡献声明。动态 key 只选择已经冻结的路由，不改变泛型声明本身的 key，也
不会在查询时扩展依赖图。

## 9. 为服务配置异步 cleanup

下面的独立声明示例可加到第一个程序中；要观察输出，需要先查询并成功创建这个服务：

```rust
async fn cleanup_notification_worker() {
    println!("通知 worker 的 cleanup 已完成");
}

#[injectable(cleanup = "cleanup_notification_worker")]
struct NotificationWorker;
```

`#[factory(cleanup = "cleanup_notification_worker")]` 使用相同配置。`cleanup` 的值
是包含函数路径的字符串；通常写成无参数、返回 `()` 的 `async fn`，其 Future 必须
满足 `Send + 'static`。等价的无参数函数若返回 `Future<Output = ()> + Send + 'static`
也可使用；它不接收 `self` 或实例参数。实例自身持有的同步资源可通过普通 Rust `Drop`
释放，不要把这个 hook 当成 `async fn close(&mut self)`。

对每个成功发布的实例，owner 关闭时调用一次配置的 hook。未实例化或构造失败的
服务不会执行它。显式 `dispose_async().await` 会等待关闭完成；清理按依赖关系
要求让消费者先完成，hook panic 会被记录并继续清理其他实例。

普通 `Drop` 不会阻塞等待异步清理；Tokio runtime 已退出时不能保证异步 hook
执行完毕。因此，在程序结束前显式关闭 scope 和 root。

## 10. 与普通 Rust 宏、模块和条件编译一起使用

服务可以放在普通模块或外部 `mod` 文件中。`macro_rules!` 也能生成模块级服务声明；
生成结果仍要使用相同的 Nestrs 属性和普通 `impl`。推荐在源码中定义模块，再用宏
生成模块里的声明；把整个私有模块和实现都藏在宏展开中，可能无法生成可见的接口
绑定，当前不保证这种组织方式可用。

声明可配合 `#[cfg]`、`#[cfg_attr]` 选择特性，但需要保证当前编译配置下请求的服务
有相应声明。切换 feature 时，对构建和 `init` 使用一致的 `--features` 参数；
rust-analyzer 以初始化生成的项目配置为准。

`#[injectable]` 的具名字段和元组字段也支持条件编译，包括通过 `cfg_attr` 选择
`inject` 或 `value`。例如：

```rust
#[injectable]
struct Diagnostics {
    #[cfg(feature = "audit")]
    #[inject]
    audit: AuditLog,
    #[cfg_attr(debug_assertions, value("debug"))]
    #[cfg_attr(not(debug_assertions), value("release"))]
    mode: &'static str,
}
```

只有当前配置启用的字段参与类型改写、实例构造与依赖图。未启用的字段不要求其
类型存在，也不会执行其 `value` 表达式或占用注入槽位。条件由目标 crate 的 rustc
统一处理；应用不需要额外导入条件编译辅助宏。

普通 `#[derive(...)]` 仍遵循 Rust 的类型要求。含注入字段的结构体会被改写，不能
因为原始字段类型实现了 `Clone`、`Default` 或 `Debug`，就假设整个服务也能派生这些
trait。优先把这类派生放在普通数据类型上，服务只按业务需要实现 trait。

crate/module 路径别名可用，例如 `use nestrs as declarations` 后使用
`#[declarations::injectable]`。单个宏重命名（如 `injectable as component`、
`factory as build_service`）也有[真实编译正例](../cargo-nestrs/tests/fixtures/di/tests/ui/macro-pass/renamed-macro-imports.rs)。
这不表示任意重命名后的多属性组合均受支持；特别是与 `primary` 组合时，需要保留
约定的属性末段名称，不能依赖特定展开顺序。完整路径 `#[nestrs::injectable]`
与常规短名称是推荐写法。

业务测试使用 `cargo nestrs test`，只执行文档示例时使用 `cargo nestrs test --doc`。
文档示例可以使用业务库的服务，也可以直接声明 `#[injectable]`、`#[factory]`、普通
trait impl 和闭合泛型查询。每段示例独立经过完整编译流程，自动绑定与查询根收集
规则和应用一致；成功编译后才由真实 rustdoc 按代码块设置运行。
普通方法、trait impl 方法、宏生成项与 `#[doc = include_str!(...)]` 中的示例均有回归。
示例中的相对 `include!`、`include_str!` 和 `include_bytes!` 仍以原文档所在目录
读取；通过 `#[doc = include_str!(...)]` 引入文档时，以实际 Markdown 目录为准。

### 跨 crate 组织服务

接口、实现和消费者可以分别放在不同 crate；应用通过同一套 `cargo nestrs` 构建，
并使用同一份兼容的 core。编译器汇总最终入口实际依赖的 crate，不会扫描磁盘上
所有 workspace 成员。接口库可以不依赖 Nestrs；使用声明属性的服务库直接依赖
`nestrs-core`，由工具提供 `nestrs` extern。

```text
order-contracts       公开接口，可以不依赖 core
order-infrastructure  依赖 contracts + core，声明具体服务
order-app             依赖上述业务库 + core + tokio，查询接口
```

接口库只需要普通 Rust trait：

```rust
// order-contracts/src/lib.rs
pub trait OrderStore: Send + Sync {
    fn backend(&self) -> &'static str;
}
```

实现库可以把 concrete 和所在模块都保持私有：

```rust
// order-infrastructure/src/lib.rs
mod storage {
    use nestrs::injectable;
    use order_contracts::OrderStore;

    #[injectable]
    struct PostgresOrderStore;

    impl OrderStore for PostgresOrderStore {
        fn backend(&self) -> &'static str {
            "postgres"
        }
    }
}
```

应用只命名公开接口，并明确引用提供实现的库：

```rust
// order-app/src/main.rs
use nestrs_core::ServiceProvider;
use order_contracts::OrderStore;
use order_infrastructure as _;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::build(None).await?;
    let store = provider.get_required_service::<dyn OrderStore>().await?;
    assert_eq!(store.backend(), "postgres");
    provider.dispose_async().await?;
    Ok(())
}
```

若应用已使用实现库的公开类型或函数，通常已有 crate 引用；仅在 manifest 列出一个
完全未使用的依赖，不应当作“这个库必定贡献服务”的保证。运行时无需注册调用，
实现库也不必为下游额外写一条查询或 `bind`。另一个消费者库只需依赖 contracts 与
core，并在字段或构造参数请求 `dyn OrderStore`；它不需要依赖具体存储库。
单独编译消费者 library 不要求应用图完整，最终 binary/test 才汇总所有候选并验证。

在这个 workspace 中从应用入口运行：

```sh
cargo nestrs check -p order-app
cargo nestrs run -p order-app
cargo nestrs graph -p order-app
```

跨 crate 的选择规则与本地一致：key 精确匹配，primary 只解决同 key 的接口候选，
optional 不隐藏歧义和生命周期错误，同一 provider 的多个接口共享生命周期缓存。
不同 crate 中同名类型保持不同身份。没有任何需求的接口不会仅因存在多个潜在实现
而产生歧义；已经注册的 concrete 服务仍全部参与图检查。

工具在实现所属 crate 的合法位置生成真正的 trait coercion，下游不需要访问私有
concrete；不会提升业务可见性或伪造 vtable。声明级预热策略、lazy 参数、key 与
constructor 输入随上游 metadata 保留，而全局 `[nestrs-cli]` 默认值来自最终入口
所属 package，不继承依赖库设置。

跨 crate 同样只处理有限闭合类型，支持可确定的泛型参数、关联类型与父接口。
`for<'a>` 父接口按真实绑定范围求解，不改成 `'static`。额外的 `Unpin`、
`UnwindSafe` 等 auto trait 会形成不同请求类型；上游没有提供其精确投影且 concrete
不可见时，下游无法凭空生成该能力。开放 `impl<T>` 也不会枚举所有可能的 `T`。
内部发现和可见性细节见[rustc 集成指南](NESTRS_RUSTC_EXTENSION_GUIDE.md)。

真实多 crate 示例见 [cross-crate-binding fixture](../cargo-nestrs/tests/fixtures/README.md#关键契约的阅读位置)。
它包含多个合法入口和一个故意歧义的入口，应按说明选择 binary；直接测试所有入口
会包含预期编译失败。完整验证可运行 `python3 tools/verify-cross-crate-binding.py`。

## 11. 遇到错误时先区分发生阶段

| 阶段 | 会检查什么 | 常见处理方式 |
| --- | --- | --- |
| `cargo nestrs check/build` | 宏、Rust 类型与借用；最终 binary / test 的全部注册依赖结构，包括缺失、重复、歧义、环和生命周期 | 修正类型约束，补齐 provider、key 或 primary，调整依赖关系 |
| `cargo nestrs graph` | 执行选定入口的 Cargo check，从同一编译计划的 sidecar 生成 HTML | 根据编译诊断修正声明；不会运行目标程序 |
| `ServiceProvider::build(options).await` | 首次装配并共享已验证执行计划，建立独立容器状态；按声明策略与 root 配置选择 Singleton 初始化入口 | 检查 Tokio 环境与初始化错误 |
| `provider.create_scope(options).await` | 按本次 scope 策略初始化 Scoped；失败时关闭未交付 scope 并返回 `ScopeBuildError` | 检查初始化与可能的清理错误 |
| 实际查询、root / scope 创建时初始化 | 运行工厂和创建实例，可能遇到外部资源故障 | 检查连接配置及初始化错误 |

图结构错误在最终入口编译时报告，Lazy 下未被使用的已注册服务也参与检查。单独
编译 library 只贡献声明和查询摘要，由最终入口检查完整应用。工厂实际创建失败则
返回初始化错误：查询失败表现为 `ResolveError`，root 创建失败表现为
`BuildError::Initialization`，scope 创建失败表现为 `ScopeBuildError`，后二者均保留
初始化错误与可能的关闭错误。包括 Lazy 下显式 `#[lazy(false)]` 的初始化任务；
可选查询不会忽略这些错误。

`build(None)` 不重新收集服务声明或选择接口实现。首次装配仍调用工具生成的执行适配
回调，取得目标程序中的真实类型身份与构造、投影函数地址；这些回调不运行服务
构造。后续 build 共享计划，但每个 root 的实例缓存与关闭状态独立。容器使用应用
当前的 Tokio runtime；图验证成功也不能保证数据库等外部资源初始化成功。

当前 `cargo nestrs graph` 还要求选定 binary 所属的应用直接依赖 `nestrs-core`，如
本文示例的 manifest。只通过业务库间接使用 core 的应用可以正常 check/build/run，
但尚不满足 graph 命令的入口条件；这是图导出当前的限制。

单独查询一个未注册类型，并不等于已经声明了这个服务；必选查询执行时返回未注册
错误，可选查询执行时返回 `None`。如果这个类型已经被另一个服务声明为必选依赖，
缺失则会使最终入口的 DI 编译失败。

| 现象 | 优先检查 |
| --- | --- |
| build 返回 `CompilerPlanUnavailable` | 应用未生成编译器计划，使用 `cargo nestrs` 构建 |
| 编辑器或普通 Cargo 提示找不到 `nestrs` | 是否通过 `cargo nestrs` 构建、是否执行 `init` 并载入生成配置；不要通过添加公开宏包规避 |
| `#[inject]` 无法识别 | 字段是否属于 `#[injectable]`，参数是否属于 `#[constructor]` / `#[factory]`；它不是独立导出的宏 |
| 未标注的服务字段要求实现 `Default` | 需要注入时加 `#[inject]`，需要普通初值时加 `#[value(...)]` |
| 只注册了 named key，默认查询找不到 | 改用 keyed 查询并传准确的 `ServiceKey` |
| 接口有多个候选 | 用不同 key 区分用途，或为同 key 候选指定恰好一个 primary |
| 重复 provider，即使已有 primary 仍报错 | 同一 concrete 类型和 key 只保留一个创建声明，尤其检查 factory 与 injectable 是否重复 |
| root 查询 Scoped 服务失败 | 建立 scope，通过 `scope.service_provider()` 查询 |
| 工厂返回值借用了参数而编译失败 | 在返回服务中持有自己的数据；不要让构造期借用逃逸 |

Singleton/Scoped 的初始化失败会在所属 owner 中缓存，重复查询不会自动重试；
Transient 下一次消费可重新构造。查询 future 被取消只取消等待，已接受的初始化
仍会继续；容器会注销失效等待者，消费者因其他依赖失败时也会移除无效订阅，不影响
仍在使用共享任务的请求。`LazyInjection` 的后续 `get()` 继续等待该句柄的同一次
初始化，不因为取消一次等待而重新创建 Transient。需要完整失败、取消与关闭契约时，
参阅[core 的失败、取消与关闭](../nestrs-core/README.md#9-失败取消与关闭)。

## 12. 继续阅读和实践

可以运行[电商结账示例](../example/di-checkout/README.md)，观察多个 scope、
keyed 支付、泛型仓库、可选风控以及异步关闭如何组合。它的业务代码只依赖公开
声明和查询入口，适合对照自己的项目组织方式。

日常开发时，先通过 `cargo nestrs check` 检查声明与 Rust 类型，再用
`cargo nestrs graph` 查看实际字段、接口请求和选中实现；需要验证工厂资源初始化
及业务行为时，执行 `cargo nestrs run` 或项目测试。
