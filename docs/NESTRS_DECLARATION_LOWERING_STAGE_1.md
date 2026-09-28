# 声明宏迁移第一阶段：共享后端与早期展开验证

> 历史阶段记录：本文保留实现当时的包划分、命令与验证结果。当前应用不依赖公开宏库；
> CLI 管理的私有 `nestrs-tool-bridge` 通过标准过程宏调用 `cargo-nestrs` 内唯一生成后端，
> 独立 `nestrs-codegen` package 已移除。当前入口和支持边界见
> [Cargo 工具链说明](NESTRS_CARGO_TOOLCHAIN.md)。本文中的旧探针命令保留阶段背景，
> 不作为当前 checkout 的应用构建方式。

本阶段把服务声明的实现从 proc-macro crate 提取到普通 Rust 编译期库
`nestrs-codegen`。`nestrs-macro` 保留薄入口，委托同一个后端完成声明分析、类型
改写和注册生成。现有业务声明语法、DI 安全边界与运行方式保持一致。

这完成了后端解耦；完整的 driver 内属性展开仍是后续工作。当前实验没有提供
生产 `cargo nestrs` 命令，也没有把 `injectable` / `factory` 变为空操作标记。

## 实际职责

| 组件 | 当前实现 |
| --- | --- |
| `nestrs-macro` | 四个 proc-macro 入口：`injectable`、`factory`、`primary`、`bind`；负责与 Rust 过程宏 ABI 互转 token |
| `nestrs-codegen` | 参数解析、字段来源分析、`primary` 属性交接、类型改写、构造与 factory adapter、cleanup adapter、泛型蓝图、依赖描述、linkme 注册及编译器 marker |
| `nestrs-core` | 描述 ABI、linkme 宿主、全图验证、冻结图、Tokio 调度、lease 与关闭 |
| 自动绑定 driver | 继续使用阶段 B 的两次编译，在真实类型信息上发现和生成 trait 绑定 |
| 早期展开探针 | 验证命名空间注册、root parsing 与完整 expansion 的覆盖差异，以及 inert 属性无法完成类型改写的边界 |

`nestrs-codegen` 是 workspace 内部 package，设置 `publish = false`，只依赖编译期
工具 `zyn`。它没有运行时、服务注册表或另一套 DI API。生成的代码仍引用
`::nestrs_core`，linkme 仍只有 core 一个宿主。core 不反向依赖 codegen、macro
或 rustc 内部库。

`nestrs-macro` 的 `injection` feature 同时控制 core 和 codegen 依赖；`zyn` 保持
不受 feature 控制的基础依赖。关闭 injection 后不会导出 DI 属性宏。

## 后端接口与展开顺序

后端提供四个文档隐藏的内部函数：

```text
expand_injectable(args, input) -> TokenStream
expand_factory(args, input) -> TokenStream
expand_primary(args, input) -> TokenStream
expand_bind(args, input) -> TokenStream
```

输入输出是带 span 的 `proc_macro2::TokenStream`。入口在普通进程中可调用，不需要
过程宏执行上下文。后端直接处理 token 和 syn AST，不把输入转为字符串再重建。
proc-macro 入口只做 ABI 转换，因此 Rust 的正常宏展开机制继续管理属性顺序、
外部模块、cfg 和 macro_rules 生成项；此阶段没有添加自制的全项目源码预处理器。

`primary` 与 `bind` 一起迁入后端，是为了保持已有属性协作与注册规则只有一份
实现。原来 `injection/macros`、`sub_macros`、`macros_attrs` 与 `utility` 的职责
划分保留在新 package 中。`TraitBinding` 仍是投影关系，不生产实例。

## 为什么暂时保留薄宏

`injectable` 会把字段 `T` 改为 `Injection<T>`，可选字段改为
`Option<Injection<T>>`。`factory` 会把声明参数改为借用 factory 输入 frame 的
共享引用；异步函数在 await 期间依赖这个真实借用。这两种转换必须在受影响代码
的类型检查前完成。

固定工具链下的实验分别确认了：

1. `after_crate_root_parsing` 提供可变 root AST，但外部模块还未加载，
   `macro_rules!` 生成的服务也未展开；此时仍能看到随后被 cfg 移除的项目。
2. driver 可以通过注册工具命名空间使 `#[nestrs::injectable]` 等属性合法出现，
   包括外部模块和宏生成项。这些属性是 inert，只会保留在语法树上。
3. `after_expansion` 才能看到上述展开后的项目和 cfg 选择结果；它所处阶段也已
   完成名称解析，不能直接追加一批未参与解析的生成项并假定它们有效。
4. 仅注册命名空间后，原始 dyn 注入字段和按值书写的 dyn factory 参数仍因缺少
   类型改写而编译失败。属性合法与声明可正确执行是两个独立条件。

这些结果没有证明未来接入不可行；它们说明目前公开的 callback 组合尚未提供
已验证的完整替代路径。下一步应验证编译器展开器接入与带卫生性的 token/span
桥接，调用已提取的同一后端。不能把某个仅能转换 root 文件的原型宣传为完整
支持外部模块、derive 与宏生成服务。

## 验证方式

无需 rustc-dev 的共享后端、宏和运行时回归：

```sh
cargo test -p nestrs-codegen
cargo test -p nestrs-macro --test runtime_lowering
cargo test -p nestrs-macro --test ui
cargo check -p nestrs-macro --no-default-features
```

后端测试直接从普通测试进程展开声明，验证字段改写、工厂 frame 签名、泛型蓝图、
primary 交接及错误诊断。运行时回归额外覆盖外部模块、调用方提供的宏参数、
cfg_attr、derive 顺序、别名、泛型服务、同步/异步 factory、optional，以及借用
跨 await 后仍有效和最终释放。已有 UI 回归继续验证 factory 借用不能逃逸。

固定 rustc 版本的早期展开与自动绑定集成：

```sh
python3 tools/compiler-probe/verify_lowering.py
python3 tools/compiler-probe/verify_autobind.py
```

早期展开报告输出到 `target/nestrs-lowering/report.json`，自动绑定报告输出到
`target/nestrs-autobind/report.json`。两个验证器检查
[`toolchain.json`](../tools/compiler-probe/toolchain.json) 的 release、完整 commit
和 host；实验 driver 的 bootstrap 设置只存在于其编译子进程，不修改全局工具链
或应用源码。普通 workspace 构建不依赖 rustc-dev。

完整验收仍运行 workspace check、test、fmt、clippy 与宏关闭 feature 检查。
UI stderr 不通过批量覆盖来适应迁移，原有诊断是需要保持的回归契约。

2026-09-28 在当前固定工具链下完成以下验证：

| 验证 | 结果 |
| --- | --- |
| codegen 单元测试 | 47 项通过，包含 5 项新增的普通进程入口测试 |
| 宏 UI 回归 | 52 项通过，既有 stderr 保持不变 |
| 声明上下文与工厂借用集成 | 外部模块及各类声明实际构建、查询、关闭通过 |
| 早期展开探针 | 4 项通过，包含两个预期编译失败的反例 |
| 阶段 B 自动绑定 | 14 次集成运行通过；6 项源码覆盖层与快照测试通过 |
| 最初的语义探针 | 5 项通过 |
| workspace 验收命令 | check、test、fmt、clippy `-D warnings`、宏 no-default-features check 全部通过 |

workspace 检查日志保存在 `target/nestrs-lowering/workspace-validation.log`。

## 后续迁移条件

正式接管声明展开仍须覆盖：每轮宏展开产生的声明、cfg 与外部模块、第三方过程宏
和 derive 顺序、token 卫生性与源码映射、私有作用域，以及编译失败位置。
工厂返回类型别名的语义归一化也尚未在本阶段新增，继续沿用现有宏支持范围。

编辑器仍通过薄宏获得展开后的类型信息。本阶段没有实现 rust-analyzer 专用
协议；设置外部 check 命令本身不能替代补全和类型推导。只有后续展开与编辑器
链路验证完整，才能移除当前 proc-macro 前端。

CLI Cargo 编排、跨 crate 自动绑定、增量缓存与 HTML 迁移继续按
[工具链方案](NESTRS_COMPILER_TOOLCHAIN_PLAN.md) 推进，不能从本阶段的后端抽取
推断这些能力已经完成。
