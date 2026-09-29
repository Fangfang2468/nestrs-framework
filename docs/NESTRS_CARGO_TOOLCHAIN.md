# Cargo Nestrs：工具管理的声明、自动绑定与 IDE

应用只依赖 `nestrs-core` 和业务所需库。`cargo-nestrs` 管理服务声明、编译器适配、
自动绑定、rustdoc、编辑器项目模型与 HTML 依赖图。声明通过标准过程宏展开，内部
`nestrs-tool-bridge` 位于 `cargo-nestrs/internal/bridge`，设置 `publish = false`，
委托唯一的 `cargo-nestrs/src/codegen` 后端。应用不在 Cargo.toml 添加宏库，也不在
运行时链接 CLI。公开 `nestrs-macro`、独立 `nestrs-codegen` 和原生工具属性展开
都不是当前架构。

## 应用声明

面向应用开发的完整示例、宏参数和常见错误见[宏使用指南](NESTRS_MACROS.md)。

CLI 为直接依赖 core 的编译单元注入名为 `nestrs` 的过程宏 extern。业务代码可以
保持短属性，也可以使用 `#[nestrs::injectable]` 等完整路径：

```rust
use nestrs::{factory, injectable, primary};

#[injectable]
struct Database;

trait Store: Send + Sync {
    fn available(&self) -> bool;
}

#[injectable]
#[primary]
struct SqlStore {
    #[inject]
    database: Database,
}

impl Store for SqlStore {
    fn available(&self) -> bool { true }
}

#[injectable(lifetime = Scoped)]
struct Checkout {
    #[inject]
    store: dyn Store,
}

struct Settings;

#[factory]
fn settings() -> Settings { Settings }
```

`#[inject]` / `#[value(...)]` 及带 `nestrs::` 前缀的 helper 由所在声明消费，不是
可单独导入的宏。注入字段转换为持有强 lease 的 `Injection<T>`，factory 输入转换
为借用实际 activation frame 的参数，随后由 Rust 检查类型和借用。

内部 bridge 只转换标准 `proc_macro::TokenStream` 并调用共享 `expand_*` 后端。
没有第二份声明分析实现，也没有 `register_tool` 或复制的 rustc 展开管线。
单个声明宏支持重命名导入；组合 primary 时保留属性末段 `injectable`、`factory`、
`primary`。crate/module 路径别名有回归覆盖，任意重命名属性之间的协调不在保证范围。

普通 `impl Trait for Concrete` 按实际接口需求参与自动绑定，业务代码不写 bind。
`nestrs::bind` 仅保留为隐藏的显式绑定 ABI 回归入口。查询仍使用 core 的四个宏，
owner、scope、借用与关闭 API 不变。`nestrs` extern 名被工具保留，同名 Cargo
依赖会报冲突；不要添加一个宏依赖来覆盖它。

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
cargo nestrs run -p nestrs-di-example -- --eager --warm-up-scopes
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
cargo nestrs run -p nestrs-di-example -- --eager --warm-up-scopes
cargo nestrs graph -p nestrs-di-example
```

安装固定版本后仍需在当前进程选择它；上述 `RUSTUP_TOOLCHAIN` 不改变全局
`rustup default`。只安装该版本、继续让脚本使用其他默认 rustc 会被身份检查拒绝。

构建脚本支持 `--release` 和 `--rustc PATH`，产生配套 CLI、driver 和 bridge：

| host | CLI | driver | bridge |
| --- | --- | --- | --- |
| Linux GNU x86-64 | `cargo-nestrs` | `nestrs-driver` | `libnestrs_tool_bridge.so` |
| Windows MSVC x86-64 | `cargo-nestrs.exe` | `nestrs-driver.exe` | `nestrs_tool_bridge.dll` |

bootstrap 只授权 `nestrs_driver` 的构建，不设置全局默认工具链，也不向应用子进程
传播该变量。CLI 不隐式安装组件。两个平台分别本机构建；Linux `.so` 和 driver
不能拷贝成 Windows 工具。Windows GNU、ARM64、其他 host 与跨 target 的 graph/IDE
不属于当前适配范围。固定 release/commit 的组件若不可获得，应明确失败，不能静默
安装另一个 Rust 版本后绕过身份检查。

CLI 默认查找同目录的 driver 与 bridge。`NESTRS_DRIVER`、`NESTRS_MACRO_BRIDGE` 和
`NESTRS_RUSTC` 可指定路径，完整编译器身份仍须匹配。桥接必须来自匹配工具链，
不能只拷贝 CLI 而遗漏它。普通 core/工具检查不需要开启 `compiler-driver` feature；
应用则必须获得 CLI 的注入环境，普通 Cargo 不提供此环境。

Cargo 仍管理依赖、features、cfg、profile、target 和 build.rs。CLI 转发 Cargo
选项和 `--` 后的业务参数，不支持与其他 Rust 编译包装器叠加。

## 按命令层级查看帮助

CLI 使用 clap 统一定义命令层级、Nestrs 自有选项及帮助内容。缺失选项值、
空的输出路径和不允许的选项组合会在加载工具链前报告，并附上当前命令的用法。
clap 参数诊断的退出码为 `2`；帮助和版本信息为 `0`，实际 Cargo 子进程的退出码
继续透传。

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
因此需要可执行的 Cargo，并保留它的完整选项说明；其余帮助由 CLI 直接输出。

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
| `cargo nestrs check/build` | 注入标准声明宏，按真实语义生成绑定，完成 Rust 类型与借用检查 |
| `cargo nestrs init` | 初始化或刷新现有项目的 Nestrs 开发环境，成功检查后生成 rust-analyzer 项目与设置 |
| `cargo nestrs test` | 执行测试；文档阶段注入同一 bridge 并转发真实 rustdoc |
| `ServiceProvider::build()` | 构造任何服务前验证当前链接单元的完整注册图，成功后冻结 |
| `cargo nestrs graph` | 独立验证选定项目的各 binary，导出项目 HTML；`--bin` 限定单入口 |
| 实际激活 | 执行 constructor/factory；外部资源初始化仍可能失败 |

`check/build` 成功不等于执行了容器全图校验，结构合法也不保证数据库连接等外部
资源成功。图诊断程序不执行服务构造，但它仍然运行一个目标程序，不能称为纯 Rust
类型检查。具体实现见[编译器适配说明](NESTRS_COMPILER_ADAPTER.md)。

## IDE 与 rustdoc

`cargo nestrs init` 面向自行组装后接入 Nestrs 的现有 Rust 项目，初始化开发环境；
也可重跑以刷新已有环境。默认生成 `<Cargo target>/nestrs/ide/rust-project.json`
与相邻的 `rust-analyzer-settings.json`，不会创建项目、添加 Cargo 依赖或改写业务源码。
CLI 不安装编辑器或工具链组件。

只有 `cargo nestrs init --vscode` 才将 linkedProjects、检查命令和过程宏服务器配置
合并到 `.vscode/settings.json`。JSONC 注释、无关设置和已有诊断偏好保留，首次变更前
备份一次；无效或重复键输入明确拒绝且不覆盖。其他使用 rust-analyzer LSP 的客户端
需按自己的接入方式加载生成项目与配置，不能把“编辑器支持 Rust”理解为它必然使用
rust-analyzer，也不承诺初始化后所有客户端自动生效。

未来的 `cargo nestrs create` 将在 bootstrap 完成后创建新项目，并在内部复用初始化
能力，直接交付已初始化的项目，用户无需再执行 `init`。`create` 当前尚未实现。

模型来自实际 rustc 单元和 Cargo artifact，保留依赖身份、重命名、多版本、cfg、
edition、test、build.rs 环境、OUT_DIR 和其他过程宏工件。core 用户在编辑器模型中
获得同名 `nestrs` 私有依赖，原版宏服务器使用真实 bridge，编辑器据此推导注入
字段和工厂参数。保存时 `cargo nestrs init check` 流式输出真实编译诊断，成功后
刷新模型；失败保留上次成功模型，无变化不重写文件。未保存源码由 LSP 自身分析。

cfg 由同一 rustc 按实际参数执行 `--print cfg` 获取。配置 `cfg.setTest = false`
及 `cargo.cfgs = []`，避免编辑器合成 test、debug_assertions 或 miri 条件；真实
test 与 profile 条件已在各编译单元中记录。保存检查的 `check.extraEnv` 固定
本次选定的 rustc、driver 和 bridge 路径，保留其他用户环境变量。

原版 rust-analyzer 仍会合并 host 默认 cfg；若实际参数移除其中某项，例如
`panic = "abort"` 或禁用默认 CPU 特性，IDE 准备会明确拒绝并保留旧模型。
debug/release 和增加 CPU 特性可以表示；此限制不影响应用的 check/build/run。

真实原版 rust-analyzer 的冷启动、hover、补全、定义跳转、未保存编辑、真实错误与
恢复，以及 feature、宏生成声明和 build.rs 产物已由 LSP 回归验证。完整使用方式与
当前平台边界见 [IDE 接入](NESTRS_IDE.md)。这不代表所有编辑器 UI 或任意宏组合均已验收。

CLI 的 `RUSTDOC` 指向 driver 的转发入口。它注入同一 bridge 及元数据搜索目录，
再调用固定 sysroot 中未经修改的 rustdoc；不使用拒绝 shim，不静默忽略 doctest。
文档内声明、concrete 查询、闭合泛型和 factory 跨 await 借用有实际运行覆盖。
独立 doctest 内新声明的 trait 绑定仍不会自动经过两阶段 driver，完整接口图应在
CLI 集成测试中验证。应用级 Clippy 接入和 `cargo nestrs clippy` 尚未实现；普通
core/构建工具的 Clippy 检查可以独立运行。

## 自动绑定、元数据与缓存

第一轮分析收集 marker 中真实的 Ty/DefId、provider、接口需求和有限闭合类型，
用 trait solver / Unsize 检查投影。FileLoader 将辅助 Rust 代码放入合法模块的
虚拟编译输入，第二轮重新展开和检查；磁盘上的业务源码不被改写。

跨 crate 分析读取上游注册回调、查询根和闭合蓝图的编码 MIR；
`always_encode_mir` 使 metadata-only check 产物也包含必要描述。类型匹配使用
rustc 的真实身份，公开重导出和 Cargo 依赖别名使用实际可访问的生成路径。

服务所属 crate 为已声明的 provider、factory 成功类型及可确定的闭合泛型生成
**自动投影能力目录**。投影代码仍在能合法访问实现类型的模块中编译，所以私有
实现也能通过公开接口提供给下游。目录不公开业务实现类型，也不实例化服务。
core 在 build 时合并当前链接单元的需求，仅启用查询根或 provider 依赖实际需要
的接口；未请求的能力不会产生路由、触发歧义或物化泛型。启用后仍在实例化前完成
整个依赖闭包的验证，成功后冻结图。

来自兄弟 crate 的同一自动投影按 concrete/interface 类型对幂等合并。已有显式
投影优先，两个显式投影重复仍报图错误；不同 concrete 实现不会因为接口相同而
被合并。精确 key、primary、optional、生命周期和实例缓存规则保持不变。

业务项目的拆分方式与支持边界见 [跨 crate DI](NESTRS_CROSS_CRATE_DI.md)。

producer metadata 会记录 bridge 的实际 crate 身份。工具不仅注入 `--extern nestrs`，
还为下游 rustc/rustdoc 添加 bridge 目录的 `-L dependency=...`；即使 consumer 没有
直接 core 依赖，也能加载上游元数据。这不意味着向无 core 的 consumer 开放服务声明。

CLI 按完整编译器身份与 **driver、bridge 的联合内容指纹** 隔离 target，graph 再
按 package/binary 隔离。Cargo 仍复用未变更构建单元，rustc incremental 当前关闭。
Windows 的目录名称使用上述完整身份的短哈希，缩短 MSVC 链接器看到的输出路径；
graph 使用 `<Cargo target>/nestrs/g<16位哈希>`，将完整编译器身份、driver/bridge
联合内容指纹及 package/binary 身份一起计算到哈希中，避免继续嵌套长目录。
缓存失效规则保持一致。这能减少 `MAX_PATH` 问题，不保证任意深的工作区路径都可用。
源码快照不覆盖任意非确定过程宏或全部 build.rs 外部输入，不能描述成完整事务。

## HTML 图边界

HTML/CSS/JavaScript 与写文件全部由 CLI 负责，core 仅提供隐藏只读图 JSON。
默认输出 `<Cargo target>/nestrs-di.html`；`--output PATH` 相对调用目录解析。

```sh
# package 总览；不受 default-run 限制
cargo nestrs graph -p nestrs-di-example

# workspace 总览；library-only package 只出现在项目清单中
cargo nestrs graph --workspace

# 限定一个 binary
cargo nestrs graph -p nestrs-di-example --bin checkout
```

省略 `--bin` 时，每个 binary 在独立诊断入口和缓存中处理，HTML 包含项目总览、
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
根的 `target/nestrs-di.html`。故意非法的生命周期用例位于 DI 测试 fixture，graph fixture 中
独立的 `invalid_graph` 仍用于验证部分失败报告，不参与正常电商图导出。

显式 `--bin` 的图验证失败、项目没有 binary、入口选择无效、元数据查询失败以及输出写入失败不会覆盖
已有 HTML。一次项目导出只描述同一组 features、cfg、profile 和 host 下的结果，
不会同时枚举未启用特性或无限开放泛型。只含 lib 的项目目前没有独立 DI 图入口。

命令保留真实 binary 的声明和查询根，生成诊断入口；不执行 main、constructor、
factory、Default、value 或 cleanup，也不创建 Tokio runtime。当前仅支持固定
host 可运行、直接依赖 core 且源码中具有标准 main 的 binary，包括
`#[tokio::main]`。无直接 core、`no_main` 及 `cfg_attr` 引入的 `no_main` 在执行前
拒绝并进入相应入口诊断；宏生成 main、lib/test/example 图目标和跨 target 执行仍未支持。

## 回归入口

```sh
cargo check -p nestrs-core -p cargo-nestrs -p nestrs-tool-bridge --all-targets
cargo test -p nestrs-core -p cargo-nestrs -p nestrs-tool-bridge
cargo clippy -p nestrs-core -p cargo-nestrs -p nestrs-tool-bridge --all-targets -- -D warnings
cargo fmt --check

cargo nestrs check --workspace --all-targets
cargo nestrs test --workspace
cargo nestrs test --manifest-path cargo-nestrs/tests/fixtures/di/Cargo.toml --all-targets
python3 tools/verify-macro-toolchain.py --skip-build
python3 tools/verify-cross-crate-binding.py --skip-build
python3 tools/verify-ide.py --skip-build
python3 tools/verify-graph.py --skip-build
python3 tools/compiler-probe/verify_autobind.py
```

DI fixture 保留原 52 个 UI 基线，加 3 个宏/helper 误用和 1 个导入成功用例；另增加
字段与工厂参数拒绝 `#[inject(key = ...)]` 的两个用例，当前共 58 个。它们经过工具
提供的 bridge，检查完整错误代码、消息和重复次数。跨 crate verifier
覆盖 metadata-only check 及 6 次 debug/release 运行，自动绑定探针保留 14 次运行。
另有跨 crate DI verifier 覆盖独立接口库、私有 class/factory、兄弟 crate 需求、
精确 key、primary、关联类型、闭合泛型及真实 HTML 图，并验证歧义在构造前失败。
图 verifier 检查副作用哨兵、目标缓存、普通运行正对照、单图失败保留输出，以及项目
部分/全部失败、跨 package 同名入口、feature 跳过与开启和 library-only 项目。
IDE verifier 记录真实 LSP 消息与断言，不能以关闭诊断代替成功。

离线 HTML 交互另有真实 Chromium 回归，输入为实际电商示例导出的单图或项目报告：

```sh
cargo nestrs graph -p nestrs-di-example --bin checkout --output target/checkout-di.html
node tools/verify-graph-page.cjs --html target/checkout-di.html --output target/nestrs-graph-page
```

脚本使用已有 Playwright 安装，不为仓库增加 npm 依赖。若 `require('playwright')`
不可用，可传 `--playwright /path/to/playwright`，或设置 `NESTRS_PLAYWRIGHT_MODULE`；
浏览器可通过 `--chromium /path/to/chromium` 或 `NESTRS_CHROMIUM_PATH` 指定。
默认使用该 Playwright 安装管理的 Chromium。只修改页面时，可以传
`--template cargo-nestrs/src/graph.html`，复用已导出的真实图数据测试当前模板，
无需重新执行 Cargo。所有路径均可配置，脚本不安装工具或下载浏览器。

这组检查覆盖实际输入页面在 1600/1280px 下的单图或项目入口筛选，以及真实 Checkout
的接口请求、精确 key 与选中实现链路（包含超过 JavaScript 安全整数范围的 indexed
十进制字符串和同文本 named key）、输入槽位与重复 Transient、
多入口局部 ID 隔离、搜索/筛选/详情、安全文本、失败及跳过报告和手机布局；还检查
大图被截断的接口/缺席输入可经搜索访问，完整槽位可分页查看。真实 Checkout 的每条
连线都经几何采样，确保不穿过无关节点，连线标签也不能覆盖节点或彼此。
浏览器处于离线模式，JavaScript 错误及网络请求都会使验证失败。结果、合成边界案例
HTML 和截图保存在指定输出目录，`report.json` 记录各场景的通过状态。

2026-09-28 已在原生 Windows `x86_64-pc-windows-msvc`、固定 Rust `1.98.0` 上
完成实际验证：workspace check/test、包含 Eager 与 scope warm-up 的电商示例、
DI fixture 与全部 56 个 UI 契约用例、三个 driver 集成测试，以及真实 rustdoc。
图 verifier 的 12 条命令和跨 crate verifier 的 6 次 debug/release 执行通过。
原版 rust-analyzer `0.3.3049` 的完整 LSP 验证也通过 default、alternate、release
三种配置，详情见 [IDE 验证与边界](NESTRS_IDE.md#验证与边界)。这些是 Windows
本机 exe/dll 的执行结果，Linux 回归另行运行；不是把 WSL 运行结果当作 Windows 结果。
两个平台的 core/工具/driver 严格 Clippy 及格式检查也已通过。

Windows 开发者可以在构建工具后，用 PowerShell 重放主要验收入口：

```powershell
cargo nestrs check --workspace --all-targets
cargo nestrs test --workspace
cargo nestrs test --manifest-path cargo-nestrs/tests/fixtures/di/Cargo.toml --tests
python tools/verify-graph.py --skip-build
python tools/verify-macro-toolchain.py --skip-build
python tools/verify-ide.py --skip-build --rust-analyzer C:\tools\rust-analyzer.exe
```

最后一条命令的 server 路径应替换为已安装的原版 rust-analyzer 路径；验证器不会
为用户安装编辑器。普通使用无需运行这些开发回归脚本。

`cargo-nestrs/tests/native_host.rs` 是 Linux/Windows 共用的真实工具回归，需要先
构建完整工具链。它在包含空格和中文的临时目录中运行 check/build/run、graph 输出
替换与失败保留、IDE 模型生成，以及编辑器配置中的保存检查；图 fixture 检查业务
main 和服务构造副作用均未执行。`bridge_metadata` 和 `rustdoc` 测试同样在两个
host 上启用，覆盖没有直接 core 依赖的下游和实际可执行 doctest。Unix shell mock
测试仍仅用于 Unix，不能替代 Windows 原生回归。完整 LSP 验证与原生 CLI 测试是
不同层级，某个平台通过项目模型测试不代表其全部编辑器交互已经验收。

[Native toolchain hosts CI](../.github/workflows/native-host.yml) 为 Linux 和 Windows
分别定义上述回归：从仓库 pin 选择编译器，安装 `rustc-dev`、`rust-src`、rustfmt 和
Clippy，再由构建脚本检查完整 commit；不可获得的固定版本会使任务失败，不回退到
stable。该任务覆盖 core/工具测试、格式、严格 Clippy 和三个真实 driver 集成测试，
不安装或启动额外 rust-analyzer LSP server。工作流文件的存在不等于云端任务已成功
执行，应以对应提交的 Actions 日志为准。

三个历史阶段的包划分与命令只作演进记录，当前用户契约以本文、[IDE 接入](NESTRS_IDE.md)
及 [README](../README.md) 为准。
