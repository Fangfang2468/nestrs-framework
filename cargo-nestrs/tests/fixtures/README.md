# 工具链 fixture 索引

本目录集中保存真实 `cargo nestrs` 编译、运行、诊断与编辑器回归所需的小型 workspace。
它们不属于仓库根 workspace；部分 binary 必须编译失败，部分包含用于检测意外执行的
panic/文件写入哨兵，不能把整个目录当成普通示例统一执行 `test --all-targets`。

正常业务示例见[example](../../../example/README.md)，手动观察错误见
[DI 错误示例](../../../example/di-errors/README.md)。本页按负责的 harness 说明用途，
用例数与断言以源码为准，不维护历史验收计数。
历次缺陷的触发条件、修复原因和当时的验证范围统一见
[修复记录](../../../docs/NESTRS_FIXES.md)；本页说明当前测试职责。

## 编译与执行入口

先按[工具链说明](../../../docs/NESTRS_CARGO_TOOLCHAIN.md)准备匹配的 CLI、driver、
私有 bridge 及固定 Rust 工具链，并将 CLI 加入 PATH。Rust integration harness 中的
编译器契约使用 `#![cfg(feature = "compiler-driver")]`；未开启 feature 的普通
`cargo test` 不会执行这些测试。driver 测试的 bootstrap/动态库环境同样按工具链说明配置。

| fixture | 主要覆盖内容与负例边界 | 负责的入口 |
| --- | --- | --- |
| [di](di/Cargo.toml) | 字段声明、同步/异步 factory、输入交付、key、optional、lazy、普通查询、所有权、取消和关闭。UI harness 按正/负例分类编译；不能把内部负例直接加入应用构建 | `cargo nestrs test --manifest-path cargo-nestrs/tests/fixtures/di/Cargo.toml --all-targets`；[ui.rs](di/tests/ui.rs) |
| [aot-invalid](aot-invalid/Cargo.toml) | 缺失、重复 provider、歧义/primary、环、Scoped 传播、服务/参数 lazy，以及直接调用和关联常量里的泛型增长。binary 即使不调用 build 也必须在 check/build 失败 | [aot_graph.rs](../aot_graph.rs)，`--features compiler-driver --test aot_graph` |
| [auto-binding](auto-binding/Cargo.toml) | 私有/宏生成实现、闭合泛型、factory 优先级、key/primary、optional、HRTB、cfg 及投影身份。有效目标逐个运行并核对生成报告；`ambiguity` 和 `duplicate_explicit` 在 check/build 必须分别产生唯一 DI003/DI009 | [autobind_contracts.rs](../autobind_contracts.rs)，`--features compiler-driver --test autobind_contracts`；[registry_abi.rs](../registry_abi.rs) 另复用闭合蓝图正例 |
| [constructors](constructors/Cargo.toml) | 显式同步构造、跨 crate 私有泛型、失败缓存与 cleanup、cfg/宏卫生/别名/遮蔽、参数来源和图槽位；`invalid_*` 检查非法签名、来源流和私有 helper 访问 | [constructor_contracts.rs](../constructor_contracts.rs)，`--features compiler-driver --test constructor_contracts` |
| [query-methods](query-methods/Cargo.toml) | 有限闭合泛型与 Self、标准 trait/运算符、常量及不可变 static 函数指针、嵌套 collect/适配器转发、未执行分支及跨 crate 摘要，并核对无关泛型及覆盖方法不误注册。Debug/Release、extra-root 与仅间接依赖 core 的 app 分别运行 | [query_method_contracts.rs](../query_method_contracts.rs)，`--features compiler-driver --test query_method_contracts` |
| [query-implicit](query-implicit/Cargo.toml) | 多层隐式 Deref/DerefMut、原生析构胶水、泛型及关联字段/GAT、容器与闭包析构；Debug/Release 保留未执行分支并排除 forget/ManuallyDrop，非法图在 check/build 拒绝 | [query_implicit_contracts.rs](../query_implicit_contracts.rs)，`--features compiler-driver --test query_implicit_contracts` |
| [query-clone](query-clone/Cargo.toml) | 元组、嵌套元组及闭包捕获的真实 Clone shim，本地与无 core 外部 helper、未执行分支、不同实例及直接/数组/不克隆对照；Debug/Release 成功运行与非法依赖 check/build 拒绝 | [query_clone_contracts.rs](../query_clone_contracts.rs)，`--features compiler-driver --test query_clone_contracts` |
| [raw-types](raw-types/main.rs) | 自动投影中的原始关联项名、私有类型与重导出、const 泛型、继承约束及 HRTB，根查询和字段注入共享同一实例 | [autobind_raw_types.rs](../autobind_raw_types.rs)，`--features compiler-driver --test autobind_raw_types` |
| [provider-lazy](provider-lazy/Cargo.toml) | 跨 crate 服务级三态初始化策略、私有 factory 的 owned lazy 参数、真实查询共享与图元数据；Debug/Release 均运行 | [provider_lazy_contracts.rs](../provider_lazy_contracts.rs)，`--features compiler-driver --test provider_lazy_contracts` |
| [diagnostics](diagnostics/Cargo.toml) | 原生 Cargo JSON 的主位置/高亮、宏调用来源、同名真实类型、别名、跨 crate 闭合来源、查询歧义、长 cause 与复杂度保护。多数 binary 必须失败，`cfg_valid` 必须通过 | [diagnostics.rs](../diagnostics.rs)，`--features compiler-driver --test diagnostics` |
| [bridge-metadata](bridge-metadata/Cargo.toml) | 只有 producer 直接依赖 core；下游 library/binary/doctest 必须能加载私有 proc-macro metadata，并通过真实运行解析服务 | [bridge_metadata.rs](../bridge_metadata.rs)，`--features compiler-driver --test bridge_metadata` |
| [macro-rustdoc](macro-rustdoc/Cargo.toml) | 库和文档内服务声明、借用 factory、泛型查询、文档属性、宏生成 Markdown 与相对 include 语义；分别执行普通 test 与 test --doc | [rustdoc.rs](../rustdoc.rs)，`--features compiler-driver --test rustdoc` |
| [macro-cross-crate](macro-cross-crate/Cargo.toml) | 标准宏展开、derive/cfg/外部模块、借用 factory；下游首次闭合库蓝图、重导出与复用已有投影，Debug/Release 保留 metadata | [verify-macro-toolchain.py](../../../tools/verify-macro-toolchain.py) |
| [cross-crate-binding](cross-crate-binding/Cargo.toml) | 上游私有实现、兄弟 crate 需求、精确 key/primary、alias、泛型/关联类型/HRTB、lazy 与实例身份；`ambiguous_candidates` 是预期失败，其他目标分别 check/run/graph | [verify-cross-crate-binding.py](../../../tools/verify-cross-crate-binding.py) |
| [graph](graph/Cargo.toml) | check 生成图而不执行业务、缓存隔离、部分失败报告、feature/多 package 选择与旧文件保护；包含非法图、无 core、no_main、宏 main 和有副作用哨兵的目标 | [verify-graph.py](../../../tools/verify-graph.py) |
| [ide](ide/Cargo.toml) | 复制到 target 后用原版 rust-analyzer LSP 验证项目模型、build-script/OUT_DIR、宏展开、构造字段类型、跳转、未保存编辑与实际诊断；原 fixture 不被编辑 | [verify-ide.py](../../../tools/verify-ide.py)，需可用的 rust-analyzer |

表中 Rust harness 参数接在 `cargo test -p cargo-nestrs` 后；这些 harness 自己调用
匹配的 `cargo-nestrs`/driver。Python verifier 从仓库根目录执行，例如：

```bash
python3 tools/verify-cross-crate-binding.py
python3 tools/verify-graph.py
```

这些 Python verifier 默认构建工具链；使用 `--skip-build` 时必须自行确保当前工具工件
匹配源码。报告及日志写入 target，以脚本输出路径为准。运行时断言、编译拒绝、HTML
检查和真实 LSP 交互证明的是不同边界，不能互相替代，也不能据此推断未执行的平台已通过。

## 关键契约的阅读位置

**类型擦除与复杂度。** [query-dynamic](query-dynamic/Cargo.toml) 通过
[query_dynamic_contracts.rs](../query_dynamic_contracts.rs) 验证普通泛型对象在 Box、引用等
载体和跨 crate helper 中转换为 dyn 后，真实调用仍进入冻结计划；同时检查无调用及
不兼容对象形状不会制造查询根。 [type-budget](type-budget/) 由
[type_budget_contracts.rs](../type_budget_contracts.rs) 生成 1100 个不同数组类型的有限元组，
对照原生 Cargo、无 DI 和含 DI 的普通业务类型，并核对真实 DI 类型及无限增长仍被准确拒绝。

**普通外部库转发。** [query-external](query-external/Cargo.toml) 由
[query_external_contracts.rs](../query_external_contracts.rs) 验证不依赖 core 的泛型库
转发 trait 调用、闭包、类型擦除及关联常量，包含被优化消除的分支和原始类型 Self。
同一 helper 的可选 core 依赖构成源码不变的对照；非法图在 Debug/Release 的
check/build 中核对唯一 DI001，不能以其他独立查询掩盖漏根。

**直接输入与 UI。** [runtime_direct_inputs.rs](di/tests/runtime_direct_inputs.rs)覆盖
用户 Default/value 的相对求值顺序，以及类型上下文、泛型、显式构造和生成名称卫生。
全部 typed 输入先读取并核对完整性的生成顺序另由 codegen 单元契约检查。普通输入、factory 借用跨 await、lazy、缓存和关闭行为
由同目录其余 `runtime_*.rs` 分工验证。UI harness 使用 `.stderr` 中的诊断代码、消息
及重复次数作为契约；额外错误、缺失错误或任意失败都不能算通过，原始输出保存在
`target/nestrs-ui/diagnostics`。

**生成名称卫生。** [runtime_codegen_hygiene.rs](di/tests/runtime_codegen_hygiene.rs)、
[runtime_generated_item_hygiene.rs](di/tests/runtime_generated_item_hygiene.rs) 与
[runtime_identifier_hygiene.rs](di/tests/runtime_identifier_hygiene.rs) 验证业务常量、类型、
函数和关联成员与生成项同名时仍保持原始解析，包括 value 的实际副作用、cfg/泛型字段、
显式构造和原始标识符。IDE fixture 另保留同名业务关联成员，真实 LSP 检查其类型和定义跳转。

**构造函数来源。** [source_flow.rs](constructors/src/bin/source_flow.rs)配合关闭/开启
`alternate` 的真实编译、运行与图检查，验证 cfg 删除存储字段不删除参数依赖、宏卫生
和真实绑定身份。`invalid_source_*` 必须命中 constructor 来源诊断；不能只靠后续
Rust 类型错误碰巧失败。始终返回 Err 的合法构造函数在
[failures.rs](constructors/src/bin/failures.rs)检查失败缓存与依赖清理。

**跨 crate 选择。** `cross-crate-binding` 的 `contracts` 通过合法重导出提供接口；
provider 可以来自消费库无法直接依赖的兄弟 crate。私有 factory 返回类型不提升可见性，
不同 crate 中同名类型仍保持不同身份。未请求接口不触发歧义，未请求泛型蓝图不被无端
物化；可用投影进入目录与实际选中投影进入最终计划是两回事。

**查询根与 graph 边界。** `query-methods/indirect-app` 仅间接依赖 core，正常
check/build/run 可以通过；当前 graph 命令仍要求所选 binary 直接依赖 core。
`cross-crate-binding` 和 `graph` 都有故意非法的 binary，全部选择时可以产生包含有效
图与失败条目的报告并非零退出。想单独观察成功图，应显式选择正例 `--bin`。

**源码优先诊断。** [diagnostics.rs](../diagnostics.rs)同时核对 diagnostics fixture
与教学目录前 29 个单错误项目的真实 Cargo JSON；教学目录的多错误例子由
[verify.py](../../../example/di-errors/verify.py)核对。分类与 cause 规则见
[诊断指南](../../../docs/NESTRS_DIAGNOSTICS_DESIGN.md)。

**Markdown 也是测试输入。**
[macro-rustdoc/src/guides/semantics-guide.md](macro-rustdoc/src/guides/semantics-guide.md)
由 [semantics.rs](macro-rustdoc/src/semantics.rs) 的 `#[doc = include_str!(...)]` 实际读取。
其中 Markdown 目录与 Rust 模块目录的同名文件、父目录和嵌套 include 路径用于检验
rustdoc 的源码定位语义。它属于编译输入，整理时必须保留路径、引用资源和代码块语义；
即使仅同步说明文字，也应通过 rustdoc harness 核对。`compile_fail`、`should_panic`、
`no_run`、`ignore` 等代码块语义也由真实 rustdoc 保留。

## 工具与 runtime 的共同边界

应用依赖 core 和业务库；声明来自工具注入的 `nestrs` 私有桥接，不添加公开宏 package
或 `nestrs-reflect` package。库贡献可信局部声明、查询摘要与 typed adapter；每个最终
binary/test/doctest 使用自己的 `__nestrs_reflect_v2` 已验证执行计划。该入口属于私有 ABI，
工具与 core 必须配套。伴随的 `*.nestrs-reflect.json` 与 graph 的 `*.nestrs-plan.json`
用于审阅，runtime 不读取它们。

默认初始化配置通过 `plan_set_options_v3` 传入，包含相互独立的 root/scope 模式和
共享构造上限；该 v3 与上述入口 v2、审阅 JSON v1、IDE constructor 模型 v2 分别
版本化。配置选择由 [startup_config.rs](../startup_config.rs) 核对；
[registry_abi.rs](../registry_abi.rs) 另外检查空图也拒绝不兼容的 Options 协议。

core 不再保留旧 GraphCompiler 或注册参考模型：生产运行期接收冻结计划，执行输入
测试直接构造执行数据。图语义由工具纯模型与真实 driver harness 覆盖；core 执行测试
的组织见[core 测试说明](../../../nestrs-core/tests/README.md)。
