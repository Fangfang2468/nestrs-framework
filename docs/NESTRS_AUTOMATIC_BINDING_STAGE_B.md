# 自动绑定阶段 B：生成绑定接入现有 DI

> 历史阶段记录：本文保留实现当时的包划分、命令与验证结果。当前应用不依赖公开宏库；
> CLI 管理私有 `nestrs-tool-bridge`，标准过程宏调用 `cargo-nestrs` 内唯一生成后端，
> driver 继续通过两次语义编译自动绑定。当前入口和支持边界见
> [Cargo 工具链说明](NESTRS_CARGO_TOOLCHAIN.md)。`verify_autobind.py` 和本阶段 14 项
> fixture 已迁移到正式 CLI 验证链；下文其余阶段描述保留历史背景，
> 不作为当前 checkout 的应用构建方式。

本阶段把上一轮“能读取 rustc 语义”的探针推进到实际绑定生成：实验驱动从服务声明
和查询需求中发现具体类型与接口，生成普通 Rust 投影和 linkme 注册，经过第二次
编译后，由现有 `ServiceProvider`、静态依赖图与 Tokio 运行时消费这些绑定。

这仍是固定工具链下的编译器集成实验。入口是
[`tools/compiler-probe/verify_autobind.py`](../tools/compiler-probe/verify_autobind.py)，
尚未提供生产 `cargo nestrs` 命令。公开 `#[bind]` 继续保留，现有示例和普通 Cargo
使用方式不因这个实验被移除。目标架构见
[`NESTRS_COMPILER_TOOLCHAIN_PLAN.md`](NESTRS_COMPILER_TOOLCHAIN_PLAN.md)。

## 1. 本阶段的使用形式

实验项目中的业务服务仍使用普通属性声明和 Rust impl：

```rust
trait PaymentPort: Send + Sync {
    fn provider_name(&self) -> &'static str;
}

#[nestrs_macro::injectable]
struct PaymentClient;

impl PaymentPort for PaymentClient {
    fn provider_name(&self) -> &'static str {
        "local"
    }
}

#[nestrs_macro::injectable]
struct Checkout {
    #[inject]
    payment: dyn PaymentPort,
}
```

用户不需要在这个 impl 上写 `#[bind]`。两阶段编译生成绑定后，concrete 和 trait
查询继续通过现有查询宏进行；它们访问同一 concrete provider 管理的实例。
`TraitBinding` 仍是投影关系，不承担实例生产，也不覆盖服务声明的 lifetime、key
或 primary。普通 Cargo 本身尚不会自动启动这条生成链路。

## 2. 语义发现入口

core 的隐藏编译器 ABI 在
[`registration/compiler.rs`](../nestrs-core/src/registration/compiler.rs)，宏通过
`nestrs_core::__private` 引用它。三个 marker 在运行时没有注册或构造行为：

| Marker | 编译器读取的事实 |
| --- | --- |
| `compiler_provider::<T>(CompilerKey)` | class/factory provider 的真实类型及声明 key |
| `compiler_request::<T>()` | 依赖字段、factory 参数或查询宏请求的实际类型 |
| `compiler_binding::<Concrete, Interface>()` | 已有显式绑定或本工具生成的绑定 pair |

`CompilerKey` 包含 `Default`、`Named(&'static str)`、`Indexed(usize)`。injectable
的普通描述、泛型蓝图和 factory 描述共用 `EmitCompilerKey`，直接生成原声明的
静态 literal；不创建额外运行时 `String`。

key 对语义发现同样重要。例如，某个显式 factory 已提供默认 key 的
`Repository<User>` 时，图编译器不会调用同类型同 key 的泛型蓝图。工具因此也不能
从这个未使用蓝图中引入新的 dyn 请求。若 factory 使用另一个 key，默认 key 的
蓝图仍然有效。三个 key 变体都保留精确匹配，命名 key 不会自动成为默认候选。

[`autobind_semantic.rs`](../cargo-nestrs/src/compiler/autobind_semantic.rs) 在
`after_analysis` 中读取类型检查后的 HIR、`TyCtxt` 和 marker 的真实 `DefId`。
别名和关联类型先按编译器规则归一化。候选发现不依赖业务类型名的字符串扫描；
源码文本仅用于最终覆盖层和可审阅的输出。

## 3. 有限候选与泛型工作队列

分析以显式 provider、已闭合的请求和已有绑定为起点，用队列展开当前需要的描述。
开放泛型描述本身不会把其中看似已经闭合的字段变成全局请求，只有相应蓝图被物化
时，字段与 factory 参数的依赖才进入队列。

对已知闭合类型，工具先让 rustc 判断它是否实现 `ProviderDefinition`，再解析并
实例化对应方法中的 marker，继续遍历新出现的依赖。显式 provider 的精确类型和
key 优先于蓝图。已展开类型被记录，重复根和重复依赖不会无限展开。

另一个有限来源是源码中明确闭合的 impl，例如：

```rust
impl Port for Repository<User> { /* ... */ }
```

当接口确实被请求、rustc 确认该类型可投影到目标接口，且它具有可用蓝图时，这个
明确闭合类型可进入候选集。因此部分只查询 trait 的泛型实现也能被发现。

对于 `impl<T> Port for Repository<T>`，本阶段仍需要已知的闭合类型来源。
工具不会枚举任意 `T`，也没有实现从任意开放 impl 自动反解全部泛型参数的算法。

最终的 concrete/interface pair 由 rustc 的 `Unsize` obligation 决定。
目标是完整的 dyn 类型，包括关联类型约束、auto traits 和内部 `for<'a>` 绑定。
内部已绑定的生命周期不被误判为开放类型。只有编译器证明约束成立
的 pair 才会被生成；不满足泛型 where 约束的实现不会因名称形似而成为候选。
没有被 DI 请求的普通 trait 实现不因存在于 crate 中就被全部导出。

## 4. 生成代码、安全和作用域

[`autobind_codegen.rs`](../cargo-nestrs/src/compiler/autobind_codegen.rs) 为每个选中的
pair 生成普通、安全 Rust：

1. 用实际 dyn 类型定义局部类型别名。
2. 生成 `&Concrete -> &Interface` coercion，交由最终 rustc 类型检查。
3. 调用现有 required/optional binding preparer。
4. 向 core 宿主的 `REFLECTED_BINDINGS` 写入 `TraitBinding` 描述回调。

生成代码不伪造 vtable、不创建额外实例、不延长外部借用生命周期。实例保活仍由
原有 `Injection`、输入 frame 和强 lease 机制负责。factory-only 类型继续使用现有
可选蓝图探测，不被强加 `ProviderDefinition` 约束。

插入位置由 rustc 的模块信息和可见性检查确定。工具会考虑 concrete 声明所在
模块、描述所在模块以及相关 impl 所在模块，选择能够合法命名 concrete 与接口的
真实源码模块。接口比 concrete 更私有时，也必须选择同时合法的作用域。
最终编译仍完整执行 Rust 的隐私和类型检查。

当前支持范围包括普通源码模块中的宏生成 impl，以及名字能在该模块合法表达的
宏生成类型。它不等价于支持任意宏卫生性场景。若需要插入的模块本身来自宏展开、
类型仅存在于函数块内、没有可用物理源码位置或没有可表达的共同作用域，工具明确
报错；不会通过改成 `pub` 或绕过隐私检查继续构建。

## 5. 两次完整编译和源码覆盖层

[`nestrs-driver.rs`](../cargo-nestrs/src/bin/nestrs-driver.rs) 的前身作为实验用
`RUSTC_WORKSPACE_WRAPPER` 接收 Cargo 的编译参数。同一 target 经历两次明确分开的
编译器调用：

1. 第一轮读取真实源码，完成宏展开、cfg 选择和类型分析，收集语义并在 codegen 前
   停止。同时记录读取到的文本源码快照。
2. 根据第一轮结果生成绑定和插入位置。所有文件必须仍与分析时的快照一致。
3. 第二轮使用 `FileLoader` 覆盖层，在原始源码路径上提供加有绑定的虚拟文本，
   从解析和宏展开开始重新编译。原有模块相对路径保持有效。
4. 第二轮再次进行语义分析，确认没有剩余待生成 pair，provider/request/已有绑定
   的计数与预期相符后继续 codegen 和链接。

这个流程没有在已检查完成的 HIR/MIR 上追加条目。原始 `.rs`、Cargo manifest 和
锁文件不会被生成器改写。为了便于排查，target 中保存单独的格式化绑定文件和
实际覆盖文本。

覆盖层按原文件字节偏移工作，检查 UTF-8 边界和源内容一致性；生成插入尽量保留
原业务源码的行号。第二轮还核对没有插入绑定的其他文本源码文件，发现源码改变或
出现第一轮未读取的新源码时拒绝继续。此实验没有因此获得对任意非确定性过程宏、
环境变化、二进制输入或全部构建脚本副作用的事务隔离保证。

## 6. 显式绑定和图规则

自动发现按真实类型 pair 去重。已存在的 `compiler_binding::<C, I>()` 会抑制
同一 pair 的自动生成，以允许迁移期保留手写 `#[bind]`。

这不删除用户原有的注册：两个显式绑定回调仍保留在 linkme 切片中，并由现有图
编译器报告重复 binding。自动去重不能掩盖用户声明错误。

多个 concrete 实现同一接口时，工具保留所有合法候选。key、primary、optional、
生命周期冲突和环检测仍由现有图编译器处理，自动绑定不自行挑选一个实现来隐藏
歧义。图结构错误仍在公开 `ServiceProvider::build` 入口执行服务构造前 panic。
本阶段没有把运行期全图校验变成已经实现的 Cargo 构建期校验。

## 7. 验证入口与证据

在仓库根目录运行：

```sh
python3 tools/compiler-probe/verify_autobind.py
```

前置条件和第一轮探针相同：使用
[`toolchain.json`](../tools/compiler-probe/toolchain.json) 中匹配 release、完整 commit
和 host 的 rustc，并安装匹配的 `rustc-dev`。验证器不会安装组件或切换默认工具链。
`RUSTC_BOOTSTRAP` 仅用于编译独立 driver crate 的子进程；fixture 应用编译不启用它。

验证器构建 driver 及覆盖层边界测试，再通过 wrapper 构建独立 fixture 项目，运行
各个 binary 内的真实 DI 断言。`cfg_selected` 分别使用默认和 alternate feature。
每项断言同时核对发现记录中的生成 binding 数量，避免把普通源码能编译误报为
自动绑定成功。验证器最后对比业务源码、manifest 和 lockfile 的哈希。

产物位于 `target/nestrs-autobind/`：

| 产物 | 用途 |
| --- | --- |
| `report.json` | 当前这次验证是否完成、各 case 结果、编译器身份和源码未改写检查 |
| `generated/<phase>/<crate-and-arguments-id>/analysis.json` | 候选/请求数量、生成和显式 binding 计数，以及每个生成 pair 的来源 |
| 同目录 `compilation.json` | 对应两轮编译是否完整成功 |
| 同目录 `*-bindings.rs`、`*-overlay.rs`、`sources.txt` | 可审阅的绑定代码、覆盖文本和路径对应关系 |
| `cargo-*.stderr.txt`、`run-*.stdout.txt`、`run-*.stderr.txt` | 构建和运行断言的原始诊断 |

fixture 明细见
[`fixtures/auto-binding/README.md`](../tools/compiler-probe/fixtures/auto-binding/README.md)。
每次运行结论以验证器成功退出及其完整 `report.json` 为准；下列结果只对应明确列出
的工具链和本阶段用例，不表示后续验收矩阵已全部完成。

2026-09-28 已在固定的 Rust 1.98.0（commit
`88d9e12ae178fab0fb5cc050a94da85685d449ea`，`x86_64-unknown-linux-gnu`）完成一次
完整运行：**14 次集成运行和 6 个覆盖层/快照测试全部通过**，应用源文件哈希保持一致。
14 次运行包括 13 个默认 binary，以及 alternate feature 下的 `cfg_selected`。
主业务用例自动生成 7 条绑定，使用非零大小类型验证 concrete/trait 同实例；预期图错误
用例同时检查错误原因与构造计数，没有用任意 panic 代替正确诊断。

## 8. 后续边界

以下工作仍需独立落地和验收：

- **跨 crate 的发现和桥接。** 当前实验处理 fixture 的单个 crate。外部泛型蓝图
  没有本 crate 的描述体时明确报错；跨 crate 私有类型、依赖库元数据与发布协议尚未
  完成。将 crate 分别送入 wrapper 不会自动解决这些问题。
- **生产 Cargo CLI。** 尚无 `cargo nestrs build/run/check/test`、分发/升级流程、
  稳定错误界面或跨平台验收。当前 wrapper 保留 Cargo 参数并用于真实 fixture，
  不代表任意 target、features 和 build.rs 组合都已支持。
- **正式语义序列化和缓存。** 当前 JSON 面向观察和回归；打印出来的类型文本不是
  跨编译、跨版本的稳定类型身份。验证器清理自己的 fixture 包并关闭增量，未承诺
  生产缓存正确性。源码、依赖、编译参数、工具版本和工具链身份的缓存协议待实现。
- **HTML 图迁移。** 现有 HTML 页面、core 配置与导出行为继续保留；CLI 尚未接管
  页面资源或真实应用链接单元的无构造图提取。
- **编辑器与诊断体验。** rust-analyzer 接入、生成位置映射、跨模块精确诊断和
  调试器体验仍需验证。保持普通源文件不被改写只是这一目标的一部分。

本阶段不改变查询宏、冻结图、Tokio 调度、实例 lease、取消和关闭契约，不新增
runtime crate，也不提前实现 bootstrap。
