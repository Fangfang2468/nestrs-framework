# 电商结账：完整的 Nestrs DI 示例

这个示例把依赖注入放进一段可以实际运行的业务流程：读取商品库存、选择支付渠道、保存订单、生成收据，并在请求结束后关闭 scope。两个并发请求共享库存与订单存储，保留各自的请求上下文。

本项目根目录为 `example/di-checkout/`，Cargo package 名称保持 `nestrs-di-example`，唯一程序入口为 `checkout`。父目录 `example/` 只组织多个示例，完整索引见[示例目录](../README.md)。

数据库、支付客户端和初始化延迟都在进程内模拟。订单和审计记录保存在仓库的内存表中，退出进程后不会持久保留。运行示例不需要数据库服务、HTTP 服务或支付账户，也不会产生真实扣款。

## 运行

先按[仓库 README](../../README.md)构建匹配的 CLI、driver 与私有桥接库，并将 CLI 加入 PATH，然后在仓库根目录执行：

```bash
cargo nestrs run -p nestrs-di-example
```

也可以进入本目录运行默认的 `checkout` 二进制：

```bash
cd example/di-checkout
cargo nestrs run
```

示例复用应用的 Tokio runtime。默认采用 Lazy 模式，只在查询服务时激活所需的依赖。下面的参数用于对照不同初始化方式：

```bash
# 预先初始化所有 Singleton 及其必要依赖
cargo nestrs run -p nestrs-di-example -- --eager

# 建立 scope 后，先预热该 scope 的 Scoped 服务
cargo nestrs run -p nestrs-di-example -- --warm-up-scopes

# 将整个 provider 的构造并发上限设为 1，观察初始化顺序
cargo nestrs run -p nestrs-di-example -- --max-concurrency 1

# 参数可以组合使用
cargo nestrs run -p nestrs-di-example -- --eager --warm-up-scopes --max-concurrency 1

cargo nestrs run -p nestrs-di-example -- --help
```

本示例默认使用 **4** 个构造名额和 **4** 个 Tokio worker。`--max-concurrency` 接受正整数，控制服务构造的并发数量；它不限制已经构造好的业务服务同时处理多少个结账请求。Eager 只预热 Singleton；它不会提前创建请求 scope，也不会把 Scoped 或 Transient 变成 Singleton。

## 用浏览器查看实际依赖图

依赖图由 `cargo nestrs graph` 生成。它逐个编译并链接所选入口，进入
编译器生成的图导出入口，读取静态依赖图；不会运行下单入口、构造服务、执行 factory
或 `#[value]` 表达式，也不需要启动 Tokio runtime。

```bash
# 项目总览，当前项目仅包含 checkout
cargo nestrs graph -p nestrs-di-example

# 只看结账应用
cargo nestrs graph -p nestrs-di-example --bin checkout

# 指定 HTML 输出位置
cargo nestrs graph -p nestrs-di-example --bin checkout --output target/checkout-di.html
```

项目图当前只有 `checkout`，因此上述 package 命令在图校验与文件写入成功后返回
退出码 `0`，不需要额外加 `--bin`。显式 `--bin checkout` 可直接查看单入口图。
故意违反 DI 生命周期的程序已迁入工具回归 fixture，不参与本项目的注册或图导出。

上面的命令从仓库根目录执行。从本项目目录导出时，可以省略 package 参数：

```bash
cd example/di-checkout
cargo nestrs graph
```

两种方式选择同一个 package，默认页面都位于仓库根的 `target/nestrs-di.html`，不是
`example/di-checkout/target/`；命令会输出实际文件路径。`--output` 相对于调用目录解析，
例如在本目录内可以用 `--output ../../target/checkout-di.html`。

HTML 包含完整图数据、样式和脚本，不依赖网络、CDN 或本地 Web 服务。业务程序的 `--graph`、`--graph-output`
参数已经移除，`ServiceProviderOptions` 不再负责 HTML 文件输出。

打开项目页面后可以直接浏览总览，也可以选择 `checkout`，然后这样探索结账流程：

1. 搜索 `CheckoutService`，点击节点查看注入字段、生命周期和声明位置。
2. 沿 `orders` 字段查看 `dyn OrderStore → Repository<Order> → Database → AppConfig`；接口节点与服务节点分开，虚线“选中实现”表示接口投影。
3. 查看 `card`、`wallet` 两个字段请求的 `dyn PaymentGateway` 接口节点；它们的 key 不同，分别选择对应的 `PaymentClient` provider。
4. 查看可选风控输入 `FraudCheck` 的缺席状态，以及 Transient `ReceiptFormatter` 的生命周期。
5. 缩放和平移画布，查看 `Repository<AuditEvent>`；它由查询宏贡献闭合类型声明，并与订单仓库共享数据库。

服务节点编号标识 provider 声明，与 `[构造]` 日志里的实例编号不同。接口请求节点
只是展示请求到实现的关系，不创建另一份实例；本例仍为 10 个服务声明，另有 3 个
已解析的接口请求。每个字段或参数保留独立输入槽位与连线，所以图上两条边指向同一
Transient 声明，运行时仍会分别构造两个实例。

图展示当前链接单元的静态服务关系，不表示实例初始化状态或业务资源是否可用。
依赖缺失、歧义、环和非法生命周期会使相应入口校验失败：单入口模式不导出新图，
项目模式则将诊断写入报告并保留其他有效入口。外部数据库或支付资源故障只会在
真正实例化服务时显现。HTML 写入错误由 CLI 报告，不影响普通容器构建。

## 业务场景与预期结果

商品 `KEYBOARD` 初始库存为 **5**，单价为 **19,900 分（199.00 元）**。程序依次执行以下场景：

| 场景 | 输入 | 预期业务结果 | 库存变化 |
| --- | --- | --- | --- |
| 并发请求一 | 数量 1，`card` 支付 | 支付成功，创建订单并返回收据 | 减少 1 |
| 并发请求二 | 数量 2，`wallet` 支付 | 支付成功，创建订单并返回收据 | 减少 2 |
| 支付拒绝 | 数量 1，支付 token 为 `declined` | 返回支付拒绝，不创建订单，归还预留库存 | 净变化为 0 |
| 库存不足 | 数量 99 | 在支付前返回库存不足，不扣款、不创建订单 | 不变 |

Alice 和 Bob 的两个成功请求各有自己的 scope，并由 `tokio::join!` 并发驱动。随后 Carol、Dave 各使用新 scope 顺序执行两个拒绝场景。成功请求的完成顺序、订单编号对应关系与日志交错顺序可能变化，但最终应当得到：

- 成功订单 **2 张**，购买数量分别为 1 和 2。
- 两张订单的金额分别为 **19,900 分**和 **39,800 分**。
- `KEYBOARD` 剩余库存 **2 件**。
- 支付拒绝和库存不足场景没有新增成功订单。
- 四个业务场景各写入一条审计记录，共 **4 条**。

忽略日志时间戳，汇总行应为：

```text
[结果] 成功订单 2 笔；剩余库存 2 件；成交金额 597.00 元；审计 4 条
```

支付拒绝属于业务结果，不等于 DI 初始化失败。库存不足也应由结账流程正常处理，不能通过构造另一个容器来恢复状态。

## DI 如何组成这段业务

图中的实线表示依赖输入，虚线标注接口选中的实现或可选输入的缺席；驱动代码只在请求入口获取业务服务，业务服务通过已注入的字段调用协作者。

```mermaid
flowchart LR
    Driver["CLI 驱动 / scope"] --> Checkout["CheckoutService · Scoped"]
    Checkout --> Context["RequestContext · Scoped"]
    Checkout --> Inventory["Inventory · Singleton"]
    Checkout -- orders --> Store["dyn OrderStore"]
    Store -. "选中实现" .-> Orders["Repository&lt;Order&gt; · Singleton"]
    Checkout -- card --> CardGateway["dyn PaymentGateway<br/>key=card"]
    Checkout -- wallet --> WalletGateway["dyn PaymentGateway<br/>key=wallet"]
    CardGateway -. "选中实现" .-> CardPayment["PaymentClient · Singleton<br/>key=card"]
    WalletGateway -. "选中实现" .-> WalletPayment["PaymentClient · Singleton<br/>key=wallet"]
    Checkout --> Receipt["ReceiptFormatter · Transient"]
    Checkout -. "可选依赖：未注册" .-> Fraud["dyn FraudCheck"]
    Orders --> Database["Database · Singleton"]
    AuditQuery["审计仓库查询宏"] --> Audit["Repository&lt;AuditEvent&gt; · Singleton"]
    Audit --> Database
```

`AppConfig` 也是 Singleton，用于提供模拟资源初始化所需的配置。上图省略了配置依赖，便于观察请求范围与共享服务的边界。

| DI 能力 | 示例中的使用方式 | 阅读时应注意的行为 |
| --- | --- | --- |
| Singleton | `AppConfig`、`Database`、`Inventory`、两个闭合仓库，以及两个 keyed `PaymentClient` | 同一个 root 的多个 scope 共享这些实例；不同 key 的支付客户端仍是两个独立实例 |
| Scoped | `RequestContext`、`CheckoutService` | 同一 scope 中重复获取会复用实例，不同 scope 的请求上下文隔离 |
| Transient | `ReceiptFormatter` | 每次消费或直接获取时创建；注入 Scoped 服务的 formatter 会随那个服务实例复用，不会在每次业务方法调用时自动重建 |
| 异步 factory | 模拟数据库和支付客户端初始化 | 依赖满足后才能启动，互不依赖的初始化可以重叠执行 |
| Trait 注入 | `dyn OrderStore` 绑定到 `Repository<Order>`；`dyn PaymentGateway` 绑定到 `PaymentClient` | 业务代码依赖接口；订单绑定的闭合泛型实现会在图编译期间被纳入 |
| Keyed 注入 | `#[inject(key = "card")]` 与 `#[inject(key = "wallet")]` 的字段均为 `dyn PaymentGateway` | trait 查询继承请求 key，取得同一 concrete 类型的两个独立客户端 |
| Optional 注入 | `Option<dyn FraudCheck>` | 本例不注册该接口，因此字段为 `None`，结账仍能正常运行 |
| 查询宏发现泛型根 | `Repository<AuditEvent>` | 没有业务服务依赖它，也能由查询宏在 build 之前贡献闭合类型声明 |
| 异步关闭 | scope 与 provider 的 `dispose_async` | 请求结束先关闭 scope，最后关闭 root；初始化失败路径也安排关闭 |

`#[inject]` 字段看起来声明的是普通服务类型，Nestrs 编译器会在 Rust 类型检查前把它改写成只读注入 token。共享状态的更新仍由业务服务自己的同步机制负责；DI 不会自动让库存的“检查并扣减”成为原子操作。

`CheckoutService::place_order` 在短 Mutex 临界区内预留库存，释放锁后再等待支付。未提交的 `Reservation` 在支付拒绝、future 取消或展开栈时通过普通 Rust `Drop` 归还库存；保存订单后调用 `commit`。这是业务 RAII，和容器的 cleanup hook 是两条不同的清理路径。

一个 `CheckoutService` 同时声明了两种支付渠道，所以 **Lazy 首次构造它时也会初始化 card 和 wallet**。运行期的 `PaymentMethod` 只决定本次调用哪个已注入的渠道，不会改变静态依赖闭包。

## 声明、构建、查询与执行业务的边界

声明代码从工具提供的 `nestrs` 命名空间导入 `injectable`、`factory`，使用 `#[injectable]`、`#[factory]` 与字段上的 `#[inject]`。CLI 注入内部过程宏桥接库，宏仍通过标准 Rust 展开；应用 manifest 无需配置宏包依赖。CLI 根据普通 `impl Trait for Concrete` 和实际注入或查询需求生成绑定，业务代码不需要 `#[bind]`。应用不手写容器内部实例记录，也不直接依赖 linkme。

应用声明检查使用 `cargo nestrs check -p nestrs-di-example --all-targets`，运行使用
`cargo nestrs run`。普通 `cargo check` 不会注入 Nestrs 的私有声明桥接和自动 trait
绑定环境，不能替代这条构建路径。`cargo nestrs init` 为现有项目初始化或刷新
开发环境，默认生成通用 rust-analyzer 项目模型与宏展开设置，不创建项目或添加依赖。
如果只打开本项目目录，可在该目录执行该命令；使用 VS Code 时加 `--vscode`，
其他 rust-analyzer LSP 客户端仍需自行加载生成配置。编译检查通过不等于容器完整
依赖图已经在运行时核查。

资源 factory 的源码参数写作 `config: AppConfig`。Nestrs 编译器把它改写为构造期间的共享借用，由输入 frame 在 `await` 期间保活；数据库和支付客户端把需要的配置字符串复制进返回实例。

构建容器时，框架先收集当前链接单元的声明，解析 trait/key/泛型依赖，检查缺失依赖、环与生命周期冲突，然后冻结构造计划。这个阶段在任何服务构造、factory 或 `#[value]` 表达式执行之前完成。Lazy 延迟的是实例化，完整图校验仍然先执行。

查询统一使用 core 导出的宏。例如：

```rust
let checkout = nestrs_core::get_required_service!(
    scope.service_provider(),
    CheckoutService,
)
.await?;

let audit = nestrs_core::get_required_service!(
    provider,
    Repository<AuditEvent>,
)
.await?;
```

返回的共享引用借用实际 root 或 scope owner。只要后续仍需使用该引用，Rust 就不允许消费 owner 进行关闭。`build`、`create_scope`、`warm_up` 与 `dispose_async` 保持普通方法；服务查询没有普通方法形式，也不需要额外的 `register!`。

查询宏会在链接期贡献类型声明，所以即使 `Repository<AuditEvent>` 的实际查询写在 `build` 后面，该闭合类型也已经进入构建时的依赖图。当前链接单元中未执行分支里的查询宏也会贡献声明；动态 key 只在运行期选择已冻结的路由，不会扩展图。

查询类型必须能独立命名，不能让静态根捕获外层泛型函数的 `T`、const 泛型参数或 `impl` 的 `Self`。应在具体调用点写 `Repository<AuditEvent>`，或使用指向闭合类型的别名。

容器变量与查询操作留在驱动层。`CheckoutService` 接收业务输入、调用注入的库存/支付/订单服务并返回业务结果；它不会把 provider 传给下游，也不会在方法内部查询容器。

## 输出怎么看

`observe::event` 为日志添加相对时间戳，`observe::created` 打印 `[构造] 类型 #实例编号`。实例编号用于观察对象身份，与业务订单号 `ORD-…` 分开。资源工厂分别模拟 Database **160 ms**、card **100 ms**、wallet **120 ms** 的等待；这些是示例设置的延迟，不是程序总运行时间的保证。

先观察 `factory start Database`、`factory start PaymentClient[card]`、`factory start PaymentClient[wallet]` 及其 `factory end` 事件，再对照请求和关闭事件：

- **Lazy**：build 完成并不表示模拟连接已经初始化。首次查询 `CheckoutService` 才会触发其必要依赖。
- **Eager**：Singleton 初始化出现在 build 返回之前，后续 scope 查询复用已完成的共享实例。
- **Scope 预热**：`warm_up` 使 Scoped 服务在业务调用前准备好，但每个 scope 仍保持自己的 `RequestContext`。
- **并发上限为 1**：需要构造的服务依次取得构造名额。恢复默认上限后，可从时间戳观察独立初始化的重叠。
- **请求日志**：比较请求上下文与共享资源标识，观察“请求隔离、资源共享”；不要把并发日志的固定行序作为正确性要求。
- **业务结果**：沿着 `业务库存预留`、`业务支付成功`、`业务库存提交`、`业务订单保存` 阅读成功路径；`业务支付拒绝` 后应跟随 `业务库存回滚`，最终库存仍为 2。
- **验证日志**：`[验证]` 展示相同 scope 中的实例复用、不同 scope 的隔离、两次直接查询 Transient 得到不同 formatter、`wallet` keyed 查询、`cash` key 与风控的可选缺席，以及 root 查询 Scoped 返回错误。
- **关闭日志**：`[关闭] 请求 A 的 scope 已完成` 等事件标记请求边界，最后是 `[关闭] root 已完成`。`cleanup hook …` 表示异步回调，`drop … #实例编号` 表示对应 Rust 对象的实际析构。

作为一次实际运行的对照，默认并发的 Lazy 模式在 build 完成后约 **162 ms** 准备好结账依赖；使用 `--eager --warm-up-scopes --max-concurrency 1` 时，观察到 card、wallet、Database 依次初始化，root build 约 **384 ms**。这些时间用于说明独立任务重叠与串行构造的区别；它们包含调度开销，不是固定耗时或日志顺序契约。

本例的 cleanup hook 是**无参数的类型级异步回调**，用于演示框架会等待清理阶段。它拿不到具体服务实例，因此日志不代表“已经关闭某个真实数据库连接”。示例中的实例本来就是内存模拟对象；不要据此推断真实连接池的关闭接口已经接入。

## 初始化失败与框架负例

以下环境变量命令使用 Bash / WSL 写法。

### 外部资源初始化失败

```bash
NESTRS_EXAMPLE_FAIL_PAYMENT=1 cargo nestrs run -p nestrs-di-example
```

这个开关让模拟支付客户端初始化失败。预期行为是打印带服务来源的初始化错误，关闭已创建的 scope 与 provider，并以非零状态退出。它不会执行正常的“两张订单”流程。

也可加上 `--eager` 对照：

```bash
NESTRS_EXAMPLE_FAIL_PAYMENT=1 cargo nestrs run -p nestrs-di-example -- --eager
```

Lazy 下，资源错误在首次激活相关服务时显现，驱动会关闭两个已创建的请求 scope，再关闭 root。Eager 下，它在 build 的预热阶段显现；build 负责关闭尚未交付的 provider，此时还没有进入请求流程。**图验证成功只说明依赖结构合法，不保证数据库或支付资源初始化成功。**

### 非法生命周期回归已移入测试 fixture

故意让 Singleton `ApplicationCache` 依赖 Scoped `RequestSession` 的用例位于
[`invalid_lifetime.rs`](../../cargo-nestrs/tests/fixtures/di/src/bin/invalid_lifetime.rs)，由
[`runtime_invalid_lifetime.rs`](../../cargo-nestrs/tests/fixtures/di/tests/runtime_invalid_lifetime.rs)
验证构造前 panic、`构造次数 = 0` 与非零退出结果。它不再是本示例的 binary。

维护框架时，可从仓库根目录单独运行这个预期失败的程序：

```bash
cargo nestrs run --manifest-path cargo-nestrs/tests/fixtures/di/Cargo.toml --bin invalid_lifetime
```

该命令预期以非零状态退出；日常学习业务场景使用本项目的 `checkout` 即可。
工具的 graph fixture 还保留自己的 `invalid_graph`，专门验证项目报告中的部分入口失败，
与本项目的正常图导出分开。

## 建议的代码阅读顺序

1. [src/domain.rs](src/domain.rs)：`CheckoutRequest`、`Order`、`AuditEvent` 与 `CheckoutError`，确认金额以分存储，业务错误与容器错误分开。
2. [src/services/config.rs](src/services/config.rs) 与 [src/observe.rs](src/observe.rs)：故障开关、本地配置、相对时间戳和实例编号。
3. [src/services/storage.rs](src/services/storage.rs)：数据库异步 factory、泛型内存仓库、`OrderStore` 绑定；[src/services/payment.rs](src/services/payment.rs)：两个 keyed factory、`PaymentGateway` 投影与业务支付。
4. [src/services/inventory.rs](src/services/inventory.rs)：并发库存预留和 `Reservation` 回滚；[src/services/checkout.rs](src/services/checkout.rs)：注入字段、Scoped 上下文、Transient formatter 与 `place_order`。
5. [src/demo.rs](src/demo.rs)：`run` 管理 root，`request_scope` 管理每次请求，`checkout_flow` 驱动四个场景。源码先保存业务/查询结果，再等待关闭，以便错误路径同样执行 disposal。
6. [src/main.rs](src/main.rs)：CLI 参数、默认业务入口与进程退出状态。

[tests/checkout.rs](tests/checkout.rs) 覆盖业务与 DI 生命周期行为，[tests/cli.rs](tests/cli.rs) 在独立进程中核对默认不写图，以及初始化失败时的关闭与退出结果。非法 DI 注册位于独立工具 fixture；HTML 命令的导出、输出路径和失败行为由 `cargo-nestrs` 的 CLI 测试覆盖。

从仓库根目录运行本示例的自动化验证：

```bash
cargo nestrs test -p nestrs-di-example --tests
```

如果当前目录已经是 `example/di-checkout/`，则使用 `cargo nestrs test --tests`。

测试检查 Lazy/Eager 下的业务结果、共享与隔离关系，以及初始化失败进程的非零退出。对照运行时可以分别使用默认模式、Eager、scope 预热和 Eager + scope 预热 + 单构造名额组合。非法图的构造次数和退出结果由上述 fixture 回归单独检查。
