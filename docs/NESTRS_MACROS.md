# Cargo Nestrs 宏使用指南

Nestrs 用属性声明服务、用字段或函数参数声明依赖，再通过查询宏取得服务。
你只需要描述“这个服务怎么创建、依赖谁、活多久”，容器负责验证依赖关系和管理实例。

本文面向编写 Nestrs 应用的开发者，从一个能运行的订单通知程序开始，介绍当前支持的
宏写法。工具链安装见[工具链说明](NESTRS_CARGO_TOOLCHAIN.md)，编辑器配置见
[项目初始化与 rust-analyzer 接入](NESTRS_IDE.md)。

## 1. 先认识两组宏

| 写法 | 用途 | 从哪里使用 |
| --- | --- | --- |
| `#[injectable]` | 让容器根据结构体字段创建服务 | `use nestrs::injectable;` |
| `#[factory]` | 用同步或异步函数创建服务 | `use nestrs::factory;` |
| `#[primary]` | 同一个接口有多个候选时，指定优先实现 | `use nestrs::primary;` |
| `#[inject]` | 结构体字段需要另一个服务；也可指定工厂参数的 key | 声明内的辅助属性，不单独导入 |
| `#[value(表达式)]` | 为结构体中的普通字段提供初始值 | 声明内的辅助属性，不单独导入 |
| `get_required_service!` 等四个查询宏 | 从 root 或 scope 取得服务 | 从 `nestrs_core` 导入 |

应用的 `Cargo.toml` 直接依赖 `nestrs-core`，无需添加 `nestrs-macro`、`nestrs-codegen`
或 `linkme`。`nestrs` 是 `cargo nestrs` 为应用提供的属性命名空间，不需要再添加一个
名为 `nestrs` 的 Cargo 依赖。

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

后文使用导入后的短名称。辅助属性也支持 `#[nestrs::inject]` 和
`#[nestrs::value(...)]`，但不要写 `use nestrs::{inject, value};`：它们不能脱离
`#[injectable]` 或 `#[factory]` 单独工作。

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
use nestrs_core::{ServiceProvider, get_required_service};

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
    let provider = ServiceProvider::build().await?;
    let scope = provider.create_scope();

    let orders = get_required_service!(scope.service_provider(), OrderService).await?;
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
默认构建方式是 Lazy：`build` 先验证完整图，第一次取得 `OrderService` 时才创建所需
实例。`graph` 会验证并导出静态依赖图，不执行邮件工厂或业务入口。

## 3. 用 injectable 声明服务

### 3.1 每个字段选择一种初始化方式

`#[injectable]` 支持具名字段结构体、元组结构体和单元结构体。服务声明必须放在
模块作用域，不能放进函数体或局部代码块；业务方法写在普通 `impl` 中。

| 字段写法 | 容器如何处理 |
| --- | --- |
| `#[inject] dependency: SomeService` | 解析必选服务 |
| `#[inject] port: dyn SomeTrait` | 根据接口和 key 选择实现 |
| `#[inject] optional: Option<SomeService>` | 没有注册时注入 `None` |
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

`#[value("文本")]` 可以初始化 `String`；数字、函数调用和代码块也可以作为表达式。
普通调用或代码块的结果应匹配字段类型。它不是依赖查询，也不在编译期或图验证时
求值。需要异步初始化、读取其他服务或返回初始化错误时，使用 `#[factory]`。

一个字段只能选择一种策略，不能同时写 `#[inject]` 和 `#[value(...)]`。
不带属性的字段必须实现 `Default`；若希望它来自容器，请明确写 `#[inject]`。

注入字段按只读共享引用使用，可以直接调用 `self.notifier.notify(...)`。
需要传给接收 `&T` 的函数时，可以显式写 `&*self.dependency`。这些字段由宏改写为
容器管理的只读注入类型，不应当作可直接移动或可变借用的普通 `T`；业务可变状态
应使用 `Mutex`、`RwLock` 或原子类型等同步机制。

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

返回 `Result` 时，错误类型 `E` 需要实现 `Debug`。错误会成为服务初始化错误；
服务不会以失败的值发布。异步工厂返回的 future 需要满足 `Send`，跨 `await` 时
不要持有非 `Send` 的值或普通 Mutex guard。

### 4.1 工厂参数就是依赖

第一个例子的参数 `config: AppConfig` 不需要写 `#[inject]`。每个工厂参数默认就是
必选依赖；`Option<T>` 表示可选依赖，`dyn Trait` 表示接口依赖。需要 key 时才添加
辅助属性，例如：

```rust
#[factory]
fn notifier_label(#[inject(key = "mail")] notifier: dyn Notifier) -> String {
    notifier.notify(0)
}
```

这是参数写法示例：使用它前需要注册 key 为 `"mail"` 的 `Notifier` 实现，不能直接
把它加入只有默认 key 的第一个程序后就期望图验证成功。下一节给出完整的 keyed 例子。

源码参数按 `config: AppConfig` 写，容器会把它作为**此次构造期间有效的共享借用**
传入，跨 `await` 也有效。不要手写成 `&AppConfig`，不要把这个借用保存进返回的服务，
或传给 `tokio::spawn` 这样要求 `'static` 的独立任务。需要长期持有配置内容时，
复制所需数据，正如例子中的 `config.sender.clone()`。

工厂不支持 `#[value]` 参数；普通常量或计算值直接写在函数体中。也不需要给工厂
返回的 `Client` 再加 `#[injectable]`：同一具体类型、同一 key 同时有两个 provider
会导致图验证失败。通常由容器调用工厂，业务入口通过查询宏取得服务。

### 4.2 当前函数约束

工厂必须是模块作用域的普通函数，不是 `impl` 方法或函数体内的局部函数；可以是
同步或异步函数，但不能是 `unsafe fn`、`extern fn` 或带泛型参数的函数。
参数使用简单名称，避免解构参数。返回值必须是具体服务，不能是 `()`。

工厂函数会被宏调整为所在模块私有，即使源码写了 `pub` 也不会成为跨模块的公开
构造接口。对外暴露服务类型及业务方法，调用方通过容器查询取得服务。

为了让宏识别成功类型，使用直接的 `Result<T, E>`、`std::result::Result<T, E>` 或
`core::result::Result<T, E>`；不要用自定义 Result 别名期待相同的错误解包行为。
需要泛型服务时，优先使用泛型 `#[injectable]` 或返回某个明确闭合类型的非泛型工厂。

## 5. 接口、key 和 primary 怎么配合

### 5.1 接口绑定由普通 impl 提供

当服务字段或查询宏请求 `dyn Notifier` 时，CLI 会根据已有服务声明和普通
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
use nestrs_core::{ServiceKey, ServiceProvider, get_required_keyed_service, get_required_service};

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
    #[inject(key = "mail")]
    mail: dyn Notifier,
    #[inject(key = "sms")]
    sms: dyn Notifier,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::build().await?;
    let service = get_required_service!(provider, NotificationService).await?;
    assert_eq!(service.mail.channel(), "mail");
    assert_eq!(service.sms.channel(), "sms");

    let channel = "mail".to_owned();
    let notifier = get_required_keyed_service!(
        provider,
        dyn Notifier,
        ServiceKey::Named(channel),
    ).await?;
    assert_eq!(notifier.channel(), "mail");
    provider.dispose_async().await?;
    Ok(())
}
```

注意两种 key 写法的区别：

| 位置 | 字符串 key | 整数 key |
| --- | --- | --- |
| 声明或注入属性 | `key = "mail"` | `key = 7` |
| 查询宏第三个参数 | `ServiceKey::Named("mail".to_owned())` | `ServiceKey::Indexed(7)` |

属性只接受非空字符串或可表示为 `usize` 的非负整数字面量，不能写运行期变量。
`#[inject("mail")]`、`#[inject(7)]` 也是支持的简写；推荐使用显式的 `key = ...`。
查询宏可以接收运行期计算出的 `ServiceKey`，但不会创建新的注册。

无 key、字符串 `"7"`、整数 `7` 是三种不同选择；默认查询不会回退到 named/indexed
key。`primary` 也不会跨 key 选择。

### 5.3 用 primary 选择同 key 的接口实现

同一个 trait、同一个 key 只有一个候选时直接使用它；有多个候选时，需要恰好一个
候选被标为 `#[primary]`。以下也是可独立运行的程序：

```rust
use nestrs::{injectable, primary};
use nestrs_core::{ServiceProvider, get_required_service};

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
    let provider = ServiceProvider::build().await?;
    let notifier = get_required_service!(provider, dyn Notifier).await?;
    assert_eq!(notifier.channel(), "mail");
    provider.dispose_async().await?;
    Ok(())
}
```

`#[primary]` 也可以和 `#[factory]` 配合，写在声明属性的上方或下方均可；不接收
参数。它只解决接口的多实现选择，不覆盖同一 concrete 类型与 key 的重复声明，
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

工厂参数也可写 `audit: Option<dyn AuditSink>`，查询宏则使用 `get_service!` 或
`get_keyed_service!`。可选只表示“允许没有候选”：存在候选时，依赖歧义、循环、
生命周期冲突和初始化失败仍然是错误，不会被吞掉转为 `None`。

## 7. 查询服务：四个宏都需要 await

| 调用 | 返回结果 |
| --- | --- |
| `get_required_service!(provider, T).await` | `Result<&T, ResolveError>` |
| `get_service!(provider, T).await` | `Result<Option<&T>, ResolveError>` |
| `get_required_keyed_service!(provider, T, key).await` | `Result<&T, ResolveError>` |
| `get_keyed_service!(provider, T, key).await` | `Result<Option<&T>, ResolveError>` |

`T` 可以是具体类型、闭合泛型类型或 `dyn Trait`。第一个参数使用 root provider
或 `scope.service_provider()`，不要直接传 `scope`。provider 表达式和 key 表达式
都只求值一次，查询不会消费 provider。

以下片段放在第一个程序的 `scope.dispose_async()` 之前即可，复用已有 `scope`：

```rust
let view = scope.service_provider();
let orders = nestrs_core::get_required_service!(view, OrderService).await?;
let notifier = nestrs_core::get_service!(view, dyn Notifier).await?;
assert!(notifier.is_some());
assert!(orders.submit(1002).contains("1002"));
```

返回引用借用实际 root 或 scope。使用引用期间不能消费对应 owner 进行关闭；
最后一次使用服务后，再调用 `scope.dispose_async().await?` 和
`provider.dispose_async().await?`。

查询入口只有宏，没有普通的 `.get_required_service::<T>()` 方法，也不需要
`register!`。`build`、`create_scope`、`service_provider`、`warm_up` 和
`dispose_async` 仍然是普通方法。

## 8. 泛型服务：在使用处写出具体类型

`#[injectable]` 支持泛型结构体。例如以下完整程序按需取得订单仓库：

```rust
use std::sync::Mutex;
use nestrs::injectable;
use nestrs_core::{ServiceProvider, get_required_service};

struct Order {
    id: u64,
}

#[injectable]
struct Repository<T: Send + Sync + 'static> {
    records: Mutex<Vec<T>>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::build().await?;
    let orders = get_required_service!(provider, Repository<Order>).await?;
    orders.records.lock().unwrap().push(Order { id: 1001 });
    assert_eq!(orders.records.lock().unwrap()[0].id, 1001);
    provider.dispose_async().await?;
    Ok(())
}
```

字段依赖或查询宏中出现的 `Repository<Order>` 会让这个闭合类型在 `build` 前进入
静态声明；即使查询写在 `build` 后面，也不需要预先调用注册函数。
`Repository<Order>` 和 `Repository<User>` 是两个不同的服务类型，各自遵循声明的
生命周期。框架不会枚举所有可能的 `T`。

查询宏的类型必须能在调用处独立命名，不能捕获外层泛型参数 `T`、const 泛型参数，
或 `impl` 的 `Self`。例如在 `impl OrderService` 内查询自身时写 `OrderService`，
不要写 `Self`。一个通用的 `async fn load<T>(...)` 不能靠查询宏动态注册任意 `T`；
应在已知具体类型的调用点查询，或把已取得的服务引用传给泛型业务函数。

已编译、已链接但没有执行的分支里的查询宏也会贡献类型声明；被 `#[cfg]` 排除的
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
是包含函数路径的字符串；回调写成无参数、返回 `()` 的异步函数，不接收 `self` 或
实例参数。它适合无参数的异步收尾；实例自身持有的同步资源可通过普通 Rust `Drop`
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

普通 `#[derive(...)]` 仍遵循 Rust 的类型要求。含注入字段的结构体会被改写，不能
因为原始字段类型实现了 `Clone`、`Default` 或 `Debug`，就假设整个服务也能派生这些
trait。优先把这类派生放在普通数据类型上，服务只按业务需要实现 trait。

声明属性推荐保留 `injectable`、`factory`、`primary` 的名称。特别是与 `primary`
组合时，不要将它们重命名为 `service`、`make` 等别名来依赖特定展开顺序。
完整路径 `#[nestrs::injectable]` 与常规短名称是推荐写法。

当前自动绑定以同一个 crate 的服务与接口需求为主要支持范围。不要假设所有依赖
crate 中的私有实现都会被全局搜集；尤其是接口只在下游被请求、实现位于上游的
情况。跨 crate 设计时，先用实际注入或查询路径验证，并阅读
[自动绑定边界](NESTRS_CARGO_TOOLCHAIN.md#自动绑定元数据与缓存)。

业务测试使用 `cargo nestrs test`。标准声明宏可用于 rustdoc/doctest，但独立 doctest
中新声明的接口自动绑定不经过完整的绑定流程；包含 trait 注入的完整用例优先放进
普通单元测试或集成测试，通过 `cargo nestrs test --tests` 验证。

## 11. 遇到错误时先区分发生阶段

| 阶段 | 会检查什么 | 常见处理方式 |
| --- | --- | --- |
| `cargo nestrs check/build` | 宏写法、Rust 类型与借用、生成代码和接口投影是否合法 | 修正声明、类型约束或借用方式 |
| `ServiceProvider::build()` 或 `cargo nestrs graph` | 全部注册的依赖结构，包括缺失依赖、重复、歧义、环和生命周期冲突 | 补齐 provider、key 或 primary，调整依赖关系 |
| 实际查询、Eager 初始化或 scope 预热 | 运行工厂和创建实例，可能遇到外部资源故障 | 检查连接配置及初始化错误 |

运行时 `build` 遇到图结构错误会在任何服务构造前 panic，Lazy 下未被使用的服务也
参与检查。工厂实际创建失败则返回初始化错误：Lazy 查询表现为 `ResolveError`，
Eager 构建失败表现为 `BuildError`。可选查询不会忽略这些错误。

单独在查询宏里写一个未注册类型，并不等于已经声明了这个服务，也不必然让 `build`
失败；必选查询执行时返回未注册错误，可选查询执行时返回 `None`。如果这个类型
已经被另一个服务声明为必选依赖，缺失则会在全图验证时被发现。

| 现象 | 优先检查 |
| --- | --- |
| 编辑器或普通 Cargo 提示找不到 `nestrs` | 是否通过 `cargo nestrs` 构建、是否执行 `init` 并载入生成配置；不要通过添加公开宏包规避 |
| `#[inject]` 无法识别 | 外层是否有 `#[injectable]` 或 `#[factory]`；它不是独立导出的宏 |
| 未标注的服务字段要求实现 `Default` | 需要注入时加 `#[inject]`，需要普通初值时加 `#[value(...)]` |
| 只注册了 named key，默认查询找不到 | 改用 keyed 查询并传准确的 `ServiceKey` |
| 接口有多个候选 | 用不同 key 区分用途，或为同 key 候选指定恰好一个 primary |
| 重复 provider，即使已有 primary 仍报错 | 同一 concrete 类型和 key 只保留一个创建声明，尤其检查 factory 与 injectable 是否重复 |
| root 查询 Scoped 服务失败 | 建立 scope，通过 `scope.service_provider()` 查询 |
| 工厂返回值借用了参数而编译失败 | 在返回服务中持有自己的数据；不要让构造期借用逃逸 |

Singleton/Scoped 的初始化失败会在所属 owner 中缓存，重复查询不会自动重试；
Transient 下一次消费可重新构造。查询 future 被取消只取消等待，已接受的初始化
仍会继续。需要完整失败、取消与关闭契约时，参阅[项目 README](../README.md#失败取消和关闭)。

## 12. 继续阅读和实践

可以运行[电商结账示例](../example/di-checkout/README.md)，观察多个 scope、
keyed 支付、泛型仓库、可选风控以及异步关闭如何组合。它的业务代码只依赖公开
声明和查询入口，适合对照自己的项目组织方式。

日常开发时，先通过 `cargo nestrs check` 检查声明与 Rust 类型，再用
`cargo nestrs graph` 查看实际字段、接口请求和选中实现；需要验证工厂资源初始化
及业务行为时，执行 `cargo nestrs run` 或项目测试。
