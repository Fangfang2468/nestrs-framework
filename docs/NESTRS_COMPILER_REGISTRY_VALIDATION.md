# 编译器注册清单迁移验收

本次验收日期为 2026-09-29。迁移将 `linkme` 分布式注册替换为最终入口的编译器注册
清单，并移除 `nestrs_core::__private`。`nestrs-core` 与未来 `nestrs-bootstrap`
继续分层；本次没有实现或合并 bootstrap。

## 实现边界

- 声明宏仍生成经过 Rust 类型检查的 constructor、factory、依赖描述和 trait 投影。
  最终 binary/test 的 driver 根据真实类型与定义身份汇总本地、上游 crate 的描述
  回调，生成版本化注册入口；每次构图填充调用者拥有的快照，没有全局可变注册表。
- core 的内部模块保持私有。编译器仅允许经来源认证的生成代码访问内部接口，完整
  HIR 审计拒绝业务源码、别名、glob 和用户宏越界访问；生成入口本身也不能被源码
  调用或取地址。真实 bridge 工件身份参与认证，不能只凭相同 crate 名获得权限。
- `Injection<T>` 作为真实的只读强 lease 类型公开；构造、指针和 lease 字段仍私有。
  rustc 原生字段隐私检查继续生效，没有公开构造、Clone、Copy 或可变解引用。
- core 不依赖 CLI、rustc 内部库或宏 crate；运行期没有引入新的 crate。编译器保留
  必要目标代码，但不提高服务或内部接口在 Rust 源码中的可见性。
- `#[injectable]`、`#[factory]`、短形式 keyed 注入、查询宏、自动 trait 绑定、
  闭合泛型、三种生命周期、图冻结、Tokio 调度、lease、取消和异步关闭契约保持。

实现细节见 [编译器适配说明](NESTRS_COMPILER_ADAPTER.md)。

## Linux 验收

固定编译器为 Rust `1.98.0`，commit
`88d9e12ae178fab0fb5cc050a94da85685d449ea`，host 为
`x86_64-unknown-linux-gnu`。最终应用与 IDE/graph 验收使用同一份不可变 CLI、driver、
bridge 副本，工件 SHA-256 记录于 `target/final-registry-validation/toolchain-sha256.json`。

| 验收范围 | 结果 |
| --- | --- |
| 普通 Cargo 的 core/工具默认成员 | check、test、Clippy `-D warnings` 通过 |
| `cargo nestrs check --workspace --all-targets --locked` | 通过，包含真实应用 |
| `cargo nestrs test --workspace --locked` | 通过，core 89 项、工具库 95 项、CLI 28 项、帮助 12 项、示例业务 3 项和 CLI 7 项 |
| 工具 `compiler-driver` feature 的全部 targets | 通过，包含 driver 16 项单元测试、metadata、内部契约、native host、reachability、registry ABI 与 rustdoc 集成 |
| 内部 DI 契约 | 11 组通过；原有 197 个断言保留，仅迁移到真实 core 内部测试边界 |
| DI fixture 的全部 targets | 通过，包括生命周期、并发关闭、取消、失败、类型、key、循环、冻结图及 59 个 UI 用例 |
| registry ABI 最终补充回归 | 4 项通过，包括入口保护、工件身份、私有上游声明与字段隐私 |
| 私有目标代码可达性 | Debug、fat LTO 均运行成功；覆盖 private static、TLS、inline/const、异步工厂与首次在下游闭合的泛型 |
| 跨 crate 自动绑定 | 22 条 check/run/graph 命令通过，覆盖 Debug/Release、primary、key、optional、重导出、泛型与歧义负例 |
| 独立自动绑定与宏工具链回归 | 14 个自动绑定场景、6 次宏跨 crate Debug/Release 运行通过 |
| 内部访问边界 probe | 28 个场景通过 |
| rustdoc | 2 种 CLI 入口通过；每轮 12 passed、1 个明确标注 ignore 的示例，8 个实际执行标记均验证 |
| 原版 rust-analyzer | default、alternate、release 三组通过；每组 25 crates，覆盖补全、hover、跳转、未保存编辑、错误恢复和保存检查 |
| graph CLI | 25 条命令通过，包括有效/失败/跳过入口、features、缓存和文件输出边界 |
| 真实 Chromium 页面 | 9 项通过；零页面错误、零网络请求，包含 trait/key 路径、重复槽位、入口隔离和大整数 key |
| 格式与静态检查 | `cargo fmt --check`、`git diff --check`、普通与 driver feature 的严格 Clippy 通过 |

新增字段隐私测试通过真实 CLI 验证 9 种非法访问，覆盖 `Injection` 的读取、赋值、
可变借用、解构、struct update、完整构造，以及 provider/error 的私有字段。
这些负例仍由 rustc 原生 `E0616` / `E0451` 拒绝；公开 options 字段的赋值、解构和
程序运行正例通过。

测试迁移没有把运行期检查改成仅检查生成文本。原手写 adapter 截留注入令牌的测试
继续验证 owner 关闭后的读取、cleanup 和最终析构次数。原有文档中的 `ignore` 标记
保持；可执行文档行为另由真实 rustdoc fixture 验证，不把 ignore 数量算作已执行用例。

主要复验命令如下。应用由 `cargo nestrs` 构建，普通 Cargo 命令作用于 workspace
默认的 core/工具成员；这两个验证范围不能混淆。

```sh
python3 tools/build-toolchain.py
export PATH="$PWD/target/debug:$PATH"
cargo check --all-targets
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check

cargo nestrs check --workspace --all-targets --locked
cargo nestrs test --workspace --locked
cargo nestrs test --manifest-path cargo-nestrs/tests/fixtures/di/Cargo.toml --locked --all-targets

# Linux：使用与已构建 driver 一致的固定 rustc/sysroot。
RUSTC_BOOTSTRAP=nestrs_driver LD_LIBRARY_PATH="$(rustc --print sysroot)/lib" \
  cargo test -p cargo-nestrs --features compiler-driver --all-targets
RUSTC_BOOTSTRAP=nestrs_driver LD_LIBRARY_PATH="$(rustc --print sysroot)/lib" \
  cargo clippy -p cargo-nestrs --features compiler-driver --all-targets -- -D warnings

python3 tools/verify-cross-crate-binding.py --skip-build
python3 tools/verify-ide.py --skip-build
python3 tools/verify-graph.py --skip-build
```

详细日志位于 `target/final-registry-validation/`；完整 feature 测试日志为
`target/final-compiler-tests.log`。`applications.json` 记录 workspace 与示例的命令、
退出码和耗时，`ide-graph-summary.json` 记录编辑器和页面验收。

## 真实结账示例

以下命令全部运行成功，示例保持单 binary 的实际项目组织，没有恢复 lib 生成 bin：

```sh
cargo nestrs run -p nestrs-di-example -- sample
cargo nestrs run -p nestrs-di-example -- sample --eager --warm-up-scopes --max-concurrency 1
cargo nestrs run -p nestrs-di-example -- place-order --customer Alice --sku KEYBOARD --quantity 1 --payment card
cargo nestrs graph -p nestrs-di-example --output target/checkout-di.html
```

两种 sample 模式都输出：成功订单 2 笔、剩余库存 2 件、成交金额 597.00 元、审计
4 条，随后完成 scope/root cleanup。图导出为 1 valid / 0 errors / 0 skipped；
页面与图数据均经过自动化检查。

## Windows 原生验收

在独立 Windows 目录，以相同 commit 的
`1.98.0-x86_64-pc-windows-msvc` 和实际 MSVC linker 构建 CLI、driver 与 bridge DLL。
没有使用 Linux 测试结果代替 Windows 验收，也没有修改系统环境变量。

实际通过工具构建、doctor、普通 Cargo 测试、registry ABI、Debug/fat-LTO 可达性、
两种 rustdoc 测试入口、示例 3 项业务与 7 项 CLI 测试，以及 sample、Eager + scope
预热和 HTML 图导出。新增字段隐私回归也单独在 Windows 验证。

工具库 Windows 的 94 项与 Linux 的 95 项差异来自平台 cfg：Windows 有 3 项专属
路径/环境测试；Linux 有 4 项对应的路径、非 UTF-8 参数与信号测试。core 均为 89 项。

Windows 命令、日志、最终源码清单和工具哈希位于
`target/windows-registry-validation/`。本次没有验收 Windows 编辑器，也没有在
Windows 重跑 Linux 的全部 UI、graph CLI 与浏览器矩阵。

## 保留的边界

这次验收证明上述用例在迁移后的实现中通过，不表示所有未来 rustc 版本或任意第三方
宏组合都已验证。编译器适配仍与固定工具链绑定，升级需要维护查询钩子、MIR 和元数据
协议并重跑回归；应用级 Clippy 入口仍未交付。

Rust 类型检查、构图时的全图结构验证、真实外部资源初始化继续是三个独立层级。
`cargo nestrs check/build` 成功不等于执行过容器图验证，也不保证数据库等外部资源
初始化成功。Tokio runtime 退出后的同步安全释放边界与原实现相同，不能承诺那时
仍可完成异步 cleanup。
