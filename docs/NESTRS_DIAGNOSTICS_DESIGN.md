# DI 编译诊断指南

本文说明当前 `cargo nestrs check/build` 的源码优先诊断：先定位用户源码、解释依赖
关系为何不成立，再给修复方向；完整类型、内部槽位和生成来源保留在最后的 `cause`。
所有可手动复现的案例统一见 [30 个错误示例](../example/di-errors/README.md)。

## 阅读一条诊断

```text
error: [NESTRS-DIxxx] <用户声明及直接失败原因>
  --> <用户源码路径>:<行>:<列>
   |
行 | <用户实际写下的代码>
   | <主位置高亮与相关声明标签>
   |
   = note: <需要时解释依赖规则或展示依赖路径>
   = help: <与当前错误有关的修复方向>
   = note: cause: <内部错误种类及原始内部说明>
```

标题、位置、标签、note 和 help 均由原生 rustc 诊断承载；终端与 Cargo JSON 共用同一份
信息。`cause` 是最后一条 note 的前缀。`NESTRS-DI001` 等标识在消息正文中，**不是**
Cargo JSON `code` 字段里的标准 Rust 错误码，不能通过 `rustc --explain` 查询。

| 信息 | 当前语义 |
| --- | --- |
| 标题与主位置 | 指出实际字段、constructor/factory 参数、查询或声明，不要求先理解生成符号 |
| 关联位置 | 指出冲突候选、另一创建声明、依赖链和已知泛型闭合来源 |
| note | 解释 key、唯一候选、环和生命周期等规则 |
| help | 给出业务可选择的修复方向，不自动代替用户选择 primary 或修改 lifetime |
| cause | 保留内部种类、完整类型、精确 key、槽位和传播路径，用于进一步排查 |

默认 key 在无关标题中省略，在 key 错配时明确展示；字符串和整数 key 不同。
cause 的 `[key=None]` 表示默认 key，不表示 optional 输入交付了 None。
显示名称来自真实 rustc 类型；同名冲突补充限定路径。类型别名可能显示为归一化后的
实际类型，名称缩短与别名不会改变真实类型身份。

## 代表输出

以下保留两个代表摘录，其余项目的源码、预期代码及修复方向集中在示例索引。
运行目录会改变路径前缀，源码编辑会改变行号，布局和关联位置合并由 rustc 决定；
本机完整输出以复现命令为准。

### 必选依赖缺失

对应 [01-missing-concrete](../example/di-errors/01-missing-concrete/src/main.rs)。

```text
error: [NESTRS-DI001] 无法注入 `OrderService.database`：没有匹配的 `Database` 服务声明
  --> 01-missing-concrete/src/main.rs:11:15
   |
11 |     database: Database,
   |               ^^^^^^^^ 这个必选依赖没有可用的服务声明
   |
   = help: 为 `Database` 添加 `#[injectable]`，或声明返回该类型的 `#[factory]`，并确保 key 匹配。
   = note: cause: MissingDependency
           缺少必选依赖：nestrs_error_missing_concrete::OrderService [key=None]（01-missing-concrete/src/main.rs:7:1）.database（槽位 0）请求 nestrs_error_missing_concrete::Database [key=None]
```

主位置直接落在字段的 `Database` 类型上。修复提示说明怎样提供缺少的服务，
不会默认建议把业务必需的字段改成 optional。

### 生命周期沿依赖链传播

对应 [18-lazy-scoped](../example/di-errors/18-lazy-scoped/src/main.rs)。

```text
error: [NESTRS-DI006] `Application` 是 Singleton，但依赖链包含 Scoped 服务 `Session`
  --> 18-lazy-scoped/src/main.rs:19:19
   |
 5 | #[injectable(lifetime = Scoped)]
   |                         ------ 此服务的生命周期是 Scoped
...
13 |     session: Session,
   |              ------- `Intermediate.session` 依赖 Session
...
19 |     intermediate: Intermediate,
   |                   ^^^^^^^^^^^^ `Application.intermediate` 依赖 Intermediate
   |
   = note: 依赖路径：
           Application.intermediate → Intermediate [Transient]
           Intermediate.session [lazy] → Session [Scoped]
   = note: `Application` 未指定 lifetime，默认是 Singleton。
   = note: Singleton 在 root 中构造；Transient、optional 和 lazy 都不会消除下游的 Scoped 要求。
   = help: 若 `Application` 属于请求，将其改为 Scoped；否则拆开应用级与请求级依赖。从 scope 查询 Singleton 不能解决此问题。
   = note: cause: ScopeRequired
           Singleton 的激活依赖需要 Scope：nestrs_error_lazy_scoped::Application [key=None]（18-lazy-scoped/src/main.rs:16:1）.intermediate（槽位 0）请求 nestrs_error_lazy_scoped::Intermediate [key=None] -> nestrs_error_lazy_scoped::Intermediate [key=None]（18-lazy-scoped/src/main.rs:8:1）.session（槽位 0）请求 nestrs_error_lazy_scoped::Session [key=None] [lazy] -> nestrs_error_lazy_scoped::Session [key=None]（18-lazy-scoped/src/main.rs:5:1）
```

主位置在 Singleton 引入问题的输入，关联位置解释 Scoped 要求从哪里传播。
未指定 lifetime 时补充默认 Singleton 规则；从 scope 查询 Singleton 也不会
改变它由 root 持有的事实。这是静态依赖约束，不表示程序已经执行或资源已经释放。

constructor 缺失会明确写 `Application::new` 的参数 `_database`，factory 缺失使用
实际函数名及参数名；两者都定位原参数类型，不用生成 frame 名称作业务标题。
同类型重复 provider 会关联两处创建声明；闭合泛型缺失同时保留模板字段与可恢复的
闭合使用位置。多个错误的完整示例见
[30-multiple-errors](../example/di-errors/30-multiple-errors/src/main.rs)。

## 错误分类与修复方向

这些标识属于工具诊断；精确排版不是稳定的公共协议。

| 标识 | 失败原因与主位置 | 修复方向 / 内部原因 |
| --- | --- | --- |
| `NESTRS-DI001` | 必选依赖无匹配服务；定位字段或构造/factory 参数类型 | 声明对应 provider，确保类型和 key 匹配；`MissingDependency` |
| `NESTRS-DI002` | 已有相关声明，但 key 不匹配；优先定位请求 key，默认 key 使用输入类型 | 统一 key 的类型和值，或为所需 key 新增服务；`MissingDependency` 的细分 |
| `NESTRS-DI003` | 接口有多个候选，无唯一选择；定位消费类型/查询，无消费时关联候选声明 | 选择唯一 primary 或按 key 区分；`AmbiguousTrait` |
| `NESTRS-DI004` | 同 key 下有多个 primary | 保留唯一 primary 或分开 key；`AmbiguousTrait` 的细分 |
| `NESTRS-DI005` | 完整服务依赖图成环；定位代表闭环中的真实输入边 | 移除一条依赖或提取共同服务；optional/lazy 不消除环；`Cycle` |
| `NESTRS-DI006` | Singleton 依赖链含 Scoped；定位 Singleton 引入该路径的输入 | 按实际所有权调整 lifetime 或拆分依赖；Transient/optional/lazy 不消除 Scoped 要求；`ScopeRequired` |
| `NESTRS-DI007` | 相同真实类型和 key 有多个创建声明 | 保留一个或区分 key；primary 不覆盖具体类型重复；`DuplicateProvider` |
| `NESTRS-DI008` | 泛型展开超过分析上限；定位引入复杂类型/调用的位置，已知时关联闭合起点 | 限制类型集合或改用运行期数据结构；`TypeExpansionLimit` |
| `NESTRS-DI009` | 相同 concrete/trait pair 显式绑定多次；关联冲突绑定 | 移除重复显式 binding；`DuplicateBinding` |
| `NESTRS-DI010` | 显式绑定没有 concrete provider | 声明创建方，投影自身不创建服务；`OrphanBinding` |
| `NESTRS-TOOL001` | 统一诊断入口报告的内部描述或执行协议不一致；无可信来源时没有主位置 | 保留诊断与工具版本排查，不要求业务修改内部槽位；`InvalidMetadata` / 协议异常 |

DI009/DI010 覆盖隐藏显式 binding 协议；正常业务使用普通 impl 自动绑定。
optional 仅允许目标缺席；服务级 lazy 只影响 owner 创建时的自主初始化。它们都不会
跳过已有关系的歧义、环或生命周期检查。查询时的连接失败或构造 panic 返回
`ResolveError`；创建期间的初始化失败分别进入 `BuildError::Initialization` 或
`ScopeBuildError`，并在返回前关闭未交付 owner，保留可能发生的清理错误。关闭阶段
的 cleanup 失败通过 `DisposeError` 报告。这些运行期结果不属于编译期结构诊断。
Rust 原生类型、trait 与借用错误继续由 rustc 报告，不强制改成 Nestrs 编号。

也不是每项工具错误都有上述编号。例如
[options ABI 握手](../cargo-nestrs/src/compiler/di_plan/emission.rs) 在缺少
`plan_set_options_v3` 或完整签名不符时，直接通过 rustc fatal 报告工具与 core
版本不匹配。验证器应核对对应场景的退出状态和诊断，不能仅以是否出现
`NESTRS-TOOL001` 判断工具链是否兼容。

## 位置如何从源码到达诊断

| 层次 | 实现与职责 |
| --- | --- |
| 原始声明 | [codegen/source.rs](../cargo-nestrs/src/codegen/source.rs) 与 [injection/render.rs](../cargo-nestrs/src/codegen/injection/render.rs) 保留原 token Span、字段/参数名称、策略位置及类型末 token，生成私有来源 marker |
| 认证与关联 | [reflection.rs](../cargo-nestrs/src/compiler/reflection.rs) 与 [di_plan/mod.rs](../cargo-nestrs/src/compiler/di_plan/mod.rs) 认证 marker 的 DefId/签名/来源，将它与真实类型及当前图实体关联；constructor 角色来自明确记录，不靠名称差异猜测 |
| 纯图模型 | [di_plan.rs](../cargo-nestrs/src/di_plan.rs) 的 `DiagnosticEvidence` 保存消费者、输入槽位、候选及真实依赖边，保留原内部 message；不依赖 rustc，不负责排版 |
| 用户信息 | [di_plan/diagnostics.rs](../cargo-nestrs/src/compiler/di_plan/diagnostics.rs) 组织标题、主位置、关联声明、原因和 help |
| 统一输出 | [compiler/diagnostics.rs](../cargo-nestrs/src/compiler/diagnostics.rs) 排序并发出原生 rustc 诊断，最后追加 cause，以受控编译失败退出 |

多 token 类型的 `Span::join` 在过程宏环境中不总能得到完整范围。生成端额外保存
`InputTypeEnd`/`ProviderTypeEnd`；driver 仅在文件、上下文和先后范围兼容时合并，
不兼容则保留可信位置。来源数据不替代类型、借用、隐私或宏卫生检查，也不写入 core
运行期计划；core 不为诊断重新分析声明。

宏实参保留原 token 时定位该 token；否则降级到可信调用位置。跨 crate 源码展示
使用 metadata 记录的 `src_hash` 校验外部文件；文件缺失或已变化时仍保留原编译位置，
同时说明片段不可加载。虚拟生成文件只能回退到可信调用来源；无法恢复时明确说明
没有精确用户位置，不把生成行号伪装成手写源码。实现不通过字段名搜索或旧位置字符串
反推精确 Span，也不为展示放宽两阶段编译输入的快照校验。

## 多错误、长路径与复杂度上限

- 独立错误按用户源码路径、行列、标识及标题排序，不依赖哈希迭代顺序。
- 相同接口/key 的候选冲突按根因报告，关联其他消费位置；已识别的冲突不再派生缺失
  错误。不同闭合泛型仍按真实类型身份区分。
- 每个有环的强连通分量报告一条代表闭环。依赖路径超过 8 条时展示前 4 条和后 4 条，
  明确省略数量；候选标签最多展示 6 个，更多候选在 note 中计数。
- cause 超过 **1,800 个 Unicode 字符**时，输出仅保留前 **600 个字符**，完整内容写入
  当前编译输出位置的 `nestrs-cause-<hash>.txt` 详情工件并给路径。写入失败会明确说明
  截断和失败原因，不假装详情仍可读取。cause 不是公开逐字段 schema。
- DI008 区分单类型复杂度与展开实例预算：单类型树上限为
  `max(8 × recursion_limit, 1024)`，闭合类型集合和查询展开预算为 **100,000**。
  walker 对复用的 interned 类型去重；这不是普通 DI 链深度限制，也不能据一次超限
  断言程序必然无限递归。已编译的 `if false` 分支仍参与分析，cfg 排除项不参与。

查询分析先筛选可能关联 DI 的调用，再按闭合后的真实实现继续展开。另一个 trait
实现含查询，不应使当前无查询的普通实现仅因携带宽类型而超限。另一方面，同一真实
helper 沿调用链持续扩大类型参数时仍可能被保守拒绝，即使业务认为最终会终止；
DI008 不等于 rustc 已证明程序无限递归。具体分析机制与边界见
[查询根与预算](NESTRS_RUSTC_EXTENSION_GUIDE.md#query-budgets)，历次误拒及修复见
[修复记录](NESTRS_FIXES.md)。

## 复现与维护验证

从仓库根目录使用匹配的工具链：

```bash
cargo nestrs check --manifest-path example/di-errors/01-missing-concrete/Cargo.toml
cargo nestrs check --manifest-path example/di-errors/01-missing-concrete/Cargo.toml --message-format=json
python3 example/di-errors/verify.py
python3 example/di-errors/verify.py --operation build
```

单个 Cargo 命令应失败；verifier 只有在每例均因预期诊断失败时才成功。
Cargo JSON 中读取 `compiler-message` 事件的 `message`、`spans`、`children` 与 `rendered`，
无需从终端文本猜测 IDE 跳转位置。各平台排版差异和完整编辑器 UI 交互应分别验证。

| 回归入口 | 核对内容 |
| --- | --- |
| [di_plan.rs](../cargo-nestrs/tests/di_plan.rs) | 结构化输入、候选、循环/Scope 路径、同根因抑制和独立错误 |
| [codegen/source.rs](../cargo-nestrs/src/codegen/source.rs) 的单元测试 | 原 token 与 marker 位置、构造角色、复合类型末端和分组 |
| [diagnostics.rs](../cargo-nestrs/tests/diagnostics.rs) | 前 29 个单错误教学项目的真实 JSON；宏、别名、跨 crate、查询、独立错误、有限复杂类型、无消费点歧义和完整长 cause |
| [aot_graph.rs](../cargo-nestrs/tests/aot_graph.rs) | 最终 check/build 拒绝非法图，不执行应用或构造副作用 |
| [example verifier](../example/di-errors/verify.py) | 全部 30 个项目，包括一次编译五错误的顺序与标题 |

运行 harness 的 feature、环境和工具准备见[工具链说明](NESTRS_CARGO_TOOLCHAIN.md)，
fixture 分工见[统一索引](../cargo-nestrs/tests/fixtures/README.md)。
