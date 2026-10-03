# rustc 语义探针

这里保留独立的编译器边界实验及正式自动绑定回归的便捷入口。生产工具使用标准过程宏桥接
与 `nestrs-driver`；探针成功不等于完整 DI、跨 crate 或 IDE 验收。当前生产实现见
[rustc 指南](../../docs/NESTRS_RUSTC_EXTENSION_GUIDE.md)，正式回归见
[工具链指南](../../docs/NESTRS_CARGO_TOOLCHAIN.md#回归入口与核对位置)和
[fixture 索引](../../cargo-nestrs/tests/fixtures/README.md)。

在仓库根目录运行，需要 Python 3.10+ 和 [toolchain.json](toolchain.json) 指定的
rustc/rustc-dev。基础与展开探针支持 `--rustc /absolute/path/to/rustc`，先核对 release、
完整 commit、host 和 compiler metadata，不安装组件或切换全局默认工具链。
当前实验 pin 为 1.98.0、`88d9e12ae178fab0fb5cc050a94da85685d449ea`、Linux GNU x86_64。
探针的固定 Linux 配置不代表生产工具全部 host 的覆盖范围。

仓库另有 [verify-macro-editor.py](../verify-macro-editor.py) 这个历史宏服务器协议
探针，当前直接构建私有 bridge 时缺少 `nestrs_tool_bridge` 的限定 bootstrap 授权；
用其实际环境最小复现了 `proc_macro_def_site` 的 E0554。该已知准备问题尚未修复，
不能将它列为当前应通过的 gate。原版 rust-analyzer 的现行完整 LSP 入口是
[verify-ide.py](../verify-ide.py)，具体覆盖见 [IDE 指南](../../docs/NESTRS_IDE.md#验证与边界)。

## 基础语义

```sh
python3 tools/compiler-probe/verify.py
```

[driver.rs](driver.rs) 输出 JSON Lines，[verify.py](verify.py) 断言以下五项：

| Case | 预期 |
| --- | --- |
| `semantic_default` | 私有/宏生成 impl、开放泛型、闭合别名、关联类型归一化和默认 cfg 可见 |
| `semantic_enabled` | 同样检查，但只包含 enabled 分支的 impl |
| `reject_missing_impl` | 缺少 trait 实现的手写投影报 E0277 |
| `reject_generic_bound` | 不满足泛型约束的手写投影报 E0277 |
| `reject_dyn_incompatible` | 不兼容 dyn 的接口报 E0038 |

正例投影为手写，并位于能够合法引用私有类型的作用域。探针枚举本 crate 的 impl，
没有识别 DI 请求或实现自动投影生成；类型文本只作观察，不是生产类型身份协议。
它不运行 fixture 或服务构造。产物在 `target/nestrs-compiler-probe/`：driver、
`report.json`、逐项语义 JSONL、诊断和 rustc 原始版本。每次重建 driver 并清理同名
旧 metadata，检查停止编译后没有应用 metadata。

## 展开时序

```sh
python3 tools/compiler-probe/verify_lowering.py
```

[lowering fixture](fixtures/lowering/) 的属性是探针注册的惰性工具属性。生产代码中
相同的 `nestrs::injectable` / `nestrs::factory` 拼写是实际过程宏，两者不同。
结果与逐阶段 AST 记录在 `target/nestrs-lowering/`。

| Case | 预期 |
| --- | --- |
| `coverage_default` | 根 AST 尚不含外部模块与宏产物；完整展开后包含它们及选中的 cfg，运行断言通过 |
| `coverage_alternate` | feature 切换后只保留对应分支，运行断言通过 |
| `inert_needs_lowering` | 属性合法但未改写 DI 类型，仍报 E0277 |
| `plain_rustc_unknown_tool` | 普通 rustc 不认识工具命名空间，报 E0433 |

探针证明根解析、展开和名称解析的时序边界：注册 `registered_tools` 不会提供展开器；
`after_expansion` 已经过名称解析，不能随意追加 item 而跳过重新检查；early lint
获得共享 AST 引用，也不能当作可变 lowering 回调。这些限制解释了薄过程宏桥接的
选择，不代表生产 constructor 关联或 AOT 计划由此探针实现。

基础/展开探针仅在构建各自 driver 的子进程设置对应 crate 的 `RUSTC_BOOTSTRAP`，
fixture 子进程不继承该授权。不要设置全局 bootstrap；产物固定写仓库 target，
没有接入正式 CLI 的 Cargo target_directory 选择逻辑。

## 正式自动绑定回归入口

早期自动绑定 fixture 已迁入
[cargo-nestrs/tests/fixtures/auto-binding](../../cargo-nestrs/tests/fixtures/auto-binding/Cargo.toml)，
由 [autobind_contracts.rs](../../cargo-nestrs/tests/autobind_contracts.rs) 统一验证。
正式测试不再读取本目录的历史 fixture；`verify_autobind.py` 保留原调用入口，
只准备工具链并执行该 Rust harness，不实现自动绑定或复制缓存目录算法。

```sh
# 默认先从源码构建匹配的 CLI、driver 与 bridge，再执行正式测试。
python3 tools/compiler-probe/verify_autobind.py
# 工具已与当前源码匹配时：
python3 tools/compiler-probe/verify_autobind.py --skip-build
```

harness 分别构建并运行有效图，覆盖私有类型、泛型、key、factory 优先级、cfg、
实例共享及投影身份；alternate 配置单独检查。`ambiguity` 和 `duplicate_explicit`
在 check / build 阶段核对准确的编译诊断，不再构建全部 binary 后等待运行期 panic。
原有效图的业务与投影断言继续保留。薄脚本每次使用新的
`target/nestrs-autobind/run-*/` 目录保存原始 stdout、stderr、命令退出码及报告，
逐项编译与执行证据由正式 harness 写入 target。构建或测试命令失败会终止后续
步骤，并传播实际退出状态；不能仅凭生成了 report.json 就认定通过。

报告将准备阶段的 `preparation_toolchain` 与实际执行阶段的 `toolchain` 分开保存。
脚本先以相同参数执行 `cargo test --no-run`，完成可能改写 CLI / driver 的测试构建，
再查询执行身份并通过原 Cargo 命令运行测试。结束后保存 `toolchain_after` 并核对
`toolchain_stable`；即使测试通过，身份变化或末态查询失败也不会报告整轮成功。
测试本身失败时仍尽量记录末态，但不会让身份核查错误覆盖原测试退出码。
这项前后核对不隔离任意并发写入，验证期间仍应避免其他构建改写同一工具目录。

不显式选择编译器时，包装脚本让正式 doctor 发现匹配工具链。`--rustc` 或
`NESTRS_RUSTC` 显式选择的编译器先用于查询 sysroot，再把实际编译器路径交给
doctor 完成身份检查；脚本不复制该检查或缓存规则。`--skip-build` 只跳过前置
工具链构建，后续仍执行 Cargo 测试，并非跳过全部编译。完整 driver 单元测试、
源码快照/overlay 契约与真实 IDE 验证有各自入口，不包含在这个包装脚本内。

只观察一个目标时，可以直接调用真实 CLI：

```sh
cargo nestrs run --manifest-path cargo-nestrs/tests/fixtures/auto-binding/Cargo.toml \
  --bin positive --locked --offline
# 预期编译失败，以观察歧义诊断。
cargo nestrs check --manifest-path cargo-nestrs/tests/fixtures/auto-binding/Cargo.toml \
  --bin ambiguity --locked --offline
```

隐藏 bind 入口只用于显式 pair 的内部兼容回归，不是业务推荐 API。
早期整组 verifier 与运行期非法图预期的迁移背景保留在
[修复记录](../../docs/NESTRS_FIXES.md)。真实 rust-analyzer LSP 交互仍由
`tools/verify-ide.py` 独立验证，详见 [IDE 指南](../../docs/NESTRS_IDE.md)。
