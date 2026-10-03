# rustc 语义探针

这里保留独立的编译器边界实验和早期自动绑定 fixture。生产工具使用标准过程宏桥接
与 `nestrs-driver`；探针成功不等于完整 DI、跨 crate 或 IDE 验收。当前生产实现见
[rustc 指南](../../docs/NESTRS_RUSTC_EXTENSION_GUIDE.md)，正式回归见
[工具链指南](../../docs/NESTRS_CARGO_TOOLCHAIN.md#回归入口与核对位置)和
[fixture 索引](../../cargo-nestrs/tests/fixtures/README.md)。

在仓库根目录运行，需要 Python 3.10+ 和 [toolchain.json](toolchain.json) 指定的
rustc/rustc-dev。基础与展开探针支持 `--rustc /absolute/path/to/rustc`，先核对 release、
完整 commit、host 和 compiler metadata，不安装组件或切换全局默认工具链。
当前实验 pin 为 1.98.0、`88d9e12ae178fab0fb5cc050a94da85685d449ea`、Linux GNU x86_64。
探针的固定 Linux 配置不代表生产工具全部 host 的覆盖范围。

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

## 自动绑定历史 fixture

`verify_autobind.py` 的全 binary 构建和旧运行期 panic 预期**尚未适配当前 AOT 验证**。
不能把它当作当前应全部通过的 gate，也不要要求此混合正负例项目 `build --bins`
成功。当前最终入口在 check/build 阶段拒绝非法图。

| Binary | 覆盖目的与当前预期 |
| --- | --- |
| `positive` | 私有/宏生成 impl、factory-only、闭合泛型、实例共享、scope 隔离、optional/key/primary、cleanup |
| `ambiguity` | 无唯一 primary 的 trait 候选；预期编译失败 |
| `duplicate_explicit` | 重复显式绑定；预期编译失败 |
| `unsatisfied_bound` | 不满足泛型约束时无绑定，optional 返回空 |
| `explicit` | 已有内部显式绑定不被自动重复生成 |
| `cfg_selected` | 默认与 alternate 各只选择当前分支 |
| `factory_override` | 同类型/key factory 优先于蓝图 |
| `factory_other_key` | 其他 key 的 factory 不抑制默认 key 蓝图 |
| `semantic_edges` | 闭合 impl 根、关联类型、auto traits、私有作用域与泛型链 |
| `source_forms` | 宏生成项、类型/const 泛型投影保持实例身份 |
| `higher_ranked` | 闭合 HRTB trait 查询与注入 |
| `unreferenced_generic` | 未物化蓝图不贡献 dyn 请求 |
| `explicit_generic_root` | 显式闭合 binding 触发其蓝图依赖验证 |

`explicit`、`duplicate_explicit`、`explicit_generic_root` 使用隐藏 bind 入口验证内部
兼容边界，不是业务推荐 API。其余业务通过普通 trait impl 自动绑定。可以定向运行：

```sh
cargo nestrs run --manifest-path tools/compiler-probe/fixtures/auto-binding/Cargo.toml \
  --bin positive --offline
# 下列命令预期非零退出，以观察编译期歧义诊断。
cargo nestrs check --manifest-path tools/compiler-probe/fixtures/auto-binding/Cargo.toml \
  --bin ambiguity --offline
```

旧验证器的日志和快照写到 `target/nestrs-autobind/`。当前生产 gate 使用
`compiler-driver` feature suite、跨 crate 验证器与独立 AOT 失败 harness；
`registry_abi` 保留历史测试文件名，但验证当前 v2 入口与真实私有权限。
实际 rust-analyzer LSP 交互由 `tools/verify-ide.py` 验证，详见 [IDE 指南](../../docs/NESTRS_IDE.md)。
