# Nestrs 编译器工具链演进方案

当前架构将应用声明、语义自动绑定、编辑器接入和 HTML 导出统一交给 `cargo-nestrs`
管理。应用仅依赖 `nestrs-core` 和业务所需库；标准属性宏来自工具内部的
`nestrs-tool-bridge`，生成后端唯一保留在 `cargo-nestrs/src/codegen`。
私有桥接使用正常的 Rust 过程宏展开机制，不恢复公开宏依赖或复制的原生编译前端。
当前使用方式见 [Cargo 工具链说明](NESTRS_CARGO_TOOLCHAIN.md) 和
[IDE 接入](NESTRS_IDE.md)。

## 1. 已确认的体验与边界

应用通过以下入口工作：

```text
cargo nestrs doctor
cargo nestrs check
cargo nestrs build
cargo nestrs run
cargo nestrs test
cargo nestrs graph
cargo nestrs init
```

用户从工具提供的 `nestrs` 命名空间导入 `injectable`、`factory` 和 `primary`，
声明构造方式、lifetime、key 和 primary，再编写普通 `impl Trait for Concrete`。
完整路径 `#[nestrs::injectable]` 同样使用标准过程宏机制。业务代码不写 bind，
Cargo.toml 不添加宏 package；`nestrs` extern 名冲突会明确失败。

私有桥接负责 token ABI 转换并调用同一生成后端。真实 rustc、经 CLI 转发的
rustdoc 和原版 rust-analyzer 都使用该桥接，但只有 Nestrs driver 执行两轮语义
编译和自动绑定。普通 Cargo 不提供应用所需的 extern 注入环境；应用级 Clippy
接入仍未实现。core 和构建工具可通过普通 Cargo 独立检查。

## 2. 职责分离

| 位置 | 职责 |
| --- | --- |
| `nestrs-core` | Provider/Binding 描述、编译器清单消费、图验证与冻结、lease、Tokio 调度与关闭、只读图数据 |
| `cargo-nestrs/internal/bridge` | `publish = false` 的 `nestrs-tool-bridge` package，提供标准 proc-macro 入口供工具注入 |
| `cargo-nestrs/src/codegen` | 属性参数与字段分析、类型/签名改写、constructor/factory/cleanup adapter、ProviderDefinition 与注册生成 |
| `cargo-nestrs/src/compiler` | 固定 rustc 接入、语义发现、trait 求解、合法模块生成及最终编译核查 |
| `cargo-nestrs/src/commands` | Cargo 编排、工具链诊断、产物隔离和 graph 目标选择 |
| `cargo-nestrs/src/ide` | 根据实际 Cargo artifact 与编译单元生成编辑器项目模型、保存检查与配置合并 |
| `cargo-nestrs/src/graph.rs` | HTML 数据转义、页面和文件导出所用渲染能力 |

core 不依赖编译器内部 crate；应用运行时不链接构建工具。`TraitBinding` 仍只是
concrete 到接口的投影，实例来自 `Provider::{Class, Factory}`，不会新增一套
生命周期或实例缓存。工具内部不存在另一套公开声明 API。

## 3. 编译链路

1. CLI 校验 pin 中的 release、完整 commit、host、rustc-dev 与 driver 身份，定位
   匹配工具链构建的 bridge。IDE 另需匹配的 rust-src 和标准过程宏服务器。
2. Cargo 决定构建单元、依赖、features、cfg、target 和 build script。driver 为
   直接依赖 core 的编译单元注入 `--extern nestrs=<bridge dylib>`，为实际下游
   metadata 消费者加入 bridge 的依赖搜索路径。
3. rustc 正常调用 bridge，共享后端在 typeck 前生成 Injection 字段、factory frame
   借用签名、注册和 typed marker。宏展开、cfg、外部模块和卫生性仍由 rustc 负责。
4. 第一轮完整语义分析收集 provider、接口需求和有限闭合类型，通过真实类型与
   Unsize/trait 求解选择可生成的投影。
5. 生成代码在 target 留存，通过保持原文件身份的 FileLoader 覆盖进入合法模块。
   第二轮重新展开、类型检查和编译，核查有没有遗漏绑定；不修改用户磁盘源码。
6. build 交付编译结果，run/test 执行业务或测试，graph 执行所选 binary 的静态
   图诊断入口并输出 HTML。rustdoc 由 driver 包装入口转发到真实固定 sysroot 工具。

当前 `check/build` 不执行完整链接单元的容器图检查。缺失依赖、图环、生命周期
闭包等规则在 `ServiceProvider::build()` 或 `cargo nestrs graph` 的诊断入口中验证。
类型检查成功、全图结构合法和外部资源初始化成功是三种不同保证。

## 4. 自动绑定规则

候选来自已声明 provider、factory 成功输出、已知闭合泛型；接口需求来自字段、
factory 参数和查询。普通 impl 本身不注册服务，不选择 lifetime，也不展开无限类型空间。

使用 rustc 的类型归一化、实际 DefId 和 trait 求解确认可转换关系，生成普通安全
coercion 与既有 required/optional preparer。隐私、对象安全性、auto trait 和
关联类型约束仍由 rustc 检查。不伪造 vtable、扩大引用生命周期或通过源码名字
猜测 trait 关系。

精确 type/key 显式 provider 优先于 fallback 蓝图；key 不回退；primary 只解决
同 key 下的 trait 多候选；重复 concrete provider 一律错误；optional 候选存在
时继续检查歧义、循环和生命周期。新增实现可能改变 optional 是否出现，也可能
导致歧义，这些是自动发现必须报告的行为。

跨 crate 汇总读取上游注册回调、查询根和闭合蓝图的类型化 MIR，metadata-only check
也编码所需描述。已知服务所属 crate 生成潜在接口投影，保存到独立自动能力目录；
私有类型的投影留在合法模块中。潜在闭合泛型及其依赖只补齐能力，不把未使用蓝图
变成实际图根。实际接口请求才触发必要的泛型展开。

最终 core 图编译器按根和依赖需求迭代启用能力，随后完成验证、冻结。
兄弟 crate 的同一自动 pair 幂等合并，显式 pair 优先而显式重复仍报错；不同 concrete
继续参与同 key 的 primary 选择。公开重导出与别名使用合法路径，不把临时 DefId
数值或类型指针持久化为稳定身份。支持范围见 [跨 crate DI](NESTRS_CROSS_CRATE_DI.md)。

## 5. IDE 与文档工具

`cargo nestrs init` 初始化或刷新手动组装的现有项目，检查选定目标，默认生成
标准 rust-project.json 和相邻的 rust-analyzer-settings.json，为直接依赖 core
的编辑器 crate 提供名为 `nestrs` 的宏依赖。它不创建项目、不添加 Cargo 依赖。
模型依据真实 Cargo artifact 与 rustc 单元保留依赖身份、重命名、多版本、edition、cfg、test、build.rs 环境和
OUT_DIR。不能准确关联依赖时明确失败，不生成缺少依赖的项目图。

只有 `init --vscode` 才写入 VS Code 配置，保留 JSONC 注释和无关设置，第一次
变更前备份一次。其他 rust-analyzer LSP 客户端仍需加载生成项目与设置，不承诺
所有支持 Rust 的编辑器都使用或自动接入 rust-analyzer。保存检查调用
`cargo nestrs init check`，输出真实 JSON 诊断；成功才更新项目模型，失败保留原模型，
没有变化时不重写。未保存编辑由原版 LSP 分析，不需要先写文件或构造服务。

未来 `cargo nestrs create` 在 bootstrap 完成后负责创建新项目，内部复用初始化
能力并交付已初始化项目；用户无需在创建后再执行 `init`。该命令属于后续规划，
当前不实现；`init` 保留为已有项目接入与刷新入口。

模型 cfg 来自同一 rustc 使用实际参数的 `--print cfg`。`cfg.setTest = false`
及 `cargo.cfgs = []` 阻止编辑器额外合成条件；真实 test/profile 条件已包含在模型。
`check.extraEnv` 固定选定的 rustc、driver 和 bridge，同时保留其他环境变量。
原版 rust-analyzer 仍会合并 host 默认 cfg，因此 IDE 明确拒绝移除默认条件的
配置，例如 `panic = "abort"` 或禁用默认 CPU 特性，并保留旧模型。debug/release
及增加 CPU 特性可表示；应用 check/build/run 不受此 IDE 限制。

真实 LSP 已验证冷启动、注入字段和工厂参数 hover、补全、定义跳转、未保存编辑、
错误诊断及恢复，以及 feature、宏生成声明和 build.rs 的 cfg/env/include。
CLI 不关闭诊断；这些验证不代表所有编辑器 UI、重命名操作和任意属性组合均已验收。

CLI 的 rustdoc 入口先由 driver 检查真实文档源码，再交给固定 sysroot 的真实
rustdoc 发现与运行示例。每个独立 doctest 都经过完整 driver，支持新声明、自动
trait 绑定、闭合泛型和 factory 借用；相对 include 保留原文档目录。
最新实现和验收见 [编译器注册清单迁移验收](NESTRS_COMPILER_REGISTRY_VALIDATION.md)。

## 6. 图与 HTML

CLI 复用真实 binary 的链接集合，通过编译器生成的诊断入口检查 graph，不进入
业务 main。它读取静态描述，不执行服务构造、factory、Default、value 或 cleanup。
它仍然运行一个目标程序，不能称为纯 Rust 类型检查。

HTML 和文件输出已从 core 移入 CLI。core 保留只读 JSON 与图验证，不再保留
`graph_output`、`graph_output_path()` 和 `BuildError::GraphExport`。
默认输出 Cargo target 下的 `nestrs-di.html`；页面继续支持搜索、缩放、关系和
槽位详情，不表示运行时实例状态。显式单入口失败不替换已有 HTML；项目总览保留
各入口的成功、错误和跳过结果，输出报告后如有错误则以非零退出。

当前每个图入口限于固定 host 可运行、直接依赖 core 且源码中有标准 main 的 binary，
包括 `#[tokio::main]`。省略 --bin 时汇总所选 package 的全部 binary，--workspace
生成项目总览，各入口的图独立。无直接 core、`no_main` 及 cfg_attr 引入的 `no_main` 明确
拒绝。宏生成 main、lib/test/example 图目标及交叉 target 执行需要独立实现和验证，
不能用只链接 lib 的替代程序代表所有目标的完整图。

## 7. Cargo、缓存与工具链升级

当前 CLI 按完整编译器身份以及 driver、bridge 的联合内容指纹隔离 target；graph
再按 package/binary 隔离，避免诊断程序污染业务产物。Cargo 复用不变构建单元，
rustc incremental 暂时关闭。桥接变化会改变缓存身份，验证器同时快照 CLI、driver
与 bridge，避免验证途中被并行构建替换。

producer metadata 保留 bridge 的实际 crate 身份。driver 与 rustdoc 为所有实际
下游消费者加入 bridge 目录的 `-L dependency=...`，包括没有直接 core 依赖的
consumer；只有直接 core 单元获得声明 extern。元数据加载与跨 crate 自动候选
汇总分别处理。

两次编译间检查已读取源码的快照。任意非确定过程宏、二进制输入、环境与 build
script 不在这份文本快照的事务保证之内，不能将当前缓存表述为完整输入事务。

升级流程：

1. 建立新 release/commit/host 兼容项，保留旧版可复现入口。
2. 适配固定版本语义接口、MIR 编码、图入口及工具桥接工件。
3. 运行声明展开、UI、文档、自动绑定、外部泛型、metadata 加载、运行时和 graph 回归。
4. 验证原版 rust-analyzer 的项目模型、宏服务器、交互诊断和缓存失效。
5. 发布匹配的 CLI、driver 与 bridge，不让项目静默跟随系统默认 rustc。

## 8. 已完成迁移与后续验收

| 范围 | 当前状态 |
| --- | --- |
| rustc 语义与自动绑定 | 两次完整编译、typed projection 和实际 DI 运行已形成闭环 |
| 声明后端与前端 | codegen 唯一实现位于 CLI，标准 proc-macro bridge 为工具内部工件 |
| Cargo、文档与 HTML | check/build/run/test、完整 driver doctest 和多 binary 项目图诊断已实现 |
| 编译器注册清单 | 已移除 linkme 与公开 __private，汇总本地/上游描述，保留真实私有边界 |
| 有限外部泛型 | 编码 MIR、公开重导出、key、上游绑定复用及下游 metadata 加载已验证 |
| 跨 crate DI | 上游需求汇总、私有实现能力目录、兄弟 crate 自动 pair 合并和有限闭合泛型 |
| 编辑器 | 原版 rust-analyzer 项目模型、保存检查及实际 LSP 交互已验证 |
| 后续边界 | 更多泛型/auto trait/投影位置、应用 Clippy、其他平台与更多目标/宏组合仍需实现或验收 |

当前回归入口：

```sh
python3 tools/build-toolchain.py
export PATH="$PWD/target/debug:$PATH"
cargo nestrs doctor
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

独立 DI fixture 保留 52 个旧 UI 语义用例，加 3 个误用和 1 个导入成功用例，以及
字段与工厂参数拒绝命名注入 key 的 2 个用例，以及 core 根部无内部导出的负例，
本轮验收共 59 个。
这些用例与实际注册清单/图/实例化/关闭回归均通过工具提供的 bridge。不得批量更新
旧 stderr 来掩盖错误语义改变。

后续验收应分别覆盖：

- 跨 crate 额外 auto trait 形状、泛型约束组合及更多合法投影位置。
- 第三方属性/derive 的更多组合、属性重命名协调和宏生成 main。
- 更多 build.rs 外部输入、feature/target/profile 组合与增量失效规则。
- lib/test/example 等独立链接集合的图导出。
- 编辑器完整引用重命名、更多客户端 UI 和其他 host 的 LSP 体验。
- 应用级 Clippy 和跨 target 诊断入口 runner。

## 9. 历史记录

[阶段 B 自动绑定](NESTRS_AUTOMATIC_BINDING_STAGE_B.md)、
[声明后端提取第一阶段](NESTRS_DECLARATION_LOWERING_STAGE_1.md) 和
[原生工具属性阶段](NESTRS_NATIVE_COMPILER_ADAPTER.md) 保留各阶段的设计和实验。
其中公开宏、独立 codegen 包和复制原生展开管线属于历史路线。当前 `#[nestrs::...]`
是工具注入的标准 extern proc macro；当前契约以
[工具链说明](NESTRS_CARGO_TOOLCHAIN.md)、[IDE 接入](NESTRS_IDE.md) 与本文件为准。
