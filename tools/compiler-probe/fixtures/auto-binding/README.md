# 自动绑定运行时验证项目

这是独立的自动绑定回归项目，未加入框架 workspace。项目依赖 `nestrs-core` 与 Tokio，
从工具提供的 `nestrs` 命名空间导入宏，通过普通 `#[injectable]`、`#[factory]`
声明服务。正式 `cargo nestrs` 注入内部过程宏桥接库并发现、生成绑定；应用 manifest
不依赖公开宏包。正例业务代码只声明服务并编写普通 trait impl；构建须经过 CLI。

| Binary | 成功退出时已经断言的行为 |
| --- | --- |
| `positive` | 私有模块中的宏生成 impl、闭合泛型别名、factory-only 类型、trait 字段与 factory 参数自动注入；concrete/trait 共用 Singleton 和同一 Scoped 实例；两个 scopes 隔离；optional、key 和 primary 选择；cleanup 与 Drop 恰好一次 |
| `ambiguity` | 两个自动候选保留为歧义，图构建在任何服务构造前 panic |
| `unsatisfied_bound` | 闭合泛型不满足 impl 的 `Ord` 约束时不生成绑定，optional 输入和可选查询为空 |
| `explicit` | 迁移期已有的一条显式绑定不被再次自动生成，实例身份保持一致 |
| `duplicate_explicit` | 两条用户显式绑定仍然产生重复 binding 图错误，不被自动去重隐藏 |
| `cfg_selected` | 默认配置与 `--features alternate` 各只有当前生效分支的一条绑定 |
| `factory_override` | 默认、named、indexed key 的显式 factory 覆盖精确类型/key 的泛型蓝图，未使用蓝图的 dyn 字段不产生自动绑定 |
| `factory_other_key` | 其他 key 的显式 factory 不抑制默认 key 蓝图，默认实例保留 optional trait 注入，factory 实例保持独立 |
| `semantic_edges` | 仅由闭合 impl 确定的泛型根、泛型依赖链、关联类型与 auto traits；外置私有模块中的更私有接口使用合法作用域；无关局部 impl 不阻断生成 |
| `source_forms` | 宏生成完整服务类型与 impl；`Repository<String>` 和 const 泛型 `Buffer<8>` 的生成投影保持实例身份 |
| `higher_ranked` | 带 `for<'a>` 的闭合接口在查询和字段注入中自动绑定，内部绑定生命周期不被误判成开放类型 |
| `unreferenced_generic` | 未物化的开放泛型蓝图不会贡献其中的 dyn 字段请求 |
| `explicit_generic_root` | 未查询的显式闭合泛型 binding 仍使其蓝图依赖进入验证，并为其 dyn 依赖自动生成绑定 |

所有 binary 都以成功退出表示断言通过，包括内部捕获预期图错误的用例。只有
`explicit`、`duplicate_explicit` 和 `explicit_generic_root` 为内部注册 ABI 回归而保留
隐藏的 `#[bind]` 入口，它不属于推荐的业务声明 API。`positive`
的指针身份断言使用非零大小的 concrete 类型，避免零大小类型共享地址而导致假阳性。

安装固定版本工具链后，可独立检查项目：

```sh
cargo nestrs check --manifest-path tools/compiler-probe/fixtures/auto-binding/Cargo.toml \
  --all-targets --locked --offline \
  --target-dir target/nestrs-compiler-probe/fixture-check
```

完整生成、源码快照测试与运行：

```sh
python3 tools/compiler-probe/verify_autobind.py
```

工具链要求和产物见[探针说明](../../README.md)。验证器直接构建 `cargo-nestrs`
package 的 CLI、driver 与内部宏桥接库，不再编译另一份实验实现。固定工具链下，13 个默认
binary 与 alternate 配置的 `cfg_selected` 共 14 次运行全部通过，同时通过正式
driver 的 6 个源码覆盖层/快照边界测试。报告位于仓库
`target/nestrs-autobind/report.json`；报告记录实际 driver 指纹及 fixture 源码未改写
检查。本项目不提供手工补齐绑定的构建脚本。
