# 仓库维护规范

## Git Commit

采用 Conventional Commits，格式为：

```text
<type>(<scope>): <中文描述>
```

| type | 用途 |
| --- | --- |
| feat | 新能力或新的用户可见行为 |
| fix | 已有功能的错误修复 |
| refactor | 调整结构，保持对外功能和行为 |
| perf | 以性能改善为主要目的 |
| test | 单独增加、修改或重构测试 |
| docs | 纯文档修改 |
| build | Cargo、依赖、构建脚本或 Rust 工具链 |
| ci | CI/CD 配置和工作流 |
| chore | 无法合理归入上述类别的维护工作，不作为默认 type |
| revert | 回滚已有提交 |

Scope 使用实际 Cargo package name，如 `nestrs-core`、`cargo-nestrs`；不得使用 `di`
之类缩写或 `graph` 等内部 module。整个 workspace 的配置或同一跨 package 架构变更
使用特殊 scope `framework`。多个 package 的不相关变化应拆分；没有合理 scope
时可以省略。不要添加没有必要的新 type。

Description 必须使用中文，准确描述改变了什么，使用明确动词，建议约 72 字符以内，
末尾不加句号。Cargo package、Rust 类型、API 与技术术语可保留英文；默认不使用
Emoji。避免“修改了一些代码”等空泛描述和只罗列底层数据结构的描述。

```text
feat(nestrs-core): 支持 Scoped 生命周期
fix(cargo-nestrs): 修复跨 crate 查询根收集
refactor(framework): 简化构造输入交付
build(framework): 更新固定 Rust 工具链
```

一个提交对应一个独立逻辑变化。功能可以包含配套测试和文档，不按文件机械拆分，
也不把无关工作塞进同一提交。混合代码和文档时按主要变化选择 type。
复杂变化可在空行后的 body 说明原因、设计和兼容性。破坏 API/行为兼容性时使用
`<type>(<scope>)!:`，或在 body 中标记 `BREAKING CHANGE:`；必须明确说明用户影响，
必要时补充迁移方式。

AI 创建提交前必须检查 `git status`、`git diff`、`git diff --staged`，根据实际变更
选择 type、scope 和 description。不得未经检查就 `git add .` / `git add -A`，
不得将用户原有无关修改纳入当前提交。没有提交或推送请求时保持工作区更改。
除非维护者明确要求，不添加 AI 署名、`Co-authored-by` 或 Emoji。

# 架构共识

本部分记录已与维护者确认的 Nestrs 架构。应用只依赖 core 与业务库，服务声明、
编译、文档和编辑器所需工具统一由 `cargo nestrs` 管理。私有过程宏桥接供 rustc、
rustdoc 和原版 rust-analyzer 共用，生成后端只有一份；应用不依赖公开宏 package。
AI 修改时必须遵守；后续改变这些边界仍须与维护者确认。

## 1. 总体定位与 package

* Nestrs 以 DI 为根基。用户面向 `nestrs-core` 与 `cargo-nestrs`；`nestrs-tool-bridge`
  是工具包内部的 `publish = false` 构建工件，不是应用依赖或独立公开 API。
* `nestrs-core` 是 DI 容器和所有运行期生态库的地基。
* `cargo-nestrs` 是构建工具：Cargo CLI、编译器适配、声明生成、IDE 项目模型和 HTML 图。
  它不是 runtime crate，应用运行时不链接工具实现。
* `cargo-nestrs/internal/bridge` 是标准 proc-macro 薄桥接，委托
  `cargo-nestrs/src/codegen`。CLI 将其以 `nestrs` extern 提供给编译器与编辑器；
  不复制生成逻辑，不恢复公开 `nestrs-macro` 或独立 `nestrs-codegen` package。
* `nestrs-bootstrap` 仍为未来的顶层引导库，负责 Application、配置和生态组合，
  导出 `NestrsFactory`；本阶段不提前实现。
* `example/` 只作多项目父目录，业务示例分别位于其子目录并各有 Cargo.toml、源码与说明。
  自动化负例放在 `cargo-nestrs/tests/fixtures/`；`example/di-errors/` 是明确标为预期编译
  失败的独立观察项目，由父目录统一说明，不混入正常业务示例的默认运行或图导出。

## 2. 分层与依赖方向

```text
工具内部：nestrs-tool-bridge → cargo-nestrs::codegen → 类型化服务声明
工具编排：cargo nestrs → rustc/rustdoc 的 extern 注入与 rust-analyzer 项目依赖
语义分析：nestrs-driver → 根据真实类型生成 binding、验证完整 DI 图并编译执行计划
运行期：  应用及未来 bootstrap/logger/config → nestrs-core
```

* core 不依赖 CLI、codegen、宏 crate 或 rustc 内部库。
* 所有运行期生态库单向依赖 core，未来 bootstrap 位于其上；生态库不得反向依赖 bootstrap。
* 编译期 codegen 位于 `cargo-nestrs/src/codegen`，只使用生成所需工具；生成的 typed adapter
  引用 core 实际所属私有模块。driver 按真实宏卫生来源与虚拟源码区间授权，普通业务
  源码不能访问；不导出 `__private` 或换名后的公开内部 ABI 模块。
* `Constructor::{Class, Factory}` 生产实例；`ProjectionAdapter` 只提供 concrete 到 trait
  的真实类型投影，不创建另一份实例。
* core 装配后用 `DependencyInput::{Absent, Immediate, Lazy}` 表达唯一输入动作，
  `CompiledDependency` 保留原始请求类型、optional、诊断与槽位；普通/延迟缺席分别
  交付准确的 None。`InputAdapter` 的内部 v2 协议为 service_type、kind: InputKind、project，
  `ProjectionAdapter` 仅含 trait_type、concrete_type、project。激活只展开 Immediate，
  完整图和关闭顺序仍包含 Lazy 目标，不把延迟边误删为无依赖。
* core 用 `ConstructionInput` 保存单槽已选执行数据，一次组成完整 `ConstructionInputs`；
  生成 adapter 的 typed take 直接经 ServiceProjector/ProjectionTarget::project 写入栈槽，
  与 trait 根查询和 lazy 共用真实 coercion、准确类型及同一实例 lease 检查。错误读取
  不消费槽位，None 和 lazy 也检查准确类型及形态；不恢复逐参数 preparer、PreparedInput
  装箱或可变准备 buffer。全部 typed 参数读取并 ensure_all_consumed 后才执行用户
  constructor/factory/Default/value；字段表达式保留原求值顺序、类型上下文和宏卫生。
  Class/Factory 保持分工，FactoryLeaseFrame 从原输入派生真实保活 lease 并借出
  FactoryInputs；不伪造跨 await 借用，不重写单一 Coordinator 状态机。
* `nestrs-reflect` 是工具链按当前项目与编译配置生成的逻辑产物，不是手写公共反射
  package。局部声明 marker、CompilerKey 与泛型 ProviderDefinition 留在生成代码；
  core 生产代码不定义 DependencyRequest、Delivery、ProviderSource 或候选注册模型。
  core 私有 activation::adapter 只保留执行能力，最终入口为 __nestrs_reflect_v2。
  driver 在引用 core 的最终 check/build 中核对 plan_set_options_v2 及完整签名，空图
  也拒绝旧 core；工具与 core 同步重编译，缓存按 driver/bridge 指纹隔离。审阅 JSON
  的格式 version 仍为 1，独立于执行 ABI 版本。每个最终入口的 metadata 旁保存
  版本化 *.nestrs-reflect.json，来自同一已选执行计划，仅用于审阅，不是运行时输入或
  稳定公开反射 API；HTML 的 *.nestrs-plan.json 是另一种
  同源展示产物，字段与编号约定不得混用。

## 3. 编译器执行计划与查询根

* 不依赖 linkme、inventory、链接段扫描或全局构造器。driver 在每个 binary/test
  入口汇总本 crate 与依赖 metadata 中的类型化声明和查询摘要，生成唯一版本化计划入口。
  rlib 保留回调 MIR 与原生可达性分析要求的目标代码，不安装可变全局注册表、
  不贡献重复的入口符号；私有 static/TLS/inline/generic 依赖必须保留，Rust 源码
  可见性不提升。
* reflect 入口 MIR 只调用已验证签名、类型身份与生成来源的执行适配回调和 core 内部写入函数，
  不执行用户 constructor/factory/Default/value/cleanup。业务 typed adapter 仍经过
  标准类型、借用与 trait 检查；不通过任意 MIR 改写伪造投影或放宽业务可见性。
* 编译器识别普通查询方法，按真实类型和跨 crate 查询摘要恢复有限闭合根，支持泛型
  辅助函数与闭合 impl 的 Self；trait/factory-only 类型不强加 ProviderDefinition 约束。
  标准 trait 方法与重载运算符保留真实关联方法和泛型实参，再由 rustc 求解业务实现，
  包括 Iterator::next、Add::add 和 +；不能只遍历业务 crate 自己定义的 trait 调用。
  隐式 Deref/DerefMut 保留类型检查的真实调整与方法身份；Drop 按优化前摘要和闭合
  实例的真实析构胶水继续展开，不靠容器名称手写析构规则，也不执行业务析构。
  关联常量及内联 const 中的查询函数指针保留真实常量身份和泛型实参；跨 crate 使用原生
  required_consts 与常量 CTFE MIR 摘要，闭合后由 rustc 选择 trait impl/default。
  不把常量伪装成 FnDef，不为查询分析求值常量或读取已求值函数地址。
  已编译但未执行的分支，包括 if false，仍贡献需求；cfg 排除代码不贡献需求。
  动态 key 求值一次，只选择冻结路由，不扩展图；不枚举无限泛型组合。
  单类型表达式复杂度上限为 max(8 × recursion_limit, 1024)，闭合类型集合和查询
  展开有 100,000 预算；这是类型族增长防护，不按普通 DI 链深度计数。
* library 编译贡献声明、投影能力和查询摘要，不要求应用图在 library 中完整。
  最终 binary/test 在 check/build 时完成候选选择、有限展开、完整图验证与计划生成；
  结构错误是编译错误，即使用户程序没有调用 build 也必须拒绝非法注册。
* core 使用入口共享的 OnceLock 装配不可变计划，各次 build 创建独立 runtime、缓存
  和实例。首次装配仍调用目标端执行适配回调取得真实 TypeId 与 typed adapter 地址；不再
  选择候选、展开泛型或分析拓扑，也不调用用户构造。不能宣称零回调或零分配。
  core 不保留 GraphCompiler 或旧注册测试模型；执行测试直接提供冻结计划，
  只读快照位于 tests/support 并通过 cfg(test) 引入。图语义由工具侧生产模型与真实
  driver 契约覆盖。未经过工具链的生产应用调用两个 build 入口返回
  BuildError::CompilerPlanUnavailable，不静默生成空计划。
* 编译器入口 ABI 是内部符号协议，不是业务可调用的 Rust API；driver 拒绝源码引用
  该入口，保留公开门面与真实 Injection 类型。

## 4. 工具私有桥接与标准宏展开

* 应用使用 `use nestrs::{injectable, constructor, factory, primary, lazy};` 和短属性，也支持
  `#[nestrs::injectable]` 等完整路径。字段/参数 helper 支持裸名及 `nestrs::` 路径。
* 字段与 factory 参数的 `inject` 语法一致，只接受裸标记 `#[inject]` 或单个字符串/
  整数字面量，如 `#[inject("mail")]`、`#[inject(123)]`。`#[inject(key = ...)]`
  为编译错误；provider 的 `#[injectable(key = ...)]`、`#[factory(key = ...)]` 配置不受影响。
* `nestrs` 是工具注入的 extern 名称，不是在 Cargo.toml 中配置的公开宏依赖。
  同名 Cargo 依赖会明确报冲突，不能静默覆盖工具桥接。
* 桥接只适配标准 proc_macro 输入输出；声明分析、字段/签名改写与注册生成复用
  `cargo-nestrs/src/codegen`。组合 primary 时保留属性末段名称；不能承诺任意重命名
  属性之间都可识别身份。crate/module 路径别名与单个宏重命名有对应回归。
* 标准 Rust 宏展开处理 cfg、外部模块、macro_rules 生成项和属性/derive 顺序；
  Injection<T> 字段和 factory frame 借用签名在类型检查前生成。
* `#[constructor]` 选择 injectable 的同步 inherent 关联构造函数，返回 Self 或
  Result<Self, E: Debug>；参数是唯一依赖来源，普通输入交付拥有 lease 的令牌，
  lazy 输入按值交付弱 owner 句柄，optional 缺席交付 None。字段保留业务写法，按成功返回字面量的整值参数来源改写，不按同名/同型猜测。
  不混用字段 inject/value/lazy，不创建第二个 provider，不补做 Default；无 constructor
  时保持自动字段模式。标准名称解析后、HIR 前按真实 impl self 身份选择生成候选，
  用 Res::Local(NodeId) 区分局部来源、宏卫生与遮蔽；cfg 排除字段不进入存储映射，
  仍存在的构造参数继续贡献依赖。宏阶段只处理签名和 adapter。保留原生类型/借用检查；
  复杂无法确认的流必须诊断，不能扫描源码或用可变宏全局表关联。
* 应用经 cargo nestrs check/build/run/test 获取桥接与自动绑定；普通 Cargo 不注入
  该环境。core 和工具自身可以用普通 Cargo 检查。应用级 Clippy 集成尚未交付。
* driver 不注册 `nestrs` 工具属性，不替换原生展开管线或复制 token server。
  编译器适配仍用于语义分析、图入口和 IDE 构建记录，升级时需维护并回归。

## 5. 自动绑定

* 普通 `impl Trait for Concrete` 按实际注入/查询需求自动参与绑定，业务代码不写 bind。
* 候选限于声明 provider、factory 成功类型和已知闭合泛型，不注册任意 impl，
  不猜测泛型实参或枚举无限类型集合。
* adapter 使用真实 Ty/DefId、归一化与 Unsize 求解检查投影，生成真实 typed coercion
  再由 rustc 检查，不伪造 vtable、不延长借用、不绕过业务类型可见性。
* 语义发现与最终生成使用两个完整编译阶段。FileLoader 只覆盖编译输入，原始源码
  不修改；生成产物保存在 target，最终编译再次检查缺失绑定。
* type/key 显式 provider 优先于蓝图；key 精确匹配；primary 只解决同 key 的 trait
  多候选；optional 不能隐藏歧义、环或生命周期错误。
* 上游注册、查询根和闭合蓝图通过编码 MIR 汇总。已知服务所属 crate 预生成合法的
  自动投影能力到编译器收集的自动 binding 描述清单，私有 concrete 不要求公开；
  不把潜在投影直接当成请求或显式注册。driver 在最终入口编译时按实际根/依赖需求
  迭代启用目录、物化必要闭合类型，再验证和冻结图；未请求接口不触发歧义或泛型物化。
* 自动 pair 在完整链接单元幂等，显式 pair 优先且重复显式 binding 仍报错。
  完整候选验证之后才裁剪未被输入或查询路由引用的投影，并同步重编号；不能提前
  裁剪来掩盖重复 binding、缺失 concrete provider 或候选歧义。
  类型身份使用真实 Ty/DefId，不按源码名字合并；不绕过隐私、不猜泛型实参。
  目录覆盖业务接口可证明的 Send/Sync 形状，额外 auto trait、未确定泛型、不可命名
  私有投影和宏生成位置仍须准确报告边界，见 `docs/NESTRS_MACROS.md` 与 `docs/NESTRS_RUSTC_EXTENSION_GUIDE.md`。
* `nestrs::bind` 只保留为文档隐藏的显式绑定 ABI 回归入口，不是推荐业务 API。
  显式 pair 不再自动重复生成，重复显式 binding 仍是编译期图错误。

## 6. DI 门面与静态图

* core 提供 get_required_service、get_service、get_required_keyed_service 和
  get_keyed_service 四个普通异步查询方法。旧查询宏及 src/query.rs 已移除，不恢复
  同名转发宏；查询根由工具链识别普通方法的真实调用，不提供单独 register!。
* build/build_with_options、create_scope、service_provider、warm_up 和消费 owner
  的 dispose_async 保留普通方法。引用绑定实际 root/scope owner 的借用期。
* 容器启动默认值由入口 package 的 Cargo.toml 顶层 `[nestrs-cli]` 设置：
  `initialization = "lazy" | "eager"`、`max-concurrent-activations = 正整数`。
  未设置字段仍为 Lazy / 32。工具编译时读取、校验并固化进当前 binary/test 入口，
  manifest 参与编译依赖跟踪；core 运行时不读取 TOML，不继承依赖或 workspace 默认配置。
  `build()` 使用项目默认值；`build_with_options` 完整显式覆盖；Options::default
  保持库的 Lazy / 32 基线。Cargo 的自定义顶层节警告不等于 Nestrs 未读取配置。
  这里覆盖的是全局默认；服务声明上的显式初始化策略仍优先。
* 服务结构体 / 工厂支持 `#[lazy]`、`#[lazy()]`、`#[lazy(true)]` 和 `#[lazy(false)]`。
  未标记继承全局默认；true 不选作自主预热根，false 在全局 Lazy 下也预热 Singleton。
  scope 创建仍不构造；显式 warm_up 默认预热 Scoped，但跳过 true，包含 false。
  Transient 不作为预热根。普通依赖仍可提前构造 lazy 目标，服务级标记不改字段包装。
  属性支持 injectable/factory 前后及 primary 组合；重复或非布尔参数拒绝。
  策略沿泛型蓝图与上游 metadata 进入最终计划，不改变完整图验证或取消/cleanup 语义。
* 最终入口编译时验证全部注册及已知闭合类型，结构错误使 cargo nestrs check/build
  失败。运行时只装配并执行已经选定的计划，之后不重新收集声明、展开泛型或变更图。
* 字段 `#[inject] #[lazy]` 生成 `LazyInjection<T>`，通过 `get().await` 首次获取。
  optional 字段为 `Option<LazyInjection<T>>`；key、trait 与闭合泛型仍使用冻结选择。
  延迟边参与缺失、歧义、环与 Scope 检查，但不作为消费者构造的就绪前提。
  同一字段合并并发访问并固定一次 occurrence（包括 Transient 的成功/失败），取消等待
  不重复提交。句柄弱持有 owner/命令通道，成功后强 lease 保活目标；不提供透明同步 Deref。
  字段和 factory 参数只接受裸 #[lazy]，不接受布尔参数或空括号。factory 参数默认
  注入，可单独标注 #[lazy]，也可组合 #[inject] 和字面量 key。延迟参数按值交付
  LazyInjection<T> / Option<LazyInjection<T>>，允许跨 await 并移入返回服务；普通
  参数仍是 frame 内借用。延迟参数共享字段的调度、验证、取消与关闭规则，构造 worker
  内首次获取尚未交付的目标仍报错，不增加构造重入或动态 resolve 通道。
* 图编译、激活任务展开、失败传播和实例释放使用非递归算法。
* Singleton 可以依赖 Transient，但整个激活闭包不得包含 Scoped；需要 Scoped 的
  Transient 只能从 scope 查询。factory 参数同样参与生命周期验证。
* Rust 类型检查、全图结构检查和外部资源初始化是三个不同层级。最终入口
  cargo nestrs check/build 包含前两层，但不能把它描述成外部资源已初始化成功。

## 7. Tokio、lease 与关闭

* 每 root 一个中央 Tokio 协调器，所有 scope/查询共享默认 32 个构造名额。
  依赖满足立即推进，没有整层屏障；worker 不递归 resolve。
* Lazy 默认；Eager 按服务策略预热 Singleton 及必要依赖，scope.warm_up 按策略预热 Scoped。
  Singleton 始终在 root 上下文构造；Transient 按每个消费槽位独立构造。
* Injection 和 ErasedServiceRef 持有强 lease；稳定实例地址、真实 factory frame
  和独立于 Tokio 的迭代 ReleaseDomain 维护内存安全。
* Singleton/Scoped 失败缓存至 owner 关闭，Transient 失败只属于该 occurrence。
  factory Result 要求 E: Debug；构造 panic 进入 ResolveError。
* 取消查询仅取消等待，接受的初始化继续。普通查询/预热的 QueryId 订阅及时退订，
  提前失败的消费者按输入槽位注销对子任务的反向订阅；Lazy 保留可接续的 watch 接收端。
  关闭先排空接受的任务，再按 owner 的消费者先于依赖约束逐个完成 cleanup/释放；含延迟边时用冻结 DAG 重排 journal，
  可同时清理的实例优先逆发布时间，无延迟边保留原逆发布顺序。每 owner 至多一个
  cleanup worker，root 等 scopes。
* 当前构造 worker 内首次等待未就绪延迟字段会明确报错，避免占据激活名额等待新任务。
  task-local 标记不传播到业务自行 spawn 的任务；factory 不得通过派生任务间接等待
  未就绪延迟字段，工具无法自动识别任意用户任务因果。此限制不是自动挂起/归还名额协议。
* dispose_async 等待取消不取消关闭；Drop 只发送幂等关闭请求，不新建 runtime 或
  block_on。Tokio 退出后只保证同步安全释放，不能保证异步 cleanup。
* 逃逸 token 延长必要内存存活但不阻塞逻辑关闭；cleanup panic 聚合成 DisposeError，
  其余 cleanup 继续。不添加自动超时或强制终止策略。

## 8. 依赖图 HTML

* HTML/CSS/JavaScript 和文件输出全部归 cargo-nestrs，输入来自编译器与执行计划
  同源的图 sidecar；core 生产代码不再生成图 JSON，不恢复 graph_output 等运行时选项。
* cargo nestrs graph 对选定 binary 执行 Cargo check 后读取 sidecar，不链接或执行
  诊断程序、不替换业务 main，也不执行 constructor/factory/Default/value/cleanup。
* 省略 --bin 时导出所选 package 的全部 binary；--workspace 导出 workspace 总览。
  default-run 不隐藏其他入口。每个入口独立编译、校验并隔离缓存；页面保留独立节点和
  依赖边，只标记共同 provider 声明的入口归属，不合并成跨入口容器。
* 项目报告保留成功、错误和 required-features 未启用的跳过状态；编译、校验或不支持
  的入口错误不阻断其他入口，写出报告后以非零退出。全部入口失败也生成诊断报告。
  显式 --bin 维持单图失败不覆盖旧输出；无 binary、入口选择无效、元数据查询或写入
  失败不覆盖旧输出。跳过不算错误，报告只有跳过时退出状态仍为 0，不代表验证成功。
  lib-only package 可列入项目清单，但不能虚构其独立服务图。
* 每个 package 独立解析特性；当前拒绝 --workspace --features，提示使用
  -p PACKAGE --features。默认 workspace members 选中多个 package 时也拒绝显式
  features，即使没有传 --workspace。--workspace 的 all-features/no-default-features 按各 package
  分别应用，不承诺复现一次 Cargo workspace 构建的 feature 合并。
* 图入口选择目前仍限 binary；lib/test/example 独立图目标尚未开放。编译期导出
  不要求 host 可执行或可直接定位 main；--target 交给 Cargo，目标库必须可用。
  driver 当前仍要求图目标直接依赖 nestrs-core；只有间接 core 依赖的普通应用虽可
  check/build/run，graph 入口仍会拒绝，不能把两者支持范围混为一谈。
  no_main/宏生成入口遵循真实编译检查，不通过运行目标程序补救；各 target 实际验证
  范围须单列，不能把 Linux 本机验证当作 Windows 或全部跨 target 验证。
* 页面展示 provider 声明、槽位与投影关系，不是实例状态；重复 Transient 输入仍独立构造。
* 默认写入 Cargo target 的 nestrs-di.html，输出错误由 CLI 报告，不污染容器构建契约。

## 9. 工具链、IDE 与验证

* nestrs-core 的测试实现统一放在 crate 根 tests/，内部单元测试位于 tests/unit/，
  编译器生成契约位于 tests/compiler/。src/ 只保留生产代码及必要的 cfg(test)/path
  模块挂载声明，不放测试文件或内联测试主体；不得为文件迁移增加公开内部 API。
  具体目录与执行方式见 nestrs-core/tests/README.md。

* pin 以 cargo-nestrs/toolchain.json 的 release、完整 commit 与支持的 host 为准。
  当前本机适配为 x86_64-unknown-linux-gnu 与 x86_64-pc-windows-msvc；driver、bridge
  和 sysroot 必须属于实际 host。不匹配时失败，不静默使用默认新编译器或退回源码扫描。
  Windows 使用本机 exe/dll 和 MSVC 工具，不要求 WSL，不宣称 Windows GNU、ARM64
  已受支持。graph 的 --target 使用静态编译产物，不要求运行目标程序；具体 target
  必须另有编译环境与回归证据。IDE 仍只支持当前 host，不能据 graph 推论跨 target IDE。
* compiler-driver feature 隔离 rustc_private；普通 core/工具单元测试无需该 feature。
  zyn 是共享生成后端的基础依赖；内部 bridge 不建立用户面向的宏 feature 契约。
* tools/build-toolchain.py 构建 CLI、driver 和匹配的 bridge。bootstrap 授权限于
  nestrs_driver 与私有 nestrs_tool_bridge 构建；bridge 仅为生成绑定取得定义点 span，
  仍委托单一 codegen 后端并保留业务 token 来源。普通 core/工具单元测试无需该授权，
  不改变全局工具链，也不向应用传播该变量。
* CLI 按完整编译器身份及 driver、bridge 的联合内容指纹隔离 target；Cargo 保留
  构建单元复用，rustc incremental 当前关闭，不宣称已有完整增量事务协议。
* 编译器和 rustdoc 同时获得 bridge 所在目录的 dependency 搜索路径，使没有直接
  core 依赖的下游也能解码上游 metadata。不能只给 producer 注入一个 extern 别名。
* doctest 先由 driver 以 cfg(doc) 检查真实源码和私有访问，再将 HIR 文档载体交给
  固定 sysroot 的真实 rustdoc。每段示例独立经过完整 driver，包含新声明、自动
  trait 绑定、闭合根和注册目录；不伪造业务类型、不静默跳过代码块。载体保留 crate
  测试属性，rustdoc 管理代码块执行语义；bootstrap 不传播进示例编译。
  仅间接依赖 core 的文档也走该流程，不能按直接 extern 是否存在而绕开最终计划汇总。
  真实源码按原 crate type 检查；纯文档载体按 lib 读取，真实 extern 工件保持不变。
  测试源与真实声明位置保留在 target 来源索引；复杂 impl self 类型不省略示例。
  当前只支持测试入口；相对 include/字节读取映射回原 Rust 文档或 Markdown 目录，
  含嵌套 include，读取继续参与两轮快照和私有路径审计，不在业务源码目录写临时文件。
* `cargo nestrs init` 面向手动组装后接入 Nestrs 的现有 Rust 项目，初始化或刷新开发
  环境；默认生成通用 rust-analyzer 项目与设置，只有 `--vscode` 才写 VS Code 配置。
  init 不创建项目、不添加 Cargo 依赖、不安装编辑器或工具链组件；其他 rust-analyzer
  LSP 客户端仍需自行加载生成配置，不能承诺所有支持 Rust 的编辑器都自动接入。
* 未来 `cargo nestrs create` 在 bootstrap 完成后负责创建项目，内部复用初始化能力，
  直接交付已初始化项目，用户无需再执行 init。create 当前属于规划，本阶段不实现。
* cargo nestrs init 依据 Cargo artifacts 与真实 rustc 单元生成 rust-project.json，
  保留依赖重命名/版本、cfg、edition、test、build.rs 环境、OUT_DIR 及过程宏工件。
  为实际编译单元直接 extern 包含 nestrs_core 的用户增加编辑器专用 `nestrs` 宏依赖，
  使用原版宏服务器。
  Windows 编辑器路径统一为普通盘符/UNC，与正常文件 URI 对应；不能只改测试 URI
  规避 verbatim 路径造成的 VFS 身份差异。设备或仅 verbatim 可表示的路径明确拒绝。
  内部 artifact/缓存身份继续归一化，首次准备与保存检查必须复用同一模型与缓存。
* cfg 由同一 rustc 按实际参数执行 --print cfg 获取；cfg.setTest=false 和
  cargo.cfgs=[] 阻止编辑器合成 test、debug_assertions 或 miri 条件。原版
  rust-analyzer 仍合并 host 默认 cfg，因此 IDE 拒绝 panic=abort、禁用默认 CPU
  特性等移除默认条件的配置并保留旧模型。debug/release 和增加 CPU 特性可表示；
  此限制不影响应用 check/build/run。
* init --vscode 合并 linkedProjects、check override 和宏服务器配置，保留无关设置、
  JSONC 注释与已有诊断偏好；不新增诊断屏蔽。保存时 init check 成功后刷新模型，
  失败保留上一份；未保存源码由 rust-analyzer 自身分析。详情见 docs/NESTRS_IDE.md。
  check.extraEnv 固定实际选定的 rustc、driver、bridge 路径并保留其他用户环境变量。
* tools/verify-ide.py 覆盖真实原版 LSP 的冷启动、字段/工厂类型、补全、定义跳转、
  未保存编辑、真实错误与恢复，以及 feature、宏生成项与 build.rs 产物；仍不能宣称
  所有编辑器 UI、重命名操作、属性组合或其他 host 都已验收。
* native_host、bridge_metadata、rustdoc 集成测试在 Linux/Windows 均启用；前者运行
  真实 CLI 检查/构建/运行、图副作用隔离与重复导出、IDE 项目生成和配置的保存检查，
  目录包含空格与中文。项目模型回归不等于完整 LSP 交互验收；不得把 Linux 结果当作
  Windows 实机结果，实际验收范围须分别记录。
* 平台能力以当前 `toolchain.json` 和各 host 的实际本地验证结果分别说明；回归命令
  由开发者手动执行。旧快照的 Windows/Linux 验收不能作为当前提交或完整编辑器 UI
  已通过的证据，也不能把单个平台的结果推广到另一个平台。
* DI UI fixture 的案例以 `cargo-nestrs/tests/fixtures/di/tests/ui.rs` 为准，经 CLI 私有 bridge 编译；
  正例检查成功，反例检查诊断 code/message 及数量，不批量覆盖 stderr 掩盖退化。
* tools/verify-graph.py 验证副作用隔离、不同 package/binary 缓存、项目部分失败和全部
  失败报告、feature 跳过、入口拒绝与文件导出。单图验证失败保留旧输出；项目报告保留
  已验证图与独立诊断。图导出必须读取编译器 sidecar，不能执行原业务入口。

## 10. 未来生态与命名

* 顶层引导库继续命名 nestrs-bootstrap，不使用暗示底层公共库的 nestrs-common。
* 未来 bootstrap 对接 logger/config/DI 等生态并聚合相应 features；运行期生态库
  建立在 core 之上，bootstrap 位于最上层。
* 本次工具链整合不增加 runtime crate，不提前实现 bootstrap、动态注册、运行期扩图
  或集合解析。

## 11. 后续范围与文档维护

* 分发安装仍待实现：完整平台工具包、工件版本/校验清单、Rust 组件准备、升级恢复和
  卸载规则均需另行设计与验收。安装入口、命令名称和发布渠道未定；`toolchain install`
  或 `setup` 不能写成可执行的现有命令。`cargo install` 不会自动部署私有 bridge 和
  完整 sysroot，现阶段使用源码构建。规划不授权发布或提交。
* bootstrap 与 create 保持第 10 节边界；应用级 Clippy、更多图目标、其他 host/编辑器
  和更广泛宏组合仍属后续范围，不从旧讨论恢复手动注册 DSL 或公开宏 package。
* `tools/compiler-probe/verify_autobind.py` 仍有运行期捕获非法图的历史预期，不能作为
  当前整组应通过的 gate；维护它时应将负例迁到 check/build 拒绝，保留有效图运行断言。
* 当前文档入口为 [文档导航](docs/README.md)。服务用法集中于声明指南，运行期设计
  集中于 core README，编译器原理集中于 rustc 指南；修改实现时更新相应唯一入口。
  重复的阶段计划与迁移日志不再增加独立主文档；详细命令、原始日志和临时核查记录放
  `target/`，性能结论归入 [性能与内存](docs/NESTRS_PERFORMANCE.md)，保留基线与适用范围。
* 文档必须区分已实现接口、源码可见能力、实际验收证据与尚未实施的计划。历史性能
  数字不能当作当前完整实现的重新测量；临时 target 证据不随仓库分发。
