# Nestrs 私有声明桥接与编译器语义适配

`cargo-nestrs/internal/bridge` 是 `publish = false` 的工具内部 proc-macro package，
名为 `nestrs-tool-bridge`。它导出 injectable、factory、primary 和隐藏的 bind 回归
入口，只转换标准 token 并调用 `cargo_nestrs::codegen::expand_*`。类型/签名改写、
constructor/factory/cleanup adapter、ProviderDefinition、描述回调与 typed marker
在 `cargo-nestrs/src/codegen` 中保持唯一实现。

应用不依赖这个 package。CLI 向 rustc、rustdoc 和编辑器提供同一工件；业务代码
从 `nestrs` 导入属性。公开宏依赖、复制的原生展开前端与独立 codegen package 均不恢复。

## 标准展开与两次语义编译

CLI 对直接依赖 core 的编译单元注入 `--extern nestrs=<bridge dylib>`。标准 Rust
过程宏机制负责 cfg、外部模块、derive、macro_rules 生成声明及卫生性；字段和
factory 参数在 typeck 前转换。后端处理带 Span 的 token，不用源文本扫描替代声明
分析。`nestrs` extern 名冲突会明确报错，不覆盖用户同名依赖。

driver 使用 rustc_private 取得真实语义并生成图入口；不注册工具属性、不替换
resolver_for_lowering_raw、不复制 rustc 展开管线或 token server。第一轮分析
marker 中的类型和请求，求解合法投影。FileLoader 将绑定加入原模块的虚拟输入，
第二轮再次进行完整展开、类型检查和绑定核查。用户磁盘源码不改写，两个阶段对
已读取源码进行快照检查。

宏本身不扫描全 crate 的 impl。应用经 cargo nestrs 获取桥接和自动绑定；普通
Cargo 不提供此环境。完整图校验仍在容器 build 或 CLI 图入口执行，外部资源的
初始化成功只在实际激活时才能确认。

## 桥接 metadata 的下游加载

producer metadata 会保存 proc-macro 依赖的实际 crate 身份 `nestrs_tool_bridge`。
仅在 producer 添加 `--extern nestrs=...`，无法保证 consumer 的递归元数据加载。
工具会为实际 rustc 编译和 rustdoc 添加 bridge 所在目录的 `-L dependency=...`，
包括没有直接 core 依赖、只消费上游库的下游单元。能力/版本探测仍保持原 rustc 行为。

这解决的是元数据可加载性，不会向无 core 的 consumer 注入新的服务声明 extern，
也不等于跨 crate 自动候选汇总。独立 bridge-metadata fixture 覆盖 producer 与
无直接 core 的 consumer lib/bin，避免把所有消费者都额外依赖 core 当作绕过办法。

CLI 缓存身份包含固定编译器身份以及 driver、bridge 的联合内容指纹。更新 bridge
也会隔离缓存；各 verifier 同时复制 CLI、driver 和 dylib，避免使用不同轮工具工件。

## 外部闭合泛型的编码 MIR

本 crate 的声明通过 HIR typed marker 分析。对于依赖库定义、当前 crate 首次
闭合的 `Repository<User>`，适配器解析真实的 `ProviderDefinition::provider`
Instance，从依赖 metadata 读取 MIR，再按真实泛型实参替换 marker 类型。依赖
扩展使用迭代工作队列，不执行服务构造、factory、Default、value 或 cleanup。

`CompilerKey` 从准确的 enum 变体和静态常量读取，保留 default/named/indexed
身份。精确 concrete 类型及 key 的显式 provider 阻止展开对应 fallback 蓝图的
依赖；不同 key 互不覆盖。缺失或不兼容的描述 metadata 明确失败。

core 的编译期 marker 使用 `inline(never)` 保留优化后可分析的调用；marker 没有
实例状态。driver 同时启用固定编译器的 `always_encode_mir`，使 metadata-only
`cargo nestrs check` 产物也包含下游所需描述。仅给方法加 inline 属性不能代替
这个 metadata 编码开关。

外部类型可以定义在 private 模块，再以另一名称公开重导出。辅助代码路径与可见性
判断共用 rustc 的 `visible_parent_map`，第二轮编译继续检查隐私和类型关系，不改变
业务模块可见性来迁就生成代码。

## 复用上游绑定

上游 library 已注册 `Repository<User> -> dyn Store` 时，下游 bin 或 integration
测试再次查询相同配对不会自动生成第二条注册。适配器读取外部回调中真实的
`compiler_binding<C, I>` typed 调用，识别已有精确配对。

这只影响是否生成新的自动绑定。描述回调、显式 binding 和 core 重复绑定检查
保持不变；用户重复显式注册仍在构图时报告。Binding 始终是同一 concrete 实例的
类型投影，不生产独立实例。

外部 metadata 的 DefIndex 表可以有空洞；实现先检查 MIR 表是否含记录，再读取
DefKey 和函数信息，避免查询未编码定义。

## 编译器清单与真实私有模块

运行期不再依赖 linkme，也不导出 `nestrs_core::__private`。`nestrs-core` 保留自身
模块边界；应用与未来 bootstrap 仍单向依赖 core。编译工具只参与构建，不链接进
业务运行时，bootstrap 的 Application、配置与生态组合职责没有下移到 core。

声明宏生成普通 typed constructor/factory/projector 与描述回调。最终 binary 和
测试入口的 driver 汇总本地及上游 metadata 中的回调，校验来源、闭合签名、描述
类型及写入函数签名，再生成一个版本化 registry 入口的 MIR。它按确定顺序调用描述
回调，将 provider、显式 binding、查询根、自动投影能力与闭合蓝图写入本次构图
拥有的快照。rlib 保存描述、MIR 与所需目标代码，没有全局注册安装和链接段扫描。
编译器将描述回调与 core 内部 ABI 函数加入原生代码生成可达性分析，保留它们依赖的
私有静态变量、TLS、inline/const 和泛型代码；这不提升 Rust 源码可见性，避免下游
缺失符号或重复生成同一个 core 函数。

这里的 MIR 生成限于上述固定协议，不替代业务代码的类型检查。constructor、
factory、trait coercion 和 factory frame 仍经过第二轮完整 Rust 检查；entry 不
执行服务构造、`Default`、`value` 或 cleanup。类型检查成功后，图结构验证仍在
`ServiceProvider::build()` 或 `cargo nestrs graph` 的诊断入口完成；外部资源初始化
由 Lazy 查询或 Eager 预热执行。这三种成功不能相互替代。

生成 adapter 引用 core 实际所属的私有模块。driver 在名称解析时允许解析这些
DefId，随后强制审计完整 HIR，只接受真实工具宏的生成 token 或已记录的虚拟源码
区间。业务源码、业务宏、宏的用户实参、别名或 glob 不能因此访问 core 内部。
元数据仍保留真实私有性；普通 rustc 不获得该访问能力，公开重导出也不夹带内部模块。
编译器 registry 入口不能被业务 Rust 源码调用或取函数地址。

`Injection<T>` 是公开的真实强 lease 类型，便于用户和 rust-analyzer 理解字段形状；
其创建与输入准备能力保持私有，不提供 `Clone`、`Copy` 或可变解引用。

## rustdoc 和原版 rust-analyzer

rustdoc 不经过 RUSTC_WRAPPER，因此 CLI 将 RUSTDOC 指向 driver 的文档入口。
所有文档测试都使用 driver builder，包括通过业务库间接依赖 core 的 crate；否则示例
调用上游封装的 DI 函数时，最终可执行文件会缺少编译器注册入口。文档源分析保留真实
crate type，纯文档载体按 lib 读取；实际过程宏和业务库仍通过原 extern 工件加载。
`cargo nestrs test` / `test --doc` 先由 driver 以 `cfg(doc)` 检查真实源码及私有访问，
再从展开后的 HIR 提取文档，交给固定 sysroot 的真实 rustdoc 发现和运行示例。
文档载体只保留文档结构；示例依赖的是 Cargo 构建的真实业务库，载体不替代运行期类型。
每段示例作为独立链接单元重新经过完整 driver，因此新声明、trait 自动绑定、闭合泛型
根与私有注册目录都使用与应用相同的编译流程，不静默跳过 doctest。

代码块的 hidden line、`compile_fail`、`should_panic`、`no_run` 和 `ignore` 仍由
rustdoc 处理。crate 的 `doc(test(attr(...)))` 与 `no_crate_inject` 被保留；固定
rustdoc 在关闭测试合并时忽略 `no_crate_inject`，适配层仅为此选用未出现在文档中的
载体 crate 名称，保留真实 `--extern`，不修改测试代码。用于启用固定 rustdoc wrapper
选项的 bootstrap 只作用于该载体，进入示例编译前即移除。
普通 inherent/trait 方法保留测试名称；复杂 impl self 类型使用稳定 HIR 索引，
`target` 中的 `documentation.origins.json` 记录真实声明和源文件位置。
当前只适配测试入口，未提供 HTML rustdoc 命令。载体与示例源码保存在 `target`，
读取映射保留原文档目录：普通注释对应原 Rust 文件，`#[doc = include_str!(...)]`
对应实际 Markdown 文件；示例中的相对 `include!`、`include_str!`、`include_bytes!`
及嵌套读取继续使用这些目录。两轮编译沿用源内容快照和真实路径审计，业务源码目录
不会写入临时文件。无法保留来源的文档会明确报错，不能用省略示例代替成功。

`cargo nestrs init` 记录真实 rustc 单元并关联本次 Cargo artifacts，生成标准
rust-project.json；core 用户具有名为 `nestrs` 的编辑器依赖，指向真实 bridge
动态库。原版 proc-macro server 与 LSP 可以展开同一声明并进行类型推导、补全和
跳转，不需要修改 rust-analyzer 或关闭诊断。默认初始化同时生成通用设置文件，
只有 `--vscode` 才修改 VS Code 配置；其他 rust-analyzer LSP 客户端需自行加载
项目与设置。`init` 可重跑刷新，不创建项目或添加 Cargo 依赖。

模型保留 Cargo 依赖身份、重命名、多版本、test/cfg/edition、build.rs 环境与
OUT_DIR，并区分 host 过程宏工件。模型不能准确关联依赖时明确失败。保存检查成功
后刷新模型，失败保留原模型；完整操作和真实 LSP 验证见 [IDE 接入](NESTRS_IDE.md)。

## 验证和限制

移除分布式链接注册后的完整 Linux、Windows、示例与 IDE 验收记录见
[编译器注册清单迁移验收](NESTRS_COMPILER_REGISTRY_VALIDATION.md)。

```sh
python3 tools/build-toolchain.py
python3 tools/verify-macro-toolchain.py --skip-build
python3 tools/verify-ide.py --skip-build
python3 tools/verify-graph.py --skip-build
python3 tools/compiler-probe/verify_autobind.py
```

- 跨 crate 已覆盖闭合蓝图、公开重导出、三种 key、已有绑定复用和 bridge metadata
  加载；任意上游 provider 因下游独有接口需求而参与绑定的完整汇总尚未实现。
- 投影必须位于能合法命名 concrete/interface 的源码作用域，不绕过隐私或伪造 vtable。
- 真实 LSP 覆盖冷启动、字段/工厂 hover、补全、定义跳转、未保存编辑、错误与恢复，
  以及 feature、宏生成项和 build.rs；全部编辑器 UI、任意宏组合和其他 host 仍需验收。
- 应用级 Clippy 入口尚未实现；core/工具的普通 Clippy 与真实应用的编译验证分开报告。
- 源码快照不构成非确定过程宏、环境变量及任意外部文件的完整事务。
- 查询根、冻结图、lease、取消与迭代释放契约不因桥接的工具管理方式而改变。

[原生工具属性阶段](NESTRS_NATIVE_COMPILER_ADAPTER.md) 保留为历史记录。当前语法中
出现 `#[nestrs::...]` 表示标准 extern proc macro，不表示恢复该原生展开路线。
