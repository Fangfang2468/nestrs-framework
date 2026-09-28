# cargo nestrs init：现有项目初始化与 rust-analyzer 接入

应用不依赖公开宏库。`cargo nestrs` 管理一个私有过程宏动态库，真实 rustc、rustdoc
和编辑器使用同一份声明生成后端。它仍是标准 Rust 过程宏机制，不需要修改原版
rust-analyzer，也不复制 rustc 的属性展开管线。

## 使用

先按仓库 README 准备完整工具链，并自行配置现有 Rust 项目的 core 与业务依赖。
在项目或 workspace 根目录执行：

```sh
cargo nestrs init
```

`init` 检查选定目标并初始化 Nestrs 开发环境。默认生成
`<Cargo target>/nestrs/ide/rust-project.json` 与相邻的 `rust-analyzer-settings.json`，
包含标准项目模型、宏服务器和保存检查设置；可以重复执行以刷新现有环境。
命令不会创建项目、添加 Cargo 依赖或改写业务源码，也不会安装编辑器、工具链组件。
当前需要匹配工具链的 rust-src 和 rust-analyzer proc-macro server
（Windows 文件带 `.exe` 后缀）。

如果使用 VS Code，增加明确的客户端选项：

```sh
cargo nestrs init --vscode
```

只有 `--vscode` 才将对应配置合并到 workspace 的 `.vscode/settings.json`。既有
无关设置和 JSONC 注释保留；首次修改前保存 `settings.json.nestrs.bak`，不覆盖已有
备份。无效或含重复键的设置文件会明确报错，不覆盖原文件。

其他支持原版 rust-analyzer LSP 的客户端需要按各自的接入方式加载生成的项目模型，
并应用相邻的设置文件。默认初始化不会自动写入这些客户端的配置；支持 Rust 的编辑器
也不一定使用 rust-analyzer，不能据此承诺所有编辑器都会自动生效。

默认包括 workspace 的全部目标，可以使用 Cargo 的选择与 feature 参数：

```sh
cargo nestrs init -p nestrs-di-example
cargo nestrs init --vscode --manifest-path path/to/Cargo.toml --features audit
cargo nestrs init --output target/editor/rust-project.json
```

切换工作目录后仍可使用生成配置中的绝对路径。
保存检查的 `check.extraEnv` 固定本次选择的 rustc、driver 和私有桥接路径，保留
用户其他环境变量；相对 `--config` 文件路径也会转为绝对路径。编辑器无需继承
初始化命令所在 shell 的工具选择环境。

`init` 面向手动组装后接入 Nestrs 的已有项目，也用于刷新已初始化的环境。
未来的 `cargo nestrs create` 将在 bootstrap 完成后负责创建项目，并在内部复用
初始化能力，交付时即已初始化；通过 `create` 创建项目的用户无需再手动运行 `init`。
`create` 当前尚未实现。

旧的 `ide` 入口已更名为 `init`，不再保留功能别名。此前使用旧入口的项目，需携带
原来的目标、feature 和输出选项重新运行 `cargo nestrs init`，刷新生成的编辑器命令；
此前配置过 VS Code 的项目应运行 `cargo nestrs init --vscode`，以同步更新
`.vscode/settings.json` 中的保存检查入口为 `init check`。内部缓存目录和本文档名保持不变。

业务声明可以保持短属性：

```rust
use nestrs::{factory, injectable};

#[injectable]
struct Settings;

struct Client;

#[factory]
async fn client(settings: Settings) -> Client {
    let _ = settings;
    Client
}
```

`nestrs` 是工具注入的声明命名空间，也支持 `#[nestrs::injectable]`。应用的
Cargo.toml 只声明 core 和业务依赖；不能再添加同名 `nestrs` 依赖来覆盖桥接。
`inject` / `value` helper 支持裸名称和 `nestrs::` 路径。组合 primary 时保留属性
末段名称；crate/module 路径别名可用，任意重命名属性之间的协调不在保证范围内。

## 两条诊断链路

rust-analyzer 的内部名称解析、类型推导和补全读取生成的项目模型。每个直接依赖
core 的编译单元在编辑器图中都有名为 `nestrs` 的私有宏依赖，因而标准宏服务器
可以把注入字段展开为 `Injection<T>`，把 factory 参数展开为正确的共享借用。

保存时，生成配置调用 `cargo nestrs init check`，流式输出真实 Cargo JSON 诊断。
成功后重新生成项目模型，自动反映 Cargo.toml、build.rs 和依赖产物的变化；失败时
保留上一次成功的项目模型。无内容变化不重写文件，避免重载和检查互相触发。
未保存的源码改动由 rust-analyzer 自身分析，不需要先落盘或执行构造器。

改变初始化命令中的 feature/目标选择，或移动、升级工具后，需要重新运行
`cargo nestrs init`；使用 VS Code 时继续附带 `--vscode`。既有 `diagnostics.disabled`
等用户偏好会保留，CLI 不新增诊断屏蔽；如果用户原先关闭了某项诊断，工具不会擅自覆盖该偏好。

## 项目模型的来源

模型依据本次成功的 Cargo artifact 消息与实际 rustc 编译单元建立：

- 通过 `--extern` 的真实产物路径关联依赖，保留 crate 重命名和多个版本。
- 保留每个 lib/bin/test/build-script 编译单元的 edition、feature/cfg 和 test 上下文。
  cfg 由相同 rustc 使用实际参数执行 `--print cfg` 获取，包含 profile 的
  `debug_assertions`、panic 策略和 CPU 特性；该查询不编译或展开业务源码。
- 配置 `cfg.setTest = false`，避免编辑器额外给非 test 单元添加 `cfg(test)`；
  真正的 test 单元已经按实际参数进入模型。
- 清空编辑器默认附加的 `cargo.cfgs`，避免它给 release 单元强加
  `debug_assertions` 或 `miri`；实际编译启用的条件已包含在各单元 cfg 中。
- 保留 build.rs 输出的环境变量、OUT_DIR，以及其他过程宏动态库和 host 上下文。
- 用 artifact 身份覆盖旧记录，缓存命中时复用；当前 Cargo artifact 集合排除过时目标。
- 原始源文件是 editor root，不生成供用户编辑的业务副本。generated OUT_DIR 通过
  单独 source include 纳入，target 的其他产物不会变成业务源文件。

桥接动态库和 driver 的内容共同参与工具链缓存身份。宏运行时版本由固定工具链
保证，升级后不会复用之前的编译缓存。模型无法准确关联的依赖会报错，不退化成
缺少依赖的项目图。

## 验证与边界

```sh
python3 tools/verify-ide.py --skip-build
# 显式选用现有 LSP server：
python3 tools/verify-ide.py --skip-build --rust-analyzer /path/to/rust-analyzer
```

验证器在 target 中复制独立 fixture，调用真实 CLI 和原版 LSP server，记录项目图、
协议消息和诊断结果；不修改原始 fixture 或真实编辑器配置。覆盖冷启动、注入字段
与工厂参数 hover、方法补全、外部定义跳转、未保存编辑、真实错误与诊断恢复，
以及 feature、宏生成声明、build.rs 的 cfg/env/include 产物。报告位于
`target/nestrs-ide-verification/report.json`。

真实错误对照使用默认启用的 E0308 类型不匹配验证未保存编辑的实时诊断，保存后的
E0425 名称错误由配置的真实 rustc 检查验证。没有额外开启 rust-analyzer 实验诊断，
也没有通过关闭诊断消除属性或类型的红线。

项目准备适配 Linux `x86_64-unknown-linux-gnu` 和 Windows
`x86_64-pc-windows-msvc` 的固定工具链。Windows 使用本机 `.exe` 和 `.dll`，在
PowerShell 中执行相同 `cargo nestrs init`，使用 VS Code 时加 `--vscode`；需要
Windows MSVC 开发工具，
不需要 WSL。生成模型与设置包含本机绝对路径，不能把 Linux/WSL 的生成文件直接
复制给 Windows 编辑器；切换平台后应在对应平台重新运行 IDE 准备命令。

Windows 导出的项目路径统一为普通盘符或 UNC 路径，与编辑器实际打开的
`file:///C:/...` 等 URI 对应。编译器内部使用的 `\\?\` canonical 路径不会让同一个
源文件在编辑器中成为两个不同文件；root module、source include/exclude、宏动态库、
sysroot 及路径环境变量按同一规则处理。仅有 verbatim 语义才能表示的文件名和设备
路径会被明确拒绝，避免为了消除前缀而改变文件身份。保存检查也先统一 workspace
身份再计算缓存目录，防止首次准备与编辑器保存使用不同缓存而反复重写模型。

2026-09-28 的原生 Windows MSVC 验收使用固定 Rust `1.98.0` 和未经修改的
rust-analyzer `0.3.3049`，通过了完整 `tools/verify-ide.py`：default、alternate、
release 三种配置，每种项目模型共 29 个 crate（包含私有 bridge）；覆盖冷启动、注入字段
与工厂参数类型、补全、定义跳转、未保存编辑、E0308 错误与恢复，以及配置中的保存
检查、E0425 编译诊断和模型稳定性。不支持的 cfg 配置也验证了拒绝与旧模型保留。
测试使用编辑器通常发送的 Windows 文件 URI，未向 RA 父进程的 PATH 手动加入
sysroot/bin，未关闭诊断。Linux 的既有原版 LSP 回归继续保留。

`native_host` 集成测试在两个平台启用，覆盖包含空格/中文的路径、真实 build.rs
输入、私有 bridge 路径和配置的保存检查，并已通过两种 host 的实际执行。完整 LSP
交互验收与该项目模型回归分开记录，不能由模型生成成功推断整个编辑器已通过验收。
完整引用重命名、每个编辑器 UI、所有属性/derive/build-script 组合和其他 host 仍需
分别验收。支持标准声明展开不等于
支持完整跨 crate 候选汇总；DI 全图结构验证仍在容器 build 或 graph 诊断入口执行。
rustdoc 的标准展开可经 CLI 使用，但独立 doctest 内新增 trait 绑定不会自动经过
两阶段 driver。应用级 Clippy 入口尚未交付。

原版 rust-analyzer 会合并 host 的默认 cfg。若实际参数移除了其中某项，例如
`panic = "abort"` 或禁用默认 CPU 特性，IDE 准备会明确拒绝并保留旧模型，避免
两个互斥分支同时激活。release/debug-assertions 变化和增加 CPU 特性可以表示。
此限制只涉及 IDE 模型；应用的 `cargo nestrs check/build/run` 不受它限制。
