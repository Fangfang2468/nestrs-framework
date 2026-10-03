# Cargo Nestrs：工具管理的声明、自动绑定与 IDE

应用只依赖 `nestrs-core` 和业务所需库。`cargo-nestrs` 管理服务声明、编译器适配、
自动绑定、完整 DI 图编译、rustdoc、编辑器项目模型与 HTML 依赖图。声明通过标准过程宏展开，内部
`nestrs-tool-bridge` 位于 `cargo-nestrs/internal/bridge`，设置 `publish = false`，
委托唯一的 `cargo-nestrs/src/codegen` 后端。应用不在 Cargo.toml 添加宏库，也不在
运行时链接 CLI。公开 `nestrs-macro`、独立 `nestrs-codegen` 和原生工具属性展开
都不是当前架构。

## 应用接入

应用 Cargo.toml 依赖 `nestrs-core` 和业务库，通过 `cargo nestrs` 编译。CLI 为
直接依赖 core 的编译单元注入名为 `nestrs` 的私有过程宏 extern；应用可以
`use nestrs::{injectable, constructor, factory, primary, lazy};`，也可以使用
`#[nestrs::injectable]` 等完整路径。`nestrs` 名称由工具保留，同名 Cargo 依赖会报冲突。

字段与参数 helper、constructor/factory、自动 trait 绑定和四个查询方法的完整用法
统一见[服务声明与查询](NESTRS_MACROS.md)。实例、scope 与关闭语义见
[core 架构](../nestrs-core/README.md)。本指南负责命令、启动配置、产物和验证入口。

## Cargo.toml 中的容器启动配置

在应用 package 的 `Cargo.toml` 顶层加入以下配置，然后使用 `ServiceProvider::build(None).await`：

```toml
[nestrs-cli]
initialization = "eager"
scope-initialization = "lazy"
max-concurrent-activations = 16
```

| 配置键 | 取值 | 缺省值 |
| --- | --- | --- |
| `initialization` | 未显式覆盖服务策略时的默认行为：`"lazy"` 按需构造；`"eager"` 在 build 返回前预热 Singleton 及必要依赖 | `"lazy"` |
| `scope-initialization` | 未显式覆盖服务策略时，`"lazy"` 按需构造；`"eager"` 在每次 create_scope 返回前初始化 Scoped 及必要依赖 | `"lazy"` |
| `max-concurrent-activations` | 大于零且能由目标平台 `usize` 表示的整数；全 root / scope 共享的构造任务上限 | `32` |

`[nestrs-cli]` 是 Nestrs 管理的自定义顶层配置节，不写成 `[package.nestrs-cli]` 或
`[package.metadata.nestrs.di]`。Cargo 本身会提示 `unused manifest key: nestrs-cli`；
这表示它不处理该节，不影响 Nestrs 的配置读取。工具对未知键、错误类型、拼错的模式、
零与负并发数报中文错误，包含 manifest 路径和配置键。

配置属于入口 package：同一个 package 的 binary / test 使用同一份设置，不自动继承
workspace 或依赖库的设置。在依赖库中调用 core 的 build，仍按最终入口配置启动。
多 package workspace 应在各个应用成员的 Cargo.toml 分别配置；虚拟 workspace 根中
的同名表不会作为成员默认值。

工具在编译时读取并固化设置，同时把 Cargo.toml 加入 rustc 的依赖跟踪。仅修改设置
也会使对应入口重新编译。运行产物不需要读取源码或 Cargo.toml，因此直接启动 binary
与 `cargo nestrs run` 使用相同的设置；配置修改后必须重新构建。

`ServiceProvider::build(Some(options))` 完整覆盖项目提供的默认配置；
`ServiceProvider::build(None)` 使用入口项目固化的配置。
`ServiceProviderOptions::default()` 仍为 root Lazy / scope Lazy / 32，不隐式读取项目配置；因此
`build(Some(Default::default()))` 表示显式选用库的基线，不能与 `build(None)` 混同。

root 和 scope 策略独立：只设置 `initialization = "eager"` 时，scope 默认仍为 Lazy。
`provider.create_scope(None).await?` 采用当前 root 中的 scope 默认值；
`create_scope(Some(ServiceScopeOptions { initialization: ... })).await?`
只覆盖这一次 scope 的默认策略，不改变随后 scope 的配置或共享构造上限。
`ServiceScopeOptions::default()` 为 Lazy；传入 `Some(Default::default())` 明确覆盖为
Lazy，传入 `None` 则采用容器保存的 scope 默认，两者并不等价。

这些设置只控制服务激活，不跳过全图校验，也不改变 scope 的隔离语义。
服务声明上的 `#[lazy]` / `#[lazy(true)]` 不作为自主预热根；`#[lazy(false)]` 则使
Singleton / Scoped 即便在所属 owner 为 Lazy 时也在创建前完成初始化。声明策略优先
于本次默认值，向 `build` 或 `create_scope` 传入 Some 也不会抹除它。普通注入依赖仍可
提前构造 lazy 目标。完整规则见
[宏使用指南](NESTRS_MACROS.md#34-用-lazy-控制某个服务是否自主预热)。
`create_scope` 与 build 都在选中服务初始化成功后才交付 owner；失败先等待关闭，
分别返回 `ScopeBuildError` 与 `BuildError::Initialization`。公开的 `warm_up` 已移除，
不再通过独立预热操作实现 scope 初始化。完整时序、失败与取消见
[core 创建和初始化说明](../nestrs-core/README.md#6-生命周期与预热)。
`cargo nestrs graph` 仍只分析图，即使配置为 Eager 也不执行 factory 或 cleanup。

## 工具链与命令

适配身份以 [toolchain.json](../cargo-nestrs/toolchain.json) 为准：

| 项目 | 当前值 |
| --- | --- |
| release | `1.98.0` |
| 完整 commit | `88d9e12ae178fab0fb5cc050a94da85685d449ea` |
| 本机 host | `x86_64-unknown-linux-gnu`、`x86_64-pc-windows-msvc` |
| 编译器组件 | 同一工具链的 `rustc-dev` |
| IDE 附加要求 | 匹配的 `rust-src` 与工具链附带的 rust-analyzer proc-macro server |

在已安装匹配工具链的环境中执行：

```sh
python3 tools/build-toolchain.py
export PATH="$PWD/target/debug:$PATH"
cargo nestrs doctor
cargo nestrs init
cargo nestrs check -p nestrs-di-example --all-targets
cargo nestrs run -p nestrs-di-example -- sample --eager --scope-initialization eager
cargo nestrs test -p nestrs-di-example
cargo nestrs graph -p nestrs-di-example
```

Windows 使用 PowerShell 和原生 Windows Rust，不依赖 WSL。先准备 Rustup、Python、
Visual Studio C++ Build Tools 与 Windows SDK，再安装并选择匹配的 MSVC 工具链：

```powershell
rustup toolchain install 1.98.0-x86_64-pc-windows-msvc --profile minimal --component rustc-dev --component rust-src
$env:RUSTUP_TOOLCHAIN = "1.98.0-x86_64-pc-windows-msvc"
python tools/build-toolchain.py
$env:PATH = "$PWD\target\debug;$env:PATH"
cargo nestrs doctor
cargo nestrs init
cargo nestrs check -p nestrs-di-example --all-targets
cargo nestrs run -p nestrs-di-example -- sample --eager --scope-initialization eager
cargo nestrs graph -p nestrs-di-example
```

安装固定版本后仍需在当前进程选择它；上述 `RUSTUP_TOOLCHAIN` 不改变全局
`rustup default`。只安装该版本、继续让脚本使用其他默认 rustc 会被身份检查拒绝。

构建脚本支持 `--release` 和 `--rustc PATH`，产生配套 CLI、driver 和 bridge：

| host | CLI | driver | bridge |
| --- | --- | --- | --- |
| Linux GNU x86-64 | `cargo-nestrs` | `nestrs-driver` | `libnestrs_tool_bridge.so` |
| Windows MSVC x86-64 | `cargo-nestrs.exe` | `nestrs-driver.exe` | `nestrs_tool_bridge.dll` |

bootstrap 只授权 `nestrs_driver` 和私有 `nestrs_tool_bridge` 的构建；bridge 使用
`proc_macro_def_site` 为生成绑定取得定义点 span，避免调用点常量与内部变量冲突。
普通 core/工具单元测试不需要该授权。不设置全局默认工具链，也不向应用子进程
传播该变量。CLI 不隐式安装组件。两个平台分别本机构建；Linux `.so` 和 driver
不能拷贝成 Windows 工具。Windows GNU、ARM64 和其他 host 不属于当前适配范围。
IDE 模型限本机 host；graph 已改为编译期 sidecar 导出，不再因目标产物不能在本机
执行而拒绝跨 target，但仍需要相应的目标编译环境，不能据此宣称所有 target 已验收。
固定 release/commit 的组件若不可获得，应明确失败，不能静默
安装另一个 Rust 版本后绕过身份检查。

CLI 默认查找同目录的 driver 与 bridge。`NESTRS_DRIVER`、`NESTRS_MACRO_BRIDGE` 和
`NESTRS_RUSTC` 可指定路径，完整编译器身份仍须匹配。桥接必须来自匹配工具链，
不能只拷贝 CLI 而遗漏它。普通 core/工具检查不需要开启 `compiler-driver` feature；
应用则必须获得 CLI 的注入环境，普通 Cargo 不提供此环境。仅用普通 Cargo 编译
core API 的应用，调用 build（无论传 None 或 Some）时返回 `BuildError::CompilerPlanUnavailable`，
不会把缺失工具链计划解释成合法空图。

维护程序可以读取 `cargo nestrs doctor --json` 的结构化报告，无需解析面向人的
文本输出。报告 `version` 为 1，包含 `rustc` 的 release / commit_hash / host，
以及 `compiler`、`sysroot`、`driver`、`macro_bridge` 和联合 `fingerprint`。
缓存目录由正式工具计算：

```sh
cargo nestrs doctor --json
cargo nestrs doctor --json --target-dir ./target/verification
```

指定 `--target-dir` 时，报告同时给出绝对的 `target_directory`、实际隔离后的
`cache_directory` 和 `compiler_output_directory`。该查询不构建项目、不创建
目标目录，也不要求当前目录有 Cargo.toml。相对路径按执行命令的当前目录解释；
工具版本或组件不匹配仍以非零状态失败。默认 `doctor` 保留面向人的文本输出。
未指定 `--target-dir` 时，三个目录字段为 null，不读取项目或环境中的 target 默认值。

### 源码构建与维护脚本的职责

`tools/build-toolchain.py` 是 CLI 尚未存在时的源码构建入口，负责检查已经安装的
固定编译器、限定 bootstrap 授权并调用 Cargo 构建 CLI / driver / bridge。它保留
首次构建必需的跨平台路径和动态库环境准备；正式应用构建、身份验证及缓存隔离
仍由 Rust 工具实现。已有工具的维护脚本通过 doctor 的结构化报告获取实际信息，
不复制缓存哈希算法。

正式编译回归的 Rust harness 和业务夹具集中于 `cargo-nestrs/tests/`；Python
验证器负责进程编排、记录证据或模拟外部客户端。原版 rust-analyzer LSP、浏览器
页面和性能测量各自保留独立验证入口。`tools/compiler-probe/` 保留 rustc 边界实验，
其中自动绑定脚本只转交正式 `autobind_contracts` 测试，正负例统一维护在
`cargo-nestrs/tests/fixtures/auto-binding/`。

源码构建入口不承担安装、升级、卸载或完整工具包分发；这些能力仍待单独设计和
验收。`cargo install` 目前也不会自动部署配套 bridge 与完整 sysroot。以上职责
整理没有新增应用依赖、runtime crate 或公开宏 package。

Cargo 仍管理依赖、features、cfg、profile、target 和 build.rs。CLI 转发 Cargo
选项和 `--` 后的业务参数，不支持与其他 Rust 编译包装器叠加。

## 按命令层级查看帮助

CLI 使用 clap 统一定义命令层级、Nestrs 自有选项及帮助内容。缺失选项值、
空的输出路径和不允许的选项组合会在加载工具链前报告，并附上当前命令的用法。
clap 参数诊断的退出码为 `2`；帮助和版本信息为 `0`，实际 Cargo 子进程的退出码
继续透传。

Nestrs 自有帮助的标题、命令说明和选项描述使用中文；命令名、参数名、环境变量
和路径等技术标识保持原样。

`cargo nestrs help` 展示命令总览。可以把 `help` 放在命令路径前后，或使用
`--help` / `-h`，查看对应层级的用法：

```bash
cargo nestrs help graph
cargo nestrs graph help
cargo nestrs graph --help

cargo nestrs help init check
cargo nestrs init help check
cargo nestrs init check help
cargo nestrs init check --help
```

前三条都展示 `graph` 帮助；后四条都展示 `init check` 的专属帮助，其中说明
JSON 诊断输出、成功后刷新项目模型，以及不能使用 `--vscode` 的限制。
`cargo nestrs init help` 则展示初始化选项与可用子命令。

帮助不会检查 Nestrs 工具链、编译项目或生成文件，在没有 `Cargo.toml` 的目录中
也可以查看。`check`、`build`、`run`、`test` 的帮助调用对应的 `cargo <命令> --help`，
因此需要可执行的 Cargo，并保留它完整的英文原始选项说明；其余帮助由 CLI 直接输出。

裸 `help` 在命令路径中识别，命令路径应写在选项之前。`--bin help`、
`--output help` 等选项值按原样使用；`--` 后的 `help`、`--help` 和 `-h` 属于
应用或测试参数。例如 `cargo nestrs run -- --help` 仍运行应用并请求应用自己的帮助。
不存在的命令路径会报错并指向相应父级帮助，不会退回命令总览。
`create` 尚未实现，查询 `cargo nestrs create help` 会明确提示这一点，
并指向现有项目的 `init` 用法。

Nestrs 的选项可以放在 Cargo 选项之后，例如：

```bash
cargo nestrs graph -p nestrs-di-example --output target/checkout-di.html
cargo nestrs init --all-targets --output target/editor/rust-project.json --vscode
```

`--output PATH` 和 `--output=PATH` 均支持；重复指定时采用最后一个值。
其余 Cargo 参数保持原有顺序和系统字符串编码，CLI 不会重建一套 Cargo 选项表来
限制可用参数。`graph` 的目标选择规则和 `init check` 禁止 `--vscode` 的规则继续适用。

## 编译、图验证和资源初始化

| 入口 | 工作与边界 |
| --- | --- |
| `cargo nestrs check/build` | 注入声明宏，生成真实类型绑定，完成 Rust 类型与借用检查；最终 binary / test 同时编译并验证完整 DI 图 |
| `cargo nestrs init` | 初始化或刷新现有项目的 Nestrs 开发环境，成功检查后生成 rust-analyzer 项目与设置 |
| `cargo nestrs test` | 执行测试；文档示例经完整 driver 编译后由真实 rustdoc 运行 |
| `ServiceProvider::build(None)` | 加载入口共享的不可变编译计划，创建独立运行时；按服务声明策略与 root 默认选取 Singleton 初始化入口 |
| `provider.create_scope(None)` | 异步创建独立 scope；按服务声明策略与 scope 默认选取 Scoped 初始化入口，成功后交付 |
| `cargo nestrs graph` | 对选定 binary 执行 Cargo check，从编译器同源 sidecar 生成 HTML；`--bin` 限定单入口 |
| 实际激活 | 执行 constructor/factory；外部资源初始化仍可能失败 |

最终 binary / test 的 `check/build` 已检查缺失依赖、重复、歧义、环和生命周期冲突；
library 编译只贡献声明和查询摘要，由最终入口合并应用需求。结构合法仍不保证数据库
等外部资源初始化成功。graph 使用同一份编译器语义计划，不运行目标程序、不替换
业务 `main`，也不调用服务构造。编译器与执行协议见[rustc 集成指南](NESTRS_RUSTC_EXTENSION_GUIDE.md#plan)，运行期行为见[core README](../nestrs-core/README.md)。

## IDE 与 rustdoc

`cargo nestrs init` 检查现有项目，生成 `<Cargo target>/nestrs/ide/rust-project.json`
及相邻设置；只有 `--vscode` 才合并 `.vscode/settings.json`。它不创建项目、不改业务
依赖、不安装编辑器。保存检查调用 `cargo nestrs init check`，输出真实 Cargo JSON
诊断；成功刷新模型，失败保留上次有效模型，无变化不重写。配置合并、constructor
语义模型、未保存编辑与平台限制统一见 [IDE 接入](NESTRS_IDE.md)。

`cargo nestrs test` / `test --doc` 使用真实 rustdoc。文档源码和每个独立示例均经
完整 driver，因此示例内可声明服务、使用自动绑定及闭合泛型。文档载体不替代实际
业务库，不能通过省略示例隐藏失败。相对 include 与来源边界见
[rustdoc 集成](NESTRS_RUSTC_EXTENSION_GUIDE.md#editors)。

当前命令不包含 `doc`、`clippy` 或 `create`。core/工具自身可以使用普通 Cargo
Clippy；这不等于应用级 Clippy 已接入 Nestrs。旧 `ide` 命令已改为 `init`，迁移时
携带原选项重新执行；VS Code 项目还应加 `--vscode` 更新保存检查命令。

## 生成产物与缓存

工具在源码所属 crate 生成合法的 typed adapter，并在最终 binary/test 汇总声明、
查询和闭合泛型，冻结完整计划。项目生成内容统称 `nestrs-reflect`，不是需要新增的
Cargo package。私有执行入口为 `__nestrs_reflect_v2`；工具和 core 要配套重编译，
引用 core 的空图也在 check 阶段验证 `plan_set_options_v3`，不接受旧协议。

| 产物 | 用途 |
| --- | --- |
| `*.nestrs-reflect.json` | 最终入口 metadata 旁的执行清单；JSON schema 仍为 `1`，记录已选节点、输入、投影、路由与顺序 |
| `*.nestrs-plan.json` | graph 命令读取的同源展示数据；schema 与清单的编号约定不同 |
| `<隔离构建目录>/nestrs/compiler/<编译单元>/` | 两轮分析记录与自动绑定的虚拟源码片段 |
| `<Cargo target>/nestrs/ide/rust-project.json` | 编辑器项目模型；默认位置相对原 Cargo target |

前两份 JSON 供审阅或展示，运行程序不读取它们；业务源码不会被工具生成的绑定覆盖。
编译器机制、跨 crate 可见性与清单字段统一见
[rustc 集成指南](NESTRS_RUSTC_EXTENSION_GUIDE.md#plan)。

CLI 按 release、完整 commit、host 和 **driver/bridge 的联合内容指纹**隔离 target。
普通 Linux 构建位于 `<Cargo target>/nestrs/<compiler identity>/<fingerprint>`；
Windows 使用完整身份的短哈希。graph 再按 package/binary 隔离，Windows graph
使用 `<Cargo target>/nestrs/g<16位哈希>`。IDE 另按构建选项与 workspace 身份隔离。
短路径降低 MSVC 路径长度压力，但不保证任意深目录都可用。

Cargo 继续复用未变更构建单元；rustc incremental 当前关闭。两轮源码快照只验证
已读取源码，不涵盖任意非确定过程宏、环境变量或全部外部文件，不是完整构建事务。
首次 core 装配仍调用目标端 adapter 取得真实 `TypeId` 与函数地址；后续 root 共享
不可变计划，但缓存与服务实例各自独立，不存在运行时重新注册或扩图。

## 编译错误的展示

DI 错误通过原生 rustc 诊断与 Cargo JSON 输出，主位置指向业务字段、
constructor/factory 参数、查询或冲突声明，附相关位置与修复提示；内部证据保留在
最后的 `cause`。`NESTRS-DI001` 等标识位于消息正文，不占用 Rust 的 `E0xxx` 编号。
错误分类、多个错误的排序、来源恢复与完整输出统一见[诊断指南](NESTRS_DIAGNOSTICS_DESIGN.md)。

可用独立的预期失败项目观察终端与 JSON 输出：

```sh
cargo nestrs check --manifest-path example/di-errors/01-missing-concrete/Cargo.toml
cargo nestrs check --manifest-path example/di-errors/01-missing-concrete/Cargo.toml --message-format=json
```

这两条命令都应因缺失依赖编译失败。更多源码、修复方向与批量核对入口见
[依赖错误示例](../example/di-errors/README.md)。

## HTML 图边界

HTML/CSS/JavaScript 与写文件全部由 CLI 负责，输入来自编译器的
`*.nestrs-plan.json` 图 sidecar，与最终执行计划共用候选选择、输入和拓扑结果。
该图文件与审阅用的 `*.nestrs-reflect.json` 用途不同；两个文件都不是容器启动时
加载的配置，也不表示运行时实例或请求的快照。
默认输出 `<Cargo target>/nestrs-di.html`；`--output PATH` 相对调用目录解析。

```sh
# package 总览；不受 default-run 限制
cargo nestrs graph -p nestrs-di-example

# workspace 总览；library-only package 只出现在项目清单中
cargo nestrs graph --workspace

# 限定一个 binary
cargo nestrs graph -p nestrs-di-example --bin checkout
```

省略 `--bin` 时，每个 binary 在独立编译计划和缓存中处理，HTML 包含项目总览、
入口筛选、共享声明归属以及各入口诊断。每个节点和依赖边始终属于实际入口，不能
把不同 binary 的 provider 合并后作统一校验，也不能理解为跨进程共享 Singleton。
指定 `--bin` 时保持原单图页面与校验契约。
不传 `-p` 或 `--workspace` 时选择 Cargo 元数据中的默认 workspace members；命令一次
接受一个 package 选择器，`--workspace` 与 `-p` 不能同时使用。

项目报告的入口有三种状态：成功、错误和跳过。未满足 `required-features` 的入口
标为跳过；`--features`、特性转发和 `--all-features` 按 Cargo 实际解析的配置生效。
这里的特性解析以每个 package 的独立构建为准。当前拒绝 `--workspace --features ...`，
未写 `-p` 或 `--workspace` 但默认 workspace members 选中多个 package 时也拒绝
`--features`；任意 workspace 特性表达式无法直接转发给逐 package 构建。请改用
`-p PACKAGE --features ...`。`--workspace --all-features` 和 `--no-default-features`
可按各 package 独立应用，不能把这一模式等同于 Cargo 一次 workspace 构建的特性合并。
跳过不算图错误，也不代表验证了该入口；只有跳过而没有错误时退出状态为 0。
每个入口的编译错误、图校验错误或当前
不支持的入口形式保存在报告中，并继续检查其他入口。存在任意错误时命令返回非零，
但仍写出报告；全部入口失败也可以查看诊断。

[电商示例](../example/di-checkout/README.md)位于 `example/di-checkout/`，package 名称
保持 `nestrs-di-example`，现在只包含正常的 `checkout` 入口。上面的 package 图命令
在图校验和文件写入成功时返回退出码 `0`。从仓库根目录执行时使用上述 `-p` 参数；
进入 `example/di-checkout/` 后可以直接执行 `cargo nestrs graph`，默认输出仍为仓库
根的 `target/nestrs-di.html`。故意非法的生命周期用例位于 aot-invalid 测试 fixture；
graph fixture 中独立的 `invalid_graph` 用于验证部分失败报告，不参与正常电商图导出。

显式 `--bin` 的图验证失败、项目没有 binary、入口选择无效、元数据查询失败以及
输出写入失败不会覆盖已有 HTML。一次项目导出只描述同一组 features、cfg、profile 和目标平台下的结果，
不会同时枚举未启用特性或无限开放泛型。只含 lib 的项目目前没有独立 DI 图入口。
当前 graph 入口仍要求所选 binary 直接依赖 `nestrs-core`，driver 对仅间接依赖 core
的入口会明确拒绝。这是图命令现有的额外限制；正常 `check/build/run` 可以通过业务库
封装间接使用容器。声明服务的 crate 应显式依赖 core，确保首次宏展开和编辑器模型都能
获得 `nestrs`；传递 core 的自动接入发生在首次展开后的语义探测。

命令保留真实 binary 的声明和查询根，只进行编译检查和 sidecar 导出；不执行 main、
constructor、factory、Default、value 或 cleanup，也不创建 Tokio runtime。图导出
不再要求定位或替换 `main`，目标程序是否能在当前 host 执行也不再是导出条件。
sidecar 跟随本次 Cargo 返回的 `.rmeta` 工件，并核对 package、binary、源码和
metadata 身份，避免把其他 feature/profile 构建留下的图作为当前结果。
编译过程仍可能运行 Cargo build script 和过程宏，因此上述保证针对应用 main 与
DI 激活，不能解释为整个编译过程不会执行任何代码。
`--target` 由 Cargo 和配套工具链处理，所需目标库仍须可用；这不表示所有 target 已有
实机验收。目前图入口选择仍限 binary，lib/test/example 独立图目标尚未开放。

## 回归入口与核对位置

普通工具单测不需要 rustc_private；应用与编译器集成须先构建完整工具。以下命令
是可重放的验证入口，实际执行结果按快照和 host 记录在[修复记录](NESTRS_FIXES.md)。
这些命令的存在不表示当前版本已在所有 host 执行。完整测试职责见
[core 测试说明](../nestrs-core/tests/README.md)。

```sh
python3 tools/build-toolchain.py
cargo test -p nestrs-core -p cargo-nestrs
cargo clippy -p nestrs-core -p cargo-nestrs --all-targets -- -D warnings
cargo fmt --all -- --check
cargo nestrs test --workspace --exclude nestrs-tool-bridge
cargo nestrs test --manifest-path cargo-nestrs/tests/fixtures/di/Cargo.toml --all-targets
```

Linux 下，`rustc` 与 Cargo 选择同一固定工具链后，编译器集成测试可执行：

```sh
LD_LIBRARY_PATH="$(rustc --print sysroot)/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
RUSTC_BOOTSTRAP=nestrs_driver,nestrs_tool_bridge \
cargo test -p cargo-nestrs -p nestrs-tool-bridge --features cargo-nestrs/compiler-driver --no-fail-fast
python3 tools/verify-macro-toolchain.py --skip-build
python3 tools/verify-cross-crate-binding.py --skip-build
python3 tools/verify-graph.py --skip-build
python3 tools/verify-ide.py --skip-build --rust-analyzer /path/to/rust-analyzer
python3 tools/compiler-probe/verify_autobind.py --skip-build
```

Windows 使用 PowerShell 和相同脚本名，把 `python3` 替换为本机 `python`，并使用
本机 rust-analyzer.exe。直接运行 compiler-driver 测试时须让 PATH 包含匹配 sysroot
的 bin，并仅对该测试进程设置 `RUSTC_BOOTSTRAP=nestrs_driver,nestrs_tool_bridge`；不要把它传播给
业务编译。上述测试和脚本由开发者在对应 host 手动执行，无需配置流水线。
私有 bridge 与 driver 一起接受上述工具构建测试；应用路径的 workspace 测试排除
bridge，因为 CLI 会清除 bootstrap，且 bridge 不是应用依赖。
workspace 的 default-members 仅含 core 和工具，因此不指定 package 的普通 Cargo
检查和单测也不会要求私有 bridge 的构建授权。
支持 host 的声明、历史本机结果和当前源码的验证结果是不同证据；某个平台通过
不代表每次修改都已在两端复测。

| 行为 | 实现或真实回归 |
| --- | --- |
| 命令解析与帮助 | [commands/cli.rs](../cargo-nestrs/src/commands/cli.rs)、[cli.rs 测试](../cargo-nestrs/tests/cli.rs) |
| pin、工具匹配与缓存 | [toolchain.rs](../cargo-nestrs/src/toolchain.rs)、[build-toolchain.py](../tools/build-toolchain.py) |
| 自动绑定正例与编译期负例 | [autobind_contracts.rs](../cargo-nestrs/tests/autobind_contracts.rs)、[auto-binding fixture](../cargo-nestrs/tests/fixtures/auto-binding/Cargo.toml) |
| 启动配置 | [project_config.rs](../cargo-nestrs/src/project_config.rs)、[startup_config 回归](../cargo-nestrs/tests/startup_config.rs) |
| graph 选择与输出保留 | [commands/graph.rs](../cargo-nestrs/src/commands/graph.rs)、[verify-graph.py](../tools/verify-graph.py) |
| 本机路径、metadata 与 doctest | [native_host.rs](../cargo-nestrs/tests/native_host.rs)、[bridge_metadata.rs](../cargo-nestrs/tests/bridge_metadata.rs)、[rustdoc.rs](../cargo-nestrs/tests/rustdoc.rs) |

离线 HTML 的交互回归使用已有 Playwright 和 Chromium，不为仓库增加 npm 依赖：

```sh
cargo nestrs graph -p nestrs-di-example --bin checkout --output target/checkout-di.html
node tools/verify-graph-page.cjs --html target/checkout-di.html --output target/nestrs-graph-page
```

可用 `--playwright PATH` / `NESTRS_PLAYWRIGHT_MODULE` 指定已有模块，用
`--chromium PATH` / `NESTRS_CHROMIUM_PATH` 指定浏览器。只改页面时，可加
`--template cargo-nestrs/src/graph.html` 复用实际图数据；脚本不安装或下载工具。
它检查离线请求、JavaScript 错误、入口/搜索/筛选、槽位与 key、连线布局和移动端，
在输出目录保存 `report.json`、HTML 与截图。
