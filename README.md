# Nestrs

Nestrs 以 `nestrs-core` 的静态依赖图 DI 为基础，通过 `cargo-nestrs` 提供编译器集成、
自动 trait 绑定和离线 HTML 依赖图。生命周期与 scope 模型参考 ASP.NET Core DI；
服务声明由普通属性过程宏在 Rust 类型检查前展开，运行时先验证完整图，再由 Tokio
非递归地调度实例化。

应用仅依赖 **`nestrs-core`** 和业务所需库，所有编译与编辑器工具由 **`cargo-nestrs`** 管理。
CLI 向编译器和 rust-analyzer 提供同一个私有声明桥接，复用 `cargo-nestrs/src/codegen`。
应用可用 `use nestrs::{injectable, factory, primary};` 和短属性，也可直接写
`#[nestrs::injectable]`；不需要在 Cargo.toml 添加宏库，业务代码不写 bind。
详细命令、工具链 pin、验证层级和当前限制见[工具链说明](docs/NESTRS_CARGO_TOOLCHAIN.md)。
首次编写服务时可先阅读[宏使用指南](docs/NESTRS_MACROS.md)，从完整程序开始了解
服务声明、字段注入、工厂、接口选择和查询宏。

工具链分发、统一安装及未来 bootstrap/create 的讨论状态和待办见
[后续待办计划](docs/NESTRS_TODO_PLAN.md)；下方仍是当前的源码构建方式。
维护容器实现时可从 [core 内部阅读指南](docs/NESTRS_CORE_INTERNALS.md) 进入图编译、
协调器、实例所有权和关闭流程；对应源码包含中文职责说明与安全不变量注释。

在安装匹配的 rustc 与 rustc-dev 后，从本仓库构建并使用工具：

```sh
python3 tools/build-toolchain.py
export PATH="$PWD/target/debug:$PATH"
cargo nestrs doctor
cargo nestrs init
cargo nestrs run -p nestrs-di-example -- sample
cargo nestrs graph -p nestrs-di-example
```

工具链严格核对 Rust release、完整 commit 和 host，不修改全局默认工具链。
应用通过 `cargo nestrs check/build/run/test` 获取私有声明桥接与自动 trait 绑定。
普通 Cargo 不提供这个注入环境。`cargo nestrs init` 为手动组装的现有 Rust 项目
准备 Nestrs 开发环境，生成原版 rust-analyzer 的项目模型和保存检查配置；
`--vscode` 才将设置写入 VS Code。初始化不关闭诊断、不改写应用源码。CLI 的本机适配包括
Linux `x86_64-unknown-linux-gnu` 与 Windows `x86_64-pc-windows-msvc`；两种 host
分别使用匹配的 driver 和过程宏动态库。跨 crate DI 支持上游服务、私有实现投影与
兄弟 crate 需求汇总，具体边界见 [跨 crate DI](docs/NESTRS_CROSS_CRATE_DI.md)；
跨 target 的 graph/IDE 仍有明确支持边界。详见 [工具链说明](docs/NESTRS_CARGO_TOOLCHAIN.md) 与
[IDE 接入](docs/NESTRS_IDE.md)。

Windows 源码构建使用 PowerShell，需要 Rustup、Python 和 Visual Studio C++
Build Tools（含 Windows SDK）。以下命令安装并选择匹配的 Rust 组件：

```powershell
rustup toolchain install 1.98.0-x86_64-pc-windows-msvc --profile minimal --component rustc-dev --component rust-src
$env:RUSTUP_TOOLCHAIN = "1.98.0-x86_64-pc-windows-msvc"
python tools/build-toolchain.py
$env:PATH = "$PWD\target\debug;$env:PATH"
cargo nestrs doctor
cargo nestrs init
cargo nestrs run -p nestrs-di-example -- sample
cargo nestrs graph -p nestrs-di-example
```

Windows 工具产物为 `cargo-nestrs.exe`、`nestrs-driver.exe` 和
`nestrs_tool_bridge.dll`，应一起使用；无需 WSL。工具链身份不匹配时会明确报错，
不能把普通最新 stable 当作固定编译器的替代品。`RUSTUP_TOOLCHAIN` 只选择当前
PowerShell 进程及其子进程的工具链，不修改 `rustup default`。

`cargo nestrs init` 可以重跑以刷新现有开发环境；它不会新建项目、添加 Cargo 依赖，
也不会安装编辑器或工具链组件。默认生成通用的 `rust-project.json` 与
`rust-analyzer-settings.json`；使用 VS Code 时执行 `cargo nestrs init --vscode`。
其他客户端需自行加载这些项目与设置，并使用 rust-analyzer LSP；支持 Rust 的编辑器
不一定使用 rust-analyzer，不能仅凭运行初始化命令就保证完成集成。

未来的 `cargo nestrs create` 将在 `nestrs-bootstrap` 完成后负责创建项目，并在内部
复用初始化能力，直接交付已初始化的项目，用户无需再运行 `init`。`create` 当前尚未实现。

示例目录见 [`example/README.md`](example/README.md)；电商项目位于
[`example/di-checkout/`](example/di-checkout/README.md)，包含并发请求、三种生命周期、
泛型仓库、keyed 支付、业务回滚和异步关闭。每个示例使用独立子目录；故意错误的 DI
声明属于工具测试 fixture，不混入正常业务项目。最小 API 示例已迁入独立回归 fixture：

```sh
cargo nestrs run --manifest-path cargo-nestrs/tests/fixtures/di/Cargo.toml --example di
```

## 声明与查询

应用依赖 `nestrs-core` 和 Tokio，不需要宏库或运行时注册收集库：

```rust
use nestrs_core::{ServiceProvider, get_required_service};
use nestrs::{factory, injectable};

struct Database;

#[factory]
async fn connect_database() -> Result<Database, &'static str> {
    Ok(Database)
}

#[injectable(lifetime = Scoped)]
struct Handler {
    #[inject]
    database: Database,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::build().await?;
    let scope = provider.create_scope();
    let _handler = get_required_service!(scope.service_provider(), Handler).await?;
    // 在此通过 handler 的注入字段调用业务方法。
    scope.dispose_async().await?;
    provider.dispose_async().await?;
    Ok(())
}
```

`#[inject]` 字段由属性宏转为持有强 lease 的只读 token，业务代码保持普通字段写法。token 没有公开构造、Clone、Copy 或可变访问。factory 参数则是只在此次调用期间有效的共享借用，跨 `await` 由真实输入 frame 保活。同步和异步 factory 均支持 `Result<T, E>`，其中 `E: Debug`；错误文本会写入解析诊断，不承诺保存原始错误的完整 `source` 链。

查询只通过 `nestrs-core` 导出的四个宏进行，接收 root provider 或 scope 的 `ServiceProviderRef`：

| 查询宏 | 结果 |
| --- | --- |
| `get_required_service!(provider, T).await` | `Result<&T, ResolveError>` |
| `get_service!(provider, T).await` | `Result<Option<&T>, ResolveError>` |
| `get_required_keyed_service!(provider, T, key).await` | `Result<&T, ResolveError>` |
| `get_keyed_service!(provider, T, key).await` | `Result<Option<&T>, ResolveError>` |

`T` 可以是 concrete 或根据实际接口需求自动绑定的 `dyn Trait`。可选查询只在类型与 key 路由不存在时返回 `None`。生命周期限制、factory 错误及 panic 仍返回 `Err`。`ServiceKey::Named(String)` 和 `ServiceKey::Indexed(usize)` 精确匹配；默认 key 不回退到其他 key。

宏自动借用 provider，每次只求值一次 provider 表达式和 key 表达式。返回引用借用实际 root/scope owner，支持 `get_required_service!(scope.service_provider(), T).await` 的临时视图写法，以及 `tokio::join!` 中的并发查询。引用仍在使用时，编译器禁止消费该 owner 进行 disposal。

`build`、`build_with_options`、`create_scope`、`service_provider`、`warm_up` 和 `dispose_async` 保持普通方法；普通查询方法及独立 `register!` 宏已移除。宏通过文档隐藏的跨 crate 桥接调用查询实现，该桥接属于宏 ABI。

## HTML 依赖图

页面生成属于 CLI：

```sh
# 查看整个 package；页面包含所有 binary 的结果
cargo nestrs graph -p nestrs-di-example

# 查看 workspace 中各 package 的程序入口
cargo nestrs graph --workspace

# 只查看指定入口
cargo nestrs graph -p nestrs-di-example --bin checkout
cargo nestrs graph -p nestrs-di-example --bin checkout --output target/checkout-di.html
```

不带 `--bin` 时导出项目级页面，列出所选 package 的全部 binary；`default-run` 不会
隐藏其他入口。`--workspace` 汇总全部 workspace package，只有 library 的 package
保留在项目清单中，不虚构运行容器。页面可按入口筛选，展示每个入口的图或错误诊断。
总览保留每个入口独立的节点和依赖边，共享的同一 provider 声明标明所属入口；它不把
不同程序的注册集合合成一个容器，也不表示它们共享运行时实例。

每个入口独立编译并校验。一部分入口失败时仍生成含有成功结果和诊断的 HTML，但
命令返回非零退出状态。电商 package 现在只有正常的 `checkout` 入口，所以上面的
`cargo nestrs graph -p nestrs-di-example` 在图校验和文件写入成功时返回退出码 `0`。
非法生命周期声明已迁入 DI 测试 fixture；graph fixture 中的 `invalid_graph` 继续
验证部分失败报告。未启用 `required-features` 的 binary 标为跳过，可使用
`--features` 或 `--all-features` 纳入本次检查。
当前各 package 独立编译，`--workspace --features ...` 会明确拒绝；未写 `-p` 或
`--workspace` 但默认选中多个 package 时，同样拒绝 `--features`。指定特性时请使用
`-p PACKAGE --features ...`。`--workspace --all-features` 与 `--no-default-features`
分别按各 package 处理，不代表复现一次 Cargo workspace 构建中的 feature 合并。

命令编译各 binary 的诊断入口，链接实际应用注册集合，完成图校验后输出 HTML。
它不进入业务 main，不创建服务，也不启动 Tokio runtime。默认页面位于 Cargo target
目录的 `nestrs-di.html`；`--output` 可指定路径。普通容器构建不写图文件，core 中的
`graph_output`、`graph_output_path()` 和 `BuildError::GraphExport` 已移除。

页面内置数据、样式与脚本，不依赖网络。搜索、缩放、平移、节点详情、来源位置、
生命周期、key、factory 参数、trait 实现选择、optional 缺席与重复输入槽位均保留。
大图按局部视图浏览，完整数据仍在页面中，可搜索任意服务并分批展开全部输入。

服务节点表示 provider 声明；接口请求节点显示依赖输入请求的 trait 与 key，并通过
“选中实现”连线指向实际 provider。例如 `orders → dyn OrderStore → Repository<Order>`。
接口请求不创建实例，也没有独立生命周期；页面的服务声明数不包含这些请求节点。
每个请求节点对应一个字段或参数槽位，不以类型显示名称合并不同输入。
每个字段或 factory 参数保留独立的输入连线，Transient 的多次消费仍产生独立实例。
图中的接口请求来自当前依赖输入，不代表全部 trait 实现候选或仅由查询宏使用的路由。
HTML 不展示实例缓存、初始化进度或任意 Rust 模块关系。图结构错误和输出文件错误
由 CLI 返回非零退出；普通运行时的结构错误仍在容器 build 前 panic。

显式 `--bin` 保留单入口模式，图校验失败时不覆盖已有 HTML。项目模式即使全部入口
失败，也会保存诊断报告；没有可列出的 binary、入口选择无效、元数据查询失败或
写入失败仍不覆盖旧文件。只有跳过而没有错误的项目报告退出状态为 0，跳过不等于已验证。

当前 graph 支持本机 binary、源码中可定位的 main（包括 `#[tokio::main]`）。
所选 binary 需直接依赖 core；宏生成 main、`no_main`、lib/test/example 图目标和
跨 target 执行尚未支持。项目页面将不支持的入口列为错误，继续处理其他入口。
本次编译配置未启用的声明、未确定类型实参的开放泛型不会被当成实际服务。

## 泛型根与验证边界

开放泛型 `#[injectable]` 生成 `ProviderDefinition`；通过依赖声明抵达的闭合泛型会在图编译期间展开。查询宏也会在链接期收集根类型，因此即使没有其他服务依赖该泛型，直接查询即可使其在 build 时纳入冻结图：

```rust
let repository = nestrs_core::get_required_service!(provider, Repository<User>).await?;
```

查询宏支持函数体内使用、宏别名和服务类型别名。对具有 `ProviderDefinition` 的类型，根声明提供闭合蓝图；factory-only 类型、普通未注册类型及 trait 查询不会因此被要求实现该 trait。重复根会去重，并保留蓝图自己的 lifetime/key；精确类型与 key 的显式 provider 优先。动态 key 只在查询时求值，不会创建新 provider、改变蓝图的 key 或扩展冻结图。

编译器根据普通 trait impl 和已知闭合类型生成自动投影目录及可选蓝图，图编译器按实际接口需求启用它们。因此，只查询或注入 trait 时也可纳入已确定的闭合泛型实现；不会猜测泛型实参或枚举无限类型空间。绑定只负责类型投影，多个接口继续共享同一 concrete provider 的生命周期与实例缓存。跨 crate 时，私有实现的投影在所属 crate 生成，最终程序汇总需求；兄弟 crate 的重复自动投影幂等合并，详见 [跨 crate DI](docs/NESTRS_CROSS_CRATE_DI.md)。

当前链接单元中的所有查询根保守合并；未执行分支里的宏同样会贡献类型声明，Eager 会预热其中的 Singleton。它们不与某一个 provider 变量绑定。required/optional 仍是查询语义：一个没有 provider 的 required 查询不会仅因出现在源码中就导致 build panic，实际查询时才返回缺失错误；optional 查询返回 `None`。

宏的服务类型必须是调用点能够独立命名的具体类型，不能引用外层泛型函数的 `T`、const 泛型参数或外层 `impl` 的 `Self` 来生成静态根。在 `impl Value` 的方法中应写 `Value` 或指向它的具体类型别名，而非 `Self`。容器构建完成后，不再读取注册清单或物化新泛型。

构建入口在任何 constructor、factory、`Default`、`#[value]` 或 cleanup 执行前验证全部注册，包含 Lazy 模式下未使用的服务。缺失依赖、重复 concrete provider、重复/孤立 binding、trait 歧义、非法槽位、环和生命周期闭包冲突会立即 **panic**，诊断带类型、key、字段/参数及来源位置。多个 trait 候选必须恰有一个 `primary`；它不解决 concrete 重复，也不跨 key 选择。

optional 没有候选时冻结为空输入；有候选时仍完整检查。Singleton 可以依赖 Transient，但其完整激活依赖闭包不能包含 Scoped，factory 参数也遵守这一规则。依赖 Scoped 的 Transient 必须从 scope 获取。

Nestrs 编译器与 Rust 类型系统确认类型化构造契约，图编译确认当前链接单元的完整结构。**结构验证成功不能保证数据库连接等外部资源初始化成功**；这些失败在实际激活时返回错误。`BuildError` 表达没有当前 Tokio runtime 和 Eager 初始化失败。`cargo nestrs check/build` 检查 Rust 声明及生成代码；全图结构校验由容器 build 或 graph 诊断入口执行。

## 生命周期和调度

默认 Lazy，仅实例化请求所需的依赖闭包。`ServiceProviderOptions` 提供 `initialization: InitializationMode::Eager`，提前初始化所有 Singleton 及必要依赖；scope 创建同步且不实例化服务，`scope.warm_up().await` 提前初始化该 scope 的全部 Scoped 服务及其依赖。Transient 仍按每次消费创建，每个重复输入槽位都得到独立实例。

Singleton 在整个 root 及 scopes 间共享，Scoped 在各 scope 隔离，直接查询的 Transient 由查询 owner 保持到关闭。Singleton 总在 root 上下文构造，不受发起查询的 scope 影响。

一个 root 对应一个中央协调任务。迭代展开任务后，已满足依赖的节点进入就绪队列；节点完成就推进后继，无整层等待屏障。worker 只构造当前节点，不递归调用 resolver。所有查询和 scopes 共享默认 **32** 个构造名额，可用 `NonZeroUsize` 配置 `max_concurrent_activations`。依赖等待不占构造名额；cleanup 使用独立受跟踪的 worker。

库使用应用当前的 Tokio runtime，不创建嵌套 runtime。单线程 runtime 支持异步并发，多线程 runtime 支持任务并行。短同步构造在 worker 中执行；阻塞初始化由用户显式使用 `spawn_blocking` 等机制安排。

## 失败、取消和关闭

Singleton/Scoped 初始化失败缓存至 owner 关闭，后续查询共享失败记录，不自动重试。Transient 失败只影响该次 occurrence。worker panic 转为带上下文的 `ResolveError`，协调器继续服务；失败依赖的消费者不执行构造。Lazy 失败保留其他请求可能使用的成功实例；Eager 失败会先关闭未交付的容器、清理成功实例，再返回 `BuildError`。

失败原因和依赖路径按不可变记录共享；短期请求的路径随其错误引用释放，不会持续追加到 Singleton 的缓存失败中。Transient 的激活任务在完成通知后回收，成功实例仍由 owner 的实例日志保活至关闭；失败的 Transient 不会留下等待 owner 关闭的终态任务。

取消查询只取消等待，协调器已经接受的初始化继续执行并收纳结果。关闭开始后拒绝新解析，先处理已接受任务，再按每个 owner 的实例成功发布时间逆序清理。每个 owner 同时仅运行一个 cleanup，完成 hook 并释放本项后才推进下一项；root 等全部 scopes 清理结束再清理自身。现有无参数 async cleanup 对每个成功发布的实例执行一次，panic 汇总为 `DisposeError`，其余 cleanup 继续。

多个 scope 并发关闭时，排队释放不等于析构完成。如果 owner 释放的是最后一个实例 lease，关闭流程会等待该实例及其同步触发的依赖析构结束，再推进本 owner 的下一项 cleanup。析构 panic 归入发起该次释放的 owner，不会记到正在执行共享释放队列的其他 scope 中。

`dispose_async(self).await` 等待完整关闭。取消其等待也不会取消已发起的关闭。普通 `Drop` 非阻塞地向已有协调器发出幂等关闭请求；运行时仍活跃时尽力异步清理，Tokio 已退出时只能同步安全释放，不能保证执行 async cleanup。库不会在 Drop 中创建 runtime 或 `block_on`。

注入 token 即使被安全代码截留，或在消费者 Drop 中移出，仍保活原实例及必要依赖。它不会阻塞逻辑关闭；关闭后不再承诺服务业务可用。最终内存释放由独立于 Tokio 的迭代队列处理，避免深依赖的嵌套 Drop 造成框架递归栈增长。

本轮没有自动超时或强制终止策略。不返回的 factory 或 cleanup 会使显式关闭持续等待。

## 工程边界和验证

`nestrs-core` 承担 DI、图校验与 Tokio 调度；CLI 负责私有声明桥接、
自动绑定、每入口注册清单生成、Cargo 编排、编辑器模型与 HTML 页面。内部桥接不是用户依赖或独立发布的
API，代码生成只有一份。core 不依赖编译工具，应用运行时不链接工具实现。`zyn`
保留为生成后端的编译期基础依赖；没有新增 runtime crate。
`nestrs-bootstrap`、动态注册、运行期扩图及集合解析不在当前范围。

普通源码检查与自动绑定后的运行验证分别运行：

```sh
cargo check -p nestrs-core -p cargo-nestrs -p nestrs-tool-bridge --all-targets
cargo test -p nestrs-core -p cargo-nestrs -p nestrs-tool-bridge
cargo clippy -p nestrs-core -p cargo-nestrs -p nestrs-tool-bridge --all-targets -- -D warnings
cargo fmt --check

cargo nestrs check --workspace --all-targets
cargo nestrs test --workspace
cargo nestrs test --manifest-path cargo-nestrs/tests/fixtures/di/Cargo.toml --all-targets
python3 tools/verify-macro-toolchain.py --skip-build
python3 tools/verify-ide.py --skip-build
python3 tools/verify-graph.py --skip-build
```

原 runtime 回归和 52 个 UI 用例保留在独立 fixture，并补充普通属性误用与导入回归。
IDE 验证器调用已安装的原版 rust-analyzer，检查冷启动诊断、字段/工厂类型、
补全、定义跳转、未保存编辑、真实错误与恢复，以及 feature 和 build.rs 配置。
服务借用、factory frame、取消、关闭和逃逸 lease 的安全契约继续由真实运行及编译
失败用例验证。历史探针与薄宏迁移过程保留在 [工具链方案](docs/NESTRS_COMPILER_TOOLCHAIN_PLAN.md)
关联文档中；当前使用方式以 [Cargo 工具链说明](docs/NESTRS_CARGO_TOOLCHAIN.md) 为准。
