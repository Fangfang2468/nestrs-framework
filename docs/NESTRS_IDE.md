# cargo nestrs init：现有项目初始化与 rust-analyzer 接入

应用不依赖公开宏库。`cargo nestrs` 管理一个私有过程宏动态库，真实 rustc、rustdoc
和编辑器使用同一份声明生成后端。它仍是标准 Rust 过程宏机制，不需要修改原版
rust-analyzer，也不复制 rustc 的属性展开管线。

## 使用

先按[工具链指南](NESTRS_CARGO_TOOLCHAIN.md#工具链与命令)准备完整工具链，并自行配置现有 Rust 项目的 core 与业务依赖。
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

`init` 用于现有项目的初始化与刷新；项目创建命令 `create` 当前尚未实现。

旧的 `ide` 入口已更名为 `init`，不再保留功能别名。此前使用旧入口的项目，需携带
原来的目标、feature 和输出选项重新运行 `cargo nestrs init`，刷新生成的编辑器命令；
此前配置过 VS Code 的项目应运行 `cargo nestrs init --vscode`，以同步更新
`.vscode/settings.json` 中的保存检查入口为 `init check`。内部缓存目录和本文档名保持不变。

声明使用工具注入的 `nestrs` 命名空间，应用不添加同名宏依赖。属性、helper、
constructor/factory 和别名的语法统一见[服务声明与查询](NESTRS_MACROS.md)；
下面只说明编辑器怎样获得这些声明的真实类型和编译诊断。

## 两条诊断链路

rust-analyzer 的内部名称解析、类型推导和补全读取生成的项目模型。每个直接依赖
core 的编译单元在编辑器图中都有名为 `nestrs` 的私有宏依赖，因而标准宏服务器
可以把注入字段展开为 `Injection<T>`，把 factory 普通参数展开为正确的共享借用。
lazy 字段显示为 `LazyInjection<T>`，其 `get().await` 返回借用服务。
constructor 的依赖参数按值交付注入令牌，factory 普通参数借用构造 frame，
factory 的 lazy 参数则按值交付可移入返回服务的 `LazyInjection<T>` 或其 optional
形式。编辑器显示的是生成后的真实类型，不能把这些参数模型混为一谈。

服务查询使用 core 的四个普通异步方法；旧查询宏已移除。泛型辅助函数、闭合
impl 的 `Self` 和跨 crate 查询根由真实编译器分析。rust-analyzer 负责当前文档的
名称解析和类型体验，不能单凭编辑器没有红线判断最终入口的 DI 图已通过验证。

原版 rust-analyzer 不运行 rustc 的 query hooks，也不生成最终执行计划入口。
编译器与 core 的私有协议细节统一见[rustc 集成指南](NESTRS_RUSTC_EXTENSION_GUIDE.md)；
编辑器仍使用 core 真实公开类型，不依赖伪造的公开注册 API。

保存时，生成配置调用 `cargo nestrs init check`，流式输出真实 Cargo JSON 诊断。
最终 binary/test 在这次编译中也完成完整 DI 图验证。成功后重新生成项目模型，
自动反映 Cargo.toml、build.rs 和依赖产物的变化；失败时
保留上一次成功的项目模型。无内容变化不重写文件，避免重载和检查互相触发。
未保存的源码改动由 rust-analyzer 自身分析，不需要先落盘或执行构造器。

改变初始化命令中的 feature/目标选择，或移动、升级工具后，需要重新运行
`cargo nestrs init`；使用 VS Code 时继续附带 `--vscode`。既有 `diagnostics.disabled`
等用户偏好会保留，CLI 不新增诊断屏蔽；如果用户原先关闭了某项诊断，工具不会擅自覆盖该偏好。

## constructor 的跨项语义模型

`#[constructor]` 位于 impl 中，而需要改写的字段位于 struct 中。真实构建先由
rustc 完成 `cfg`、标准宏展开和名称解析，再按 struct 身份与参数的真实局部绑定
确定哪些字段接收 `Injection<T>` 或 `LazyInjection<T>`。因此，宏中同名但卫生
上下文不同的变量不会混淆，被 `cfg` 排除的字段也不会进入字段映射。

原版 rust-analyzer 不运行这个 driver hook。`init` 将编译器已经确认的构造选择
作为每个实际编译单元的模型交给私有 bridge，bridge 复用同一代码生成后端应用
这些选择；不会在编辑器中按字段名称或函数文本重新推断 impl。生成后端的
`ConstructorMode` 在普通 rustc 路径使用 `Deferred`，在编辑器路径按模型选择
`Automatic` 或 `Explicit`；两者复用同一个 renderer。test、feature 和 profile
变体各有独立记录。

constructor 模型 v2 同时保存完整构造方法 token 和已分配的编辑器辅助项名称。
真实构建按已认证辅助项的 `DefId` 连接两个属性展开；原版 rust-analyzer 无法使用
这个 rustc 会话身份，因此编译器从该编译单元标准展开后的标识符目录分配无冲突名称，
bridge 精确匹配声明与方法后重放同一连接。业务可以使用内部 helper 的同名成员，
也可以占用某个候选编辑器名称，分配会跳过它；目录包含 trait 默认成员的本地声明和
跨 crate 调用路径，不能只收集 impl 中显式写出的方法。类型、借用检查和业务定义跳转仍保留。

不改变服务声明和构造方法 token 的日常编辑可复用现有模型。新增服务、修改构造
方法、注入参数或字段、改变成功返回值的字段来源，或者调整相关 `cfg` 后，需要保存并通过
配置的 `cargo nestrs init check`，或重新运行 `cargo nestrs init`，才能刷新模型。
模型按完整声明 token 和可用的真实文件位置匹配。同文件内仅行号移动时，可按
完整声明寻找选择一致的记录；部分 proc-macro server 不提供文件 Span 时，只在
当前精确编译单元内匹配完整声明，所有命中的构造模式与字段选择必须一致。无法匹配
或存在歧义会明确报错，不静默退回自动构造，也不关闭 Rust 类型诊断。真实构建主动
清除编辑器模型环境，始终重新分析语义，不使用旧模型代替编译器验证。

## 项目模型的来源

模型依据本次成功的 Cargo artifact 消息与实际 rustc 编译单元建立：

- 根源码按固定 rustc 的真实选项规则识别，再与 Cargo target/artifact 身份对应；
  支持 Cargo.toml 中指定的非 `.rs` 或无后缀入口，以及包含空格、中文的路径。
  选项值即使以 `.rs` 结尾，也不会被当作根源码。doctest 源码检查复用同一规则。
- 通过 `--extern` 的真实产物路径关联依赖，保留 crate 重命名和多个版本。
- 保留每个 lib/bin/test/build-script 编译单元的 edition、feature/cfg 和 test 上下文。
  cfg 由相同 rustc 使用实际参数执行 `--print cfg` 获取，包含 profile 的
  `debug_assertions`、panic 策略和 CPU 特性；该查询不编译或展开业务源码。
- 配置 `cfg.setTest = false`，避免编辑器额外给非 test 单元添加 `cfg(test)`；
  真正的 test 单元已经按实际参数进入模型。
- 清空编辑器默认附加的 `cargo.cfgs`，避免它给 release 单元强加
  `debug_assertions` 或 `miri`；实际编译启用的条件已包含在各单元 cfg 中。
- 保留 build.rs 输出的环境变量、OUT_DIR，以及其他过程宏动态库和 host 上下文。
- 保留编译器确认的 constructor 构造模式与字段选择，按实际编译单元传给 bridge。
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
协议消息和诊断结果；不修改原始 fixture 或真实编辑器配置。检查冷启动、注入字段
与工厂参数 hover、constructor 展开及普通/lazy 字段类型、同名业务关联成员的 hover
和定义跳转、方法补全、外部定义跳转、
未保存编辑、真实错误与诊断恢复，以及 feature、宏生成声明、build.rs 的 cfg/env/include 产物。报告位于
`target/nestrs-ide-verification/report.json`。

验证器的真实错误对照使用默认启用的 E0308 类型不匹配验证未保存编辑的实时诊断，保存后的
E0425 名称错误由配置的真实 rustc 检查验证。没有额外开启 rust-analyzer 实验诊断，
也没有通过关闭诊断消除属性或类型的红线。

冷启动或未保存编辑可能使正在分析的诊断请求失效。验证器保留对 RequestCancelled
和 ContentModified 的重试，并在 `textDocument/diagnostic` 返回
`ServerCancelled (-32802)` 且 `data.retriggerRequest=true` 时重新请求。重试和
短暂等待共用原请求的超时预算；未明确允许重试的取消及其他错误仍使检查失败，
不会通过忽略诊断让验收通过。相关行为由 `tools/test_verify_ide.py` 单独回归。

本地可使用固定 Rust 工具链附带的原版 `rust-analyzer`：用 `rustc --print sysroot`
取得路径后，把其中的 `bin/rust-analyzer`（Windows 为 `.exe`）通过
`--rust-analyzer` 传给上述验证器。也可指定已安装的 VS Code 扩展服务器。
LSP 验证与 `native_host` 的项目模型测试分别执行，记录实际使用的服务器与 host；
仅运行项目模型测试不能证明完整 LSP 交互通过。

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

本指南描述当前代码的支持边界与回归入口；按日期与工作区快照保存的验收结果见
[修复记录](NESTRS_FIXES.md)。
Linux LSP 通过不能替代 Windows 原生结果，`native_host` 模型生成通过也不能
替代完整 LSP 交互。完整引用重命名、每个编辑器 UI、任意属性/derive/build-script
组合和其他 host 需要分别验证；应用级 Clippy 尚未接入。

原版 rust-analyzer 会合并 host 的默认 cfg。若实际参数移除了其中某项，例如
`panic = "abort"` 或禁用默认 CPU 特性，IDE 准备会明确拒绝并保留旧模型，避免
两个互斥分支同时激活。release/debug-assertions 变化和增加 CPU 特性可以表示。
此限制只涉及 IDE 模型；应用的 `cargo nestrs check/build/run` 不受它限制。
`init` 目前也明确限制为 pin 中的本机 host，不能用跨 target 的 graph 能力推断
跨 target 编辑器模型已支持。

## 生成文件丢失时

`rust-project.json`、相邻配置和构造模型位于 target，是可以重新生成的构建产物。
执行 `cargo clean`、删除 target、切换工作区路径或升级工具后，编辑器保存的
`linkedProjects` 可能仍指向旧文件，从而提示 `Failed to read json file`。
请在对应平台的项目目录重新运行初始化：

```sh
cargo nestrs init --vscode
```

其他 LSP 客户端使用 `cargo nestrs init` 并重新加载输出；原来指定过 `--output`、
package、feature 或 profile 时应带上相同选项。初始化成功后重载 rust-analyzer
工作区。若初始化本身失败，应先处理其真实编译诊断；创建空 JSON 或关闭诊断都
无法恢复依赖和 constructor 的语义信息。

## 核对实现

| 契约 | 源码 |
| --- | --- |
| 默认 workspace / all-targets、检查成功后发布、保存命令 | [commands/init.rs](../cargo-nestrs/src/commands/init.rs) |
| 编译单元的 cfg、环境、extern 与构造模型记录 | [capture.rs](../cargo-nestrs/src/ide/capture.rs)、[constructor.rs](../cargo-nestrs/src/ide/constructor.rs) |
| 项目图、host cfg 可表示性与 Windows 路径 | [project.rs](../cargo-nestrs/src/ide/project.rs) |
| JSONC 合并、一次备份与检查环境 | [settings.rs](../cargo-nestrs/src/ide/settings.rs) |
| bridge 的 constructor 选择与严格匹配 | [constructor_ide.rs](../cargo-nestrs/src/codegen/constructor_ide.rs) |
| LSP 行为与取消重试 | [verify-ide.py](../tools/verify-ide.py)、[test_verify_ide.py](../tools/test_verify_ide.py) |
