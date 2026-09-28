# Nestrs 原生声明编译器适配层（历史阶段）

> 历史阶段记录：复制的原生属性展开管线、token server 和 rustdoc 拒绝桥接已移除。
> 当前由 CLI 管理私有 `nestrs-tool-bridge`，以标准过程宏调用 `cargo-nestrs` 内唯一后端；
> 应用不依赖公开宏库。当前 `#[nestrs::...]` 语法表示工具注入的 extern proc macro，
> 不表示恢复本文的原生展开路线。本文保留当时的设计和命令；当前实现见
> [编译器适配说明](NESTRS_COMPILER_ADAPTER.md) 和 [工具链说明](NESTRS_CARGO_TOOLCHAIN.md)。

本阶段以 `cargo-nestrs` 中的普通编译期代码和固定版本 compiler driver 接管声明展开。
应用不需要 `nestrs-macro` 或独立的 `nestrs-codegen` package。DI 的运行时 ABI、
provider/binding 分工、冻结图、lease、Tokio 调度和关闭契约保持不变。

## 原生属性在正确的展开位置执行

适配器通过 `registered_tools` 注册 `nestrs` 命名空间，同时替换
`resolver_for_lowering_raw` 中的早期展开编排。该编排保留 rustc 的 cfg 处理、
外部模块加载、宏展开、derive、lint、测试 harness、过程宏 harness 和名称解析。
`ResolverExpand` 装饰器先让原 resolver 登记调用上下文及卫生信息，再将
`nestrs::injectable`、`nestrs::factory` 和 `nestrs::primary` 解释为可执行属性。
用于迁移回归的隐藏 `nestrs::bind` 入口继续复用旧 ABI；正常业务实现不需要它。

字段、参数和声明的处理发生在普通属性展开位置，所以外部 `.rs` 模块和
`macro_rules!` 生成的声明同样受支持。`cfg` 排除的声明不会参与展开；未知的
`nestrs` 属性、独立出现的 inject/value helper 和错误的属性路径会在原始位置报错。
字段 helper 同时接受 `#[inject]` / `#[value(...)]` 和 namespaced 写法。

代码生成后端通过活跃的 `proc_macro` token bridge 运行，但它是链接在 driver
中的普通 Rust library 代码，没有构建或加载一个 Nestrs proc-macro dylib。
桥接保留 rustc token、源文件、Span 和 syntax context，没有把用户文件转换为字符串
再扫描/替换。工具属性没有宏定义 DefId，适配后的 token server 明确允许这种来源；
不伪造宏 DefId，不进行 token/Span 的 unsafe ABI 转换。

该编排和 token server 来自固定 rustc 的对应实现，来源与 MIT 授权全文保存在
`cargo-nestrs/THIRD_PARTY_NOTICES.md`。升级编译器时必须重新核对这些适配点，
不能只修改版本号跳过回归。

## 外部闭合泛型的编码 MIR

本 crate 的声明由 HIR 中的 typed marker 调用分析。对于依赖库定义、当前 crate
首次闭合的 `Repository<User>`，适配器使用 rustc 解析出的
`ProviderDefinition::provider` Instance，读取依赖库 metadata 中的 MIR，
再按该 Instance 的真实泛型实参替换 marker 类型。依赖扩展继续使用同一个迭代工作队列。
不会调用服务构造、factory、Default、value 表达式或 cleanup。

`CompilerKey` 从准确的 enum 变体和静态常量读取，保留 default/named/indexed 三种
身份。当前 crate 中精确 concrete 类型与 key 匹配的显式 provider，仍会阻止展开该
泛型 fallback 的依赖；不同 key 不相互覆盖。缺失或不兼容的描述 metadata 会直接报错。

为使 release 优化后仍能检查描述，core 的三个编译期 marker 使用 `inline(never)`。
它们没有注册或实例状态。driver 同时启用固定编译器的 `always_encode_mir` 选项，
使 `cargo nestrs check` 生成的 `.rmeta` 也包含下游需要的描述；仅给方法加 inline
属性无法突破 rustc 默认的 metadata-only MIR 编码限制。

外部类型可能定义在 private 模块、再以另一个名称公开导出。辅助代码类型路径和
可访问性判断共用 rustc 的 `visible_parent_map`，最终仍由第二次完整编译检查隐私与类型关系。
不会为了通过生成代码而更改业务模块的可见性。

## 复用上游已生成的绑定

上游 library 可能已经注册了 `Repository<User> -> dyn Store`，下游 bin 或 integration
测试又查询同一个 concrete 和接口。适配器从外部 metadata 中读取生成注册回调的
真实 `compiler_binding<C, I>` typed 调用，收集已有的精确配对，然后省去新的自动回调。

该处理只影响“是否再生成一条自动绑定”。现有 linkme 回调、显式绑定和 core 的重复
绑定检查均保持原样。上游或本 crate 中的显式重复注册仍由构图阶段报告；binding
仍然是同一个实例的投影关系。

外部 metadata 的 DefIndex 表可以有空洞。实现先检查 MIR 表中是否有记录，再读取
DefKey 和函数信息，避免对未编码定义调用要求存在记录的 rustc 查询。

## 本阶段的实际边界

- 上述跨 crate 能力覆盖已知闭合泛型蓝图和上游已有绑定复用。它还不等于任意跨 crate
  provider/接口需求的全链接单元汇总。上游声明了普通 provider、但只有下游查询接口，
  且没有任何已知闭合类型或上游绑定可供发现时，不能据此声称候选已经完整收集。
- 自动绑定辅助代码仍需能够在真实源码模块中合法命名 concrete 与接口。没有可用
  源码插入点的宏生成模块等情况会明确报错。
- `rustdoc` 不遵循 Cargo 的 `RUSTC_WRAPPER`。native library 的 doctest 路径尚未
  接管；工具链应明确报告这一边界，不静默跳过文档测试。普通库和 core 的文档测试
  仍交给固定版本 rustdoc。具体用户诊断见工具链使用文档。
- 查询宏仍要求闭合类型，运行时不会物化冻结图之外的新类型。上述 metadata 分析
  不改变 required/optional 查询、生命周期和失败传播规则。

## 可重复验证

```text
python3 tools/build-toolchain.py
python3 tools/verify-native-toolchain.py --skip-build
RUSTC_BOOTSTRAP=nestrs_driver cargo clippy -p cargo-nestrs --features compiler-driver --bin nestrs-driver -- -D warnings
```

`native-cross-crate` 是独立的 Cargo fixture，应用依赖只有 core 与 Tokio。验证器执行
一次 `check --all-targets`，再分别以 debug 和 release 运行三个 binary：

1. 原生命名空间、primary 顺序、cfg、derive、外部模块、宏生成项、async factory 和普通 trait 自动绑定。
2. 下游首次出现的泛型闭合类型、依赖链、别名、私有定义模块的重命名公开导出、三种 key 和 concrete/trait 实例身份。
3. 上游已绑定的 concrete/trait 对在下游再次查询，验证不会出现重复自动注册。

所有执行完成真实 build/query/dispose；验证器比较原始源码、manifest 和 lockfile
摘要，证明编译没有改写应用文件。报告位于 `target/nestrs-native-toolchain/report.json`。
