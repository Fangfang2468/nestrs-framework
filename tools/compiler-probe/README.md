# rustc 语义探针

该目录保留最初的语义探针、声明迁移的早期展开探针，以及自动绑定回归项目。
自动绑定验证现已迁入正式 `cargo-nestrs` 编译链，不再维护一份独立驱动实现。
原始探针用于验证编译器阶段与语义边界；当前使用方式见
[Cargo 工具链说明](../../docs/NESTRS_CARGO_TOOLCHAIN.md)。

## 声明迁移的早期展开验证

```sh
python3 tools/compiler-probe/verify_lowering.py
```

该探针对比 root parsing 与完整 expansion 阶段，验证外部模块、宏生成声明和 cfg
选择的实际覆盖。它还证明注册工具属性命名空间本身不会执行字段或 factory 参数
改写。报告位于 `target/nestrs-lowering/report.json`，详见
[声明迁移第一阶段](../../docs/NESTRS_DECLARATION_LOWERING_STAGE_1.md)。
这是历史阶段的边界实验。当前服务生成后端位于 `cargo-nestrs` 内部，由
工具私有桥接库的标准过程宏入口调用；driver 以 `nestrs` extern 注入该库并负责
自动绑定，属性仍走标准 Rust 宏展开。
原始探针仍用于证明命名空间注册和真正展开具有不同的语义。

## 自动绑定集成验证

```sh
python3 tools/compiler-probe/verify_autobind.py
```

该命令通过 `tools/build-toolchain.py` 构建配套 CLI、driver 和私有 bridge，运行
包括六项源码快照/覆盖层边界断言在内的 driver 测试，再通过 CLI 对独立 fixture
执行“工具提供的标准过程宏展开 → 语义发现
→ 生成绑定 → 第二次完整编译”，然后运行真实 DI 断言。主业务 fixture 只依赖
core 与 Tokio，宏桥接由工具管理，不含公开宏依赖或 binding 标注；生成代码复用现有 binding/preparer/lease ABI。
当前支持范围见[编译器适配说明](../../docs/NESTRS_COMPILER_ADAPTER.md)，历史结果
保留在[阶段 B 说明](../../docs/NESTRS_AUTOMATIC_BINDING_STAGE_B.md)。
输出在仓库 `target/nestrs-autobind/`，不会改写 fixture 源码。验证器每次仅清理并
重建自己的 fixture package，保留依赖构建缓存。CLI 使用编译器身份以及 driver、
bridge 的联合内容指纹隔离产物；验证器将本次 CLI、driver 和 bridge 一起复制到
自己的 target 子目录，避免同时进行的工具链开发替换正在验证的工件。14 项运行
断言与业务源文件哈希检查继续保留。

原版 rust-analyzer 的实际 LSP 交互由 `tools/verify-ide.py` 独立验证，覆盖冷启动、
类型提示、补全、跳转、未保存编辑及诊断恢复，详见 [IDE 接入](../../docs/NESTRS_IDE.md)。
语义探针和自动绑定运行断言不能代替这项编辑器验证。

## 基础语义探针

在仓库根目录执行：

```sh
python3 tools/compiler-probe/verify.py
```

需要 Python 3.10+、[toolchain.json](toolchain.json) 指定的 rustc 和匹配的 `rustc-dev`
组件。目前实验环境为：

```text
release: 1.98.0
commit-hash: 88d9e12ae178fab0fb5cc050a94da85685d449ea
host: x86_64-unknown-linux-gnu
```

可用 `--rustc /absolute/path/to/rustc` 选择编译器。验证器先检查 release、完整
commit、host 和必要的 compiler metadata，不匹配会报告错误。验证器不安装组件，
不切换默认工具链。基础语义探针不使用 Cargo；自动绑定验证器会构建正式工具链，
需要启用其 `compiler-driver` feature。实验 fixture 未加入 workspace。

当前 driver 使用 `#![feature(rustc_private)]`。仅编译这个探针的子进程设置
`RUSTC_BOOTSTRAP=nestrs_compiler_probe`，范围限制为同名 crate；fixture 的编译进程
移除该变量。driver 的动态库路径只在子进程中配置。此做法用于固定编译器的实验，
不代表正式 Nestrs 工具链的分发方案，不应设置全局 `RUSTC_BOOTSTRAP`。

## 验证内容与产物

[driver.rs](driver.rs) 输出 JSON Lines；[verify.py](verify.py) 编译 driver 并断言
以下五个 case：

| Case | 预期 |
| --- | --- |
| `semantic_default` | 私有 impl、宏生成 impl、开放泛型、闭合别名、关联类型归一化与默认 cfg 分支可见 |
| `semantic_enabled` | 相同语义检查通过，只包含 enabled 分支的 impl |
| `reject_missing_impl` | 手写投影缺少 trait 实现，rustc 报 `E0277` |
| `reject_generic_bound` | 手写投影不满足泛型约束，rustc 报 `E0277` |
| `reject_dyn_incompatible` | 接口不兼容 dyn，rustc 报 `E0038` |

正例内的投影函数均为手写，位于能够合法引用私有类型的模块中。它们证明该作用域
内的转换可以通过类型检查，不证明框架已经能自动生成或插入相同代码。

所有产物保存在仓库的 `target/nestrs-compiler-probe/`：

- `driver`：本机编译器适配实验程序。
- `report.json`：实际编译器身份、命令、退出码及逐项断言结果。
- `*.jsonl`：语义记录，包括类型、源码位置、宏展开标记和归一化结果。
- `*.diagnostics.txt`：对应编译器诊断。
- `rustc-version.txt`：本次 `rustc -vV` 的原始结果。

每次运行重建 driver，并移除各 case 同名的旧 metadata 后重新分析。正例确认
`Compilation::Stop` 没有产生应用 metadata；反例确认失败时没有成功分析记录。
driver 不执行 fixture，不生成应用可执行文件，不运行任何服务构造。
这个实验目录固定输出到仓库 target，尚未接入正式 CLI 的 Cargo target_directory
选择逻辑。

2026-09-28 在上述环境实跑五项全部通过。附加格式验证：

```sh
rustfmt --edition 2024 --check tools/compiler-probe/driver.rs tools/compiler-probe/fixtures/*.rs
```

## 当前边界

基础 `driver.rs` 枚举本 crate 的全部 trait impl，尚未识别 DI provider 和接口请求，
未针对任意候选主动调用 trait solver。它也没有解决辅助代码插入、宏卫生性、
跨 crate 元数据、私有类型投影桥接、Cargo 两阶段构建、缓存或 HTML 迁移。

JSON 中的类型文本仅用于观察与断言，不是可用于生产绑定的类型身份协议。
开放泛型仍是符号类型；读取一个泛型 impl 不意味着可以枚举其所有闭合实例。
这些结论仅说明原始语义探针的边界，不能代表后续工具链实现的能力。

自动绑定实现现位于 `cargo-nestrs/src/compiler/` 和
`cargo-nestrs/src/bin/nestrs-driver.rs`，包含 DI 标记识别、有限候选求解、合法作用域
生成和两次编译；其当前边界以 Cargo 工具链说明为准。此前的三份 `autobind_*.rs`
实验副本已删除，避免两套实现分歧。
