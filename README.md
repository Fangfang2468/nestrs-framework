# Nestrs

Nestrs 是以静态依赖图为基础的 Rust DI 框架。服务通过属性声明，`cargo nestrs`
借助 rustc 的真实类型完成自动 trait 绑定、完整依赖图验证和执行计划生成；
`nestrs-core` 加载不可变计划，在 Tokio 上调度服务构造、查询和异步关闭。

应用依赖 `nestrs-core` 与业务库，使用 `cargo nestrs check/build/run/test` 编译。
工具以 `nestrs` extern 提供私有过程宏桥接，应用不在 Cargo.toml 中添加宏 package。
普通 `impl Trait for Concrete` 按实际注入与查询需求参与自动绑定，业务代码无需手写 bind。

## 项目组成

| 位置 | 职责 |
| --- | --- |
| [`nestrs-core/`](nestrs-core/README.md) | 公开 DI API、冻结计划装配、生命周期、实例所有权和执行调度 |
| [`nestrs-config/`](nestrs-config/README.md) | 可独立使用的配置加载、合并、Serde 读取、子树和来源诊断；宏、自动校验与 DI 接入仍待实现 |
| [`cargo-nestrs/`](docs/NESTRS_CARGO_TOOLCHAIN.md) | CLI、声明生成、rustc 适配、IDE 模型及离线 HTML 依赖图 |
| `cargo-nestrs/internal/bridge/` | 工具内部的标准 proc-macro 薄桥接，复用唯一 codegen 后端 |
| [`example/`](example/README.md) | 电商业务项目与独立的预期编译失败示例 |
| [`cargo-nestrs/tests/fixtures/`](cargo-nestrs/tests/fixtures/README.md) | 工具链端到端回归输入，含有意非法声明 |

运行期生态库单向依赖 core。`nestrs-bootstrap`、项目创建和统一安装分发尚未实现；
后续架构边界见 [AGENTS.md](AGENTS.md#10-未来生态与命名)。

## 从源码运行

先准备 [toolchain.json](cargo-nestrs/toolchain.json) 指定的 Rust 与匹配的 `rustc-dev`、
`rust-src`。当前 pin 为 Rust `1.98.0`、完整 commit
`88d9e12ae178fab0fb5cc050a94da85685d449ea`，支持的本机 host 是 Linux GNU x86_64
和 Windows MSVC x86_64。release、commit、host 必须全部匹配。

以下命令在 Linux 仓库根目录执行：

```sh
python3 tools/build-toolchain.py
export PATH="$PWD/target/debug:$PATH"
cargo nestrs doctor
cargo nestrs run -p nestrs-di-example -- sample
cargo nestrs graph -p nestrs-di-example
```

Windows 本机构建步骤、环境变量、缓存和回归命令统一见
[工具链指南](docs/NESTRS_CARGO_TOOLCHAIN.md)。构建脚本不修改全局默认工具链。
`cargo nestrs graph` 读取编译期计划生成离线 HTML，不运行应用或创建服务。

接入已有项目的编辑器环境时执行 `cargo nestrs init`，使用 VS Code 时加 `--vscode`。
它生成原版 rust-analyzer 的项目模型与保存检查配置，不创建项目或安装依赖。
其他 LSP 客户端须加载生成配置；具体步骤和限制见 [IDE 指南](docs/NESTRS_IDE.md)。

## 最小服务程序

应用 Cargo.toml 添加 `nestrs-core` 和启用 `macros`、`rt-multi-thread` 的 Tokio；
在本仓库外使用源码时，把 `nestrs-core` 的 path 指向实际目录。
下面代码保存为 `src/main.rs`，通过 `cargo nestrs run` 执行：

```rust
use nestrs::{factory, injectable};
use nestrs_core::ServiceProvider;

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
    let provider = ServiceProvider::build(None).await?;
    let scope = provider.create_scope(None).await?;
    let _handler = scope
        .service_provider()
        .get_required_service::<Handler>()
        .await?;
    scope.dispose_async().await?;
    provider.dispose_async().await?;
    Ok(())
}
```

`#[inject]` 字段生成持有实例 lease 的 `Injection<T>`，通过只读解引用访问服务。
Singleton 在容器内共享，Scoped 在同一 scope 内共享，Transient 每次解析创建新实例；
root 不接收 Scoped 查询。`#[lazy]` 输入通过 `LazyInjection<T>::get().await` 显式访问；
provider 级 `#[lazy]` 则决定服务是否被选作所属 owner 创建期的自主初始化入口，
普通依赖仍可能提前构造它。root 和 scope 的默认初始化策略独立，均为 Lazy；
配置、服务级覆盖与失败清理见 [core 的初始化说明](nestrs-core/README.md)。
可选注入/查询仅在路由缺席时返回 `None`，不会隐藏歧义、循环或生命周期错误。

构造函数、同步/异步 factory、key、trait、泛型、lazy 和跨 crate 的完整用法统一见
[服务声明与查询](docs/NESTRS_MACROS.md)。引用在使用期间阻止对应 owner 被消费关闭；
应用应显式 `dispose_async().await` 以等待异步 cleanup 完成。

## 阅读入口

| 目标 | 文档 |
| --- | --- |
| 选择阅读顺序 | [文档导航](docs/README.md) |
| 编写服务及查询 | [声明指南](docs/NESTRS_MACROS.md) |
| 理解容器设计、所有权和关闭流程 | [core 架构](nestrs-core/README.md) |
| 理解 rustc 扩展、跨 crate 分析和 reflect 协议 | [编译器原理](docs/NESTRS_RUSTC_EXTENSION_GUIDE.md) |
| 使用 CLI、固定工具链和运行回归 | [工具链指南](docs/NESTRS_CARGO_TOOLCHAIN.md) |
| 配置编辑器 | [IDE 指南](docs/NESTRS_IDE.md) |
| 阅读编译错误并定位源码 | [诊断格式](docs/NESTRS_DIAGNOSTICS_DESIGN.md)与[错误示例](example/di-errors/README.md) |
| 查看有基线的实测结果 | [性能与内存](docs/NESTRS_PERFORMANCE.md) |
| 追溯多轮缺陷修复及历史验收范围 | [修复记录](docs/NESTRS_FIXES.md) |
| 维护测试 | [core 测试](nestrs-core/tests/README.md)与[工具 fixture 索引](cargo-nestrs/tests/fixtures/README.md) |

## 本地基础检查

```sh
cargo test -p nestrs-core
cargo test -p cargo-nestrs
cargo fmt --all -- --check
```

这些普通 Cargo 检查不覆盖全部 rustc 集成。编译器契约、跨 crate、真实 DI 程序、
图导出和 LSP 各有独立入口，按[工具链指南](docs/NESTRS_CARGO_TOOLCHAIN.md)执行。
各项检查由开发者在本地按需执行；某个 host 的结果与完整编辑器交互验收分别说明，
不互相替代。
