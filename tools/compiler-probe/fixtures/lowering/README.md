# 编译前端时序探针

在仓库根目录运行：

```sh
python3 tools/compiler-probe/verify_lowering.py
```

该探针使用与阶段 A/B 相同的 `toolchain.json`，完整校验 rustc release、commit 和
host。只给构建探针 driver 的子进程设置 `RUSTC_BOOTSTRAP=nestrs_lowering_probe`；
被测应用没有启用该环境变量，不修改默认工具链，也不改写 fixture 源码。
结果、逐阶段 AST 记录、诊断及可执行文件均写入 `target/nestrs-lowering/`。

这是编译器集成边界验证，不是新的 DI 声明入口。示例里的
`#[nestrs::injectable]` / `#[nestrs::factory]` 只是用于证明工具属性的行为，
并不意味着生产 API 已采用该名称或已经实现对应改写。

验证覆盖四次编译：

| 用例 | 预期 |
| --- | --- |
| `coverage_default` | 根 AST 只含直接声明和尚未过滤的 cfg 分支；展开后的 AST 含外部模块、宏生成声明及选中的分支。完成编译和运行断言。 |
| `coverage_alternate` | 切换 feature 后，展开 AST 只包含相应 cfg 分支。完成编译和运行断言。 |
| `inert_needs_lowering` | driver 接受工具属性，但 dyn 字段及按值 factory 参数仍触发 E0277，证明合法属性不等于 DI lowering。 |
| `plain_rustc_unknown_tool` | 普通 rustc 报 E0433，证明命名空间属性并不会自行成为合法的框架声明。 |

`coverage.rs` 还验证宏生成来源和外部文件位置可被保留，`derive(Debug)`、宏调用处的
类型别名以及私有外部模块声明继续遵守普通 Rust 规则。这些断言没有证明**发生 DI
改写时**的 derive 顺序和卫生性；完整迁移仍需单独验证。

当前 pinned compiler 的接口边界为：

* `Callbacks::after_crate_root_parsing` 提供可变 AST，但调用时外部模块尚未解析，
  `macro_rules!` 产物也尚不存在。
* `override_queries` 可以覆盖 `registered_tools`，此探针通过
  `rustc_resolve::registered_tools_ast` 保留正常工具集合后加入 `nestrs`。
  这只注册惰性工具属性，不提供属性展开器。
* `Callbacks::after_expansion` 能观察完整展开 AST，但其正常编译流程已执行名称
  解析，不能借此随意追加生成声明而跳过重新解析和检查。
* `Config::register_lints` 的 early lint visitor 接收共享 AST 引用，不能作为
  可变 item lowering 回调。`CStore::load_macro_untracked` 是方法而非可覆盖的 query。
* `ResolverExpand::register_builtin_macro` 需要访问正在执行展开的 resolver。
  标准流程在私有 `configure_and_expand` 内创建、驱动该展开过程，当前 Config
  没有直接暴露对应的扩展注册回调。复制或修改这段流程将成为编译器适配层的额外
  维护范围，不能包装成一次简单的 callback 配置。

因此本阶段保留薄过程宏作为已有的早期展开入口，优先抽离共同的声明处理与代码
生成后端。未来可以继续验证更深的 rustc 集成，但不能把过程宏换成 no-op 后宣称
已由 CLI 接管其类型改写工作。
