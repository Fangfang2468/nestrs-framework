# 修复说明与详细记录

本文统一保留 2026-10-01 至 2026-10-03 多轮修复及其后续复核。每轮分别记录触发条件、
根因、处理方式、正式回归入口和当时的验证边界；后续发现不会覆盖或删除前一轮的记录。
当前用法见[服务声明与注入](NESTRS_MACROS.md)，实现原理见
[编译器扩展指南](NESTRS_RUSTC_EXTENSION_GUIDE.md)和
[core 运行期设计](../nestrs-core/README.md)，测量方法与数值见
[性能与内存](NESTRS_PERFORMANCE.md)。这些文档各自维护当前契约，本文维护修复历史。

文中的“通过”是对应工作区快照和测试范围的历史结果，负例通过表示编译器按预期拒绝。
测试套件存在重叠，外层 harness 数不能与内部案例数相加；259 项显式 ignored 的生成
文档测试没有计为执行通过。2026-10-02、2026-10-03 下述修复与复核均使用 Linux
`x86_64-unknown-linux-gnu`、固定 Rust 1.98.0，完整 commit 为
`88d9e12ae178fab0fb5cc050a94da85685d449ea`；没有将 2026-10-01 的 Windows
历史结果视为后续实现的 Windows 验收。

原始日志、任务起点快照、精确差异及工具哈希保存在各轮 `target/` 目录，不随仓库分发。
本文正文和正式源码、测试链接保留必要说明，使清理 `target/` 后仍能理解修复。
记录编号 R01～R13 是本文的时间线索引，不是原报告中会重复使用的 F1/P2 编号，
也不是 Git 提交号。各轮基线均包含当时已有未提交工作，没有用 Git HEAD 代替实际快照。

## 修复与复核时间线

| 日期 / 记录 | 修复内容 | 随后复核发现的边界 |
| --- | --- | --- |
| 2026-10-01 / R01 | LSP 诊断取消重试与原生平台验证 | 双平台结果仅适用于该轮源码；不继承到后续修复 |
| 2026-10-02 / R02 | 地址来源安全修复、factory 局部名称冲突 | Class 生成绑定和 raw 名称拼接仍有遗漏 |
| 2026-10-02 / R03 | Class 卫生、raw 名称、static / 标准库转发查询、非 `.rs` 根文件 | 自动绑定局部变量和具名辅助项尚未隔离 |
| 2026-10-02 / R04 | core 订阅、关闭工作空间、容量回收和地址恢复器优化 | 后续发现的是既有 codegen 缺口，未归因为内存优化 |
| 2026-10-02 / R05 | 自动绑定局部变量、factory provider 与 cfg carrier 名称隔离 | 匿名 const 内辅助项、constructor 成员、隐式调用等组合仍有遗漏 |
| 2026-10-02 / R06 | 全部生成项卫生、constructor 身份、隐式 Deref/Drop、raw 关联类型 | 普通对象 dyn 擦除、无关宽类型预算、括号 optional 尚未覆盖 |
| 2026-10-02 / R07 | dyn 来源、DI 预算相关性、括号 optional | 投影父接口和同 trait 不同 impl 的预算误用尚未覆盖 |
| 2026-10-02 / R08 | 父接口投影归一化、实际查询预算、小投影 | 普通外部 helper 与不同 impl 的增长身份仍有遗漏 |
| 2026-10-02 / R09 | 外部原生 MIR、关联类型相关性、真实实例增长身份 | 元组 Clone 编译器适配代码未进入实际 MIR 路径 |
| 2026-10-03 / R10 | Clone shim 的真实实例 MIR | 第五轮独立复核未发现新增可复现缺陷，范围限当时 Linux 快照 |
| 2026-10-03 / R11 | 统一 root / scope 创建初始化，移除独立 warm_up API | 属于明确授权的 API 调整；此前功能和性能结论不自动覆盖本次实现 |
| 2026-10-03 / R12 | build / create_scope 各收敛为接受 Option 的单一入口 | 保留 R11 的初始化契约；None 继承默认，Some 完整覆盖，两者不能混同 |
| 2026-10-03 / R13 | 修复 IDE 验证脚本对单行字段访问的依赖 | 夹具格式化阻断正式验证；保留原有 LSP 类型和导航断言 |

这些发现来自逐步扩充的输入组合。后续新缺陷不意味着前一轮的原始修复失效；每轮都应
同时保留原触发复测和新增失败证据，不能用已有测试全绿替代边界核查。

## R01：LSP 诊断取消重试与双平台历史验收

**触发与根因。** 原版 rust-analyzer 可以对 `textDocument/diagnostic` 返回
`ServerCancelled (-32802)`，并通过布尔值 `data.retriggerRequest = true` 请求重试。
验证器未处理此协议分支，合法的服务端取消会使 LSP 验收失败。

**修复。** [LSP 验证器](../tools/verify-ide.py)只对上述方法、错误码和严格布尔标志
重试。每次分配新请求 ID，保留原参数和总 deadline；等待计入原预算，不丢弃通知和
服务端请求。其他错误仍失败，原有 `-32800/-32801` 行为保留。

**正式回归。** [Python 回归](../tools/test_verify_ide.py)的 7 项测试覆盖新 ID、
通知保留、标志缺席/false/非布尔值、其他错误，以及反复取消或响应到达时耗尽预算。
相同测试用于旧脚本时 3 项失败，修复后 7 项通过。Linux 真实 LSP 观察到
`-32802/retriggerRequest=true` 并恢复；Windows 本轮真实会话只观察到 `-32801`，
其 `-32802` 分支由单测覆盖。

**历史验证与演进。** 本轮 Linux GNU 和原生 Windows MSVC 均执行了实际工具链、
DI、跨 crate 和原版 LSP 检查；两端 477 个实现/测试/配置文件逐个核对一致。
当时增加了 CI 配置，但未执行云端 Actions；后续工作区已移除该 workflow，不能把
本记录写成当前存在自动 CI 门禁。原始证据：`target/readiness-fixes/REPORT.md`
与 `final-audit.json`。

## R02：直接构造输入之后的地址来源与 factory 名称修复

**背景。** 直接构造输入调整将逐参数装箱准备改为完整 `ConstructionInputs` 和
typed take，保留真实 factory lease frame。输入/lease 数组容量问题在该轮开发中修正，
中间样本未混入最终性能统计；该性能变化及基线由[性能与内存](NESTRS_PERFORMANCE.md)
统一说明。其后 `target/core-final-fixes-20261002/` 单独处理以下两项正确性问题。

**地址来源。** 旧 `ErasedService` 在封装仍可能移动时提前保存从 `Box` 取得的地址。
地址数值相等不能证明旧指针在后续所有权移动之后仍可解引用。修复保留
`Box<dyn Any + Send + Sync>`，通过准确类型的地址恢复函数，在服务发布到稳定实例
记录后从当前共享借用派生地址；由 lease 保持实例，只有最后一个 lease 释放后才移动
或销毁记录中的值。没有引入裸分配所有权或手工析构，原指针持有者的两项
`unsafe Send/Sync` 实现移除。后来的 R04 将每实例恢复器 Box 改为按类型静态共享，
这两个阶段不能混写成一步。

正式入口为 [ErasedService](../nestrs-core/src/activation/erased_service.rs)、
[实例 lease](../nestrs-core/src/activation/instance.rs)及
[地址回归](../nestrs-core/tests/unit/activation/erased_service.rs)。回归验证未发布封装
移动、失败 downcast 后发布、跨线程读取、ZST、准确类型/不定长类型拒绝和唯一析构。
独立标准库模型还运行了 Stacked Borrows / Tree Borrows；这是不同 nightly 上的合成
模型，不是整个仓库的 Miri 通过证明。

**factory 局部变量。** 旧 adapter 的 `__nestrs_factory_input_0`、context、service、
error 等固定局部名会匹配业务同名常量。修复复用原 factory 函数标识符承载参数元组，
调用保持 `self::factory`；typed take 按声明顺序求值，最后验证全部消费后才调用用户
函数。普通参数仍借用真实 frame，lazy 参数按值移动，未增加替代实例或跨 await 假借用。
[真实 DI 回归](../cargo-nestrs/tests/fixtures/di/tests/runtime_direct_inputs.rs)
覆盖同步、异步、显式 Future、成功/失败、optional、lazy 和同实例断言。

**当轮验证。** 普通测试 363 项、compiler-driver 281 项、DI 28 个外层测试及跨 crate
32 项通过。内存复核的 352 个正式进程样本逐对分配指标一致，另有 6 个长循环进程；
该结果只支持这些负载下未观察到新增分配或持续残留，不证明吞吐/延迟提升。
原始证据：`target/core-final-fixes-20261002/final-audit.json`、
`pointer-design-review.md`、`factory-hygiene/notes.md` 和 `memory/`。

**随后遗漏。** `target/core-cloud-review-20261002/` 与 `target/audit-20261002/`
确认 Class 绑定和 raw 拼接仍失败。factory 的修复不能替代 Class 和其他生成位置的
卫生处理，故继续 R03。

## R03：Class / raw 标识符及首批查询和工具兼容性修复

这一轮汇总五类已复现缺陷，原始缺陷审查为 `target/audit-20261002/REPORT.md`，
修复证据为 `target/fixes-20261002/REPORT.md`。

### Class 绑定卫生与 raw 标识符

`const error: usize = 47` 会使 Result constructor 生成的 `.map_err(|error| ...)`
把 `error` 解析为常量模式。显式 constructor 的 inputs/instance 和自动字段构造的
context/instance 也有同根因问题。修复由私有 bridge 将真实 `Span::def_site()` 显式
交给唯一 codegen 后端，统一生成卫生 Ident；业务类型、表达式、构造函数体保留原 token
来源，错误格式化使用显式参数。维护者授权仅扩展到私有 bridge 构建，应用与普通
core/工具测试不接收 bootstrap。

raw factory `fn r#type()` 和 raw injectable `struct r#type` 曾把 `r#` 直接拼入
内部符号，导致过程宏 panic。内部名称拼接改用 `IdentExt::unraw()`，业务引用继续使用
原 Ident。raw constructor 方法名原本已能工作，本轮保留该控制，未把问题泛化成所有
raw 名称不可用。正式回归位于
[Class / raw 名称契约](../cargo-nestrs/tests/fixtures/di/tests/runtime_identifier_hygiene.rs)，
实现入口包括[constructor 生成](../cargo-nestrs/src/codegen/constructor_codegen.rs)与
[factory 生成](../cargo-nestrs/src/codegen/injection/macros/factory/codegen.rs)。

### static 查询根、标准库转发和 Cargo 根输入

跨 crate 的不可变 `static QUERY = query::<T>` 曾遗漏初始化器摘要；标准库
`Iterator::collect` 转发到业务 `next()` 时，也会丢失 `Repository<T>` 查询。
两者均表现为编译成功、计划为空、实际解析报 `ServiceNotRegistered`，而本地 static、
跨 crate const 或直接 `next()` 对照成功。

修复在私有摘要中保留 static 初始化器的真实身份与调用关系，不求值函数地址；按真实
类型、DefId 和 rustc Instance 读取相关标准库 MIR，用工作队列传播到固定点。
[查询方法契约](../cargo-nestrs/tests/query_method_contracts.rs)与
[上游转发 fixture](../cargo-nestrs/tests/fixtures/query-methods/library/src/forwarding_queries.rs)
覆盖 static、关联常量、inline const、函数项、嵌套 collect 和未执行分支等组合。

合法 Cargo 根文件 `entry.code`、无后缀路径曾因 `.rs` 后缀猜测而无法 init/doctest，
虽然 check/build 成功。修复按固定 rustc 的真实选项表识别位置输入，并按原参数索引
替换 doctest 载体；constructor 模型使用实际 compiler input。
[native host 回归](../cargo-nestrs/tests/native_host.rs)与
[rustdoc 回归](../cargo-nestrs/tests/rustdoc.rs)保留中文、空格、非 `.rs` 和无后缀输入。

**当轮验证。** 历史 7 个触发与 7 个对照共 14 次真实运行通过；普通工具 231、core
134、compiler-driver/bridge 288、DI 30 外层、应用 workspace 375 项通过。
后续受限资源测量不替代功能回归，测量完整范围集中于性能文档。原始证据还包括
`target/fixes-20261002/query/`、`hygiene-formal/`、`tooling/`。

## R04：core 内存优化及随后正确性复核

**问题与处理。** 多个常见运行期结构保留了不必要的小分配或峰值容量。本轮做四项
调整：零/单订阅内联，Pending 输入按需分配；复用完整冻结图的关闭排序工作空间；
每 4096 个调度轮按窗口峰值回收明显过剩容量；地址恢复器按类型静态共享。
它们保留查询取消只取消等待、成功实例 lease、完整 Lazy 关闭依赖和失败缓存契约。
无调度事件时不会自动唤醒缩容，长期 root 的 Transient 和历史 cleanup 错误也仍按
owner 契约保留，不能将这些保留描述成已消除。

**正式回归。** [订阅测试](../nestrs-core/tests/unit/runtime/subscriptions.rs)真实挂起
4096 个查询、交错取消 2048 个并保留 128 个 Lazy watch；
[关闭测试](../nestrs-core/tests/unit/runtime/cleanup.rs)以独立传递可达性算法核对
543 个四节点 DAG、185,163 种发布历史；
[容量测试](../nestrs-core/tests/unit/runtime/capacity.rs)检查峰后回收不丢活动记录或失败
缓存；[投影分配测试](../nestrs-core/tests/unit/activation/construction/projection.rs)
检查静态恢复器与真实投影。

**历史结果。** core Debug/Release 各 146 项、DI 30 外层、workspace 387 项、11 个
隔离 core 编译契约和 32 个跨 crate 场景通过。九场景三版本七轮配对测量，以及同机
PostgreSQL/Redis 受限负载，详见[性能与内存](NESTRS_PERFORMANCE.md)；保留大图
工作空间存活量上升等取舍，不以分配请求下降代替 RSS 或服务获取速度结论。
原始证据：`target/core-memory-20261002/REPORT.md`、`completion-audit.json`。

**复核发现与因果。** `target/final-functional-audit-20261002/` 新确认两类名称问题，
并证明涉及的三个生产文件与内存优化前完全一致。因此它们是既有 codegen 缺口，
不是本轮内存优化引入的回归。core 运行期审阅与回归没有确认新增可复现缺陷；此结论
也不等于穷尽所有并发交错。对应修复见 R05。

## R05：自动绑定局部变量与具名辅助项隔离

**触发与根因。** compiler source overlay 的 `service`、`projected`、`slot`、
`input`、`target` 被业务同名常量/static 捕获，造成合法自动 trait 绑定编译失败。
另外，factory provider const 和 cfg 字段 derive carrier struct 与同名业务 item
争用命名空间，报 E0428。bridge 中 Class/factory 局部变量的修复未自动覆盖这些位置。

**修复。** [自动绑定生成](../cargo-nestrs/src/compiler/autobind_codegen.rs)将投影
代码置于原匿名 const 内的独立私有模块，不导入父模块名称；真实类型路径、coercion、
强 lease 和来源审计保留。factory provider 与
[条件字段 carrier](../cargo-nestrs/src/codegen/conditional_fields.rs)使用已授权
def-site Ident，业务声明、属性和 value token 保持来源。没有通过改成长固定名要求
业务避让，也没有增加 bootstrap 授权或公开宏 package。

**正式回归。** [自动绑定卫生](../cargo-nestrs/tests/autobind_hygiene.rs)覆盖五个
局部名、static、私有嵌套类型、raw 名称、泛型、宏、路径别名及同实例投影；
[生成项名称回归](../cargo-nestrs/tests/fixtures/di/tests/runtime_generated_item_hygiene.rs)
覆盖具名项冲突。原审计 7 个触发运行通过，4 个对照成功，2 个私有访问负例继续拒绝。
新增同源回归在旧工具上出现 10 条 E0428，新工具的 3 项运行通过。

**当轮结果与后续。** compiler-driver 289 项、workspace 387 项、DI 33 个外层
通过。之后 `target/final-functional-audit-after-fixes-20261002/` 重新确认这 7 个
原触发已修好，但新增探针发现匿名 const 内的辅助项、marker 和 constructor 关联项
仍有其他卫生缺口，以及隐式查询/raw 关联类型问题，继续 R06。原始修复记录为
`target/identifier-isolation-repair-20261002/REPORT.md`。

## R06：生成项完整卫生、constructor 身份和隐式调用

本轮来自上述独立复核的五项发现，修复目录为
`target/merge-readiness-repair-20261002/`。五项不是 R05 原触发再次失败。

**生成项捕获。** 匿名 const 不能独自提供宏卫生：业务 `__nestrs_reflect` 模块或
`__nestrs_construct` 函数可能被生成项遮蔽，甚至让 `#[value(...)]` 编译成功却调用
内部空 marker、静默跳过业务副作用。字段模式/input marker 的 let 还会被业务常量
解释为模式。修复统一处理反射模块、marker、构造/注册辅助项和生成引用的 def-site
卫生，保留业务 token；正式
[生成项回归](../cargo-nestrs/tests/fixtures/di/tests/runtime_codegen_hygiene.rs)
检查副作用次数、业务函数/模块/类型同名、隐藏 bind 和私有访问边界。

**constructor 成员身份。** `__nestrs_constructor_activate`、
`__NESTRS_CONSTRUCTOR` 等生成关联项可能与合法成员重名。编译器改为只将认证候选
连接到真实 AssocFn DefId，QSelf 保留服务实参；IDE v2 从标准展开 AST 的标识符目录
分配名称并保留完整方法 token、绝对源码位置及模型输入校验。

中途独立复核发现，IDE 的初版名称目录只收集 impl 中显式写出的成员，仍会与本地
或上游 trait 的默认静态成员重名。同一程序 CLI 运行成功，但原版 rust-analyzer
将业务调用绑定到生成 helper，出现 E0061/E0308，hover 也显示错误的构造签名。
修复将目录扩展为完整标准展开 AST 的标识符，包含业务调用路径，使上游默认成员的
实际使用名称也被避开。修后本地/上游 trait 两组运行、模型与真实 LSP 均通过，
Deref 控制组仍正常。独立前后证据保存在
`query-roots/independent-constructor-review.md`，正式 IDE 回归入口为
[verify-ide.py](../tools/verify-ide.py)。

本轮独立复核还发现专门化 impl 会触发 rustc ICE，继续增加 Self 蓝图参数覆盖检查：
允许真实类型/const/生命周期参数改名和重排；具体、嵌套、重复或固定 const 专门化
给出正常诊断。完整蓝图仍要求参数直接覆盖，不在 HIR 前展开类型别名证明等价。
正式入口为 [constructor 契约](../cargo-nestrs/tests/constructor_contracts.rs)及
[有效泛型身份](../cargo-nestrs/tests/fixtures/constructors/src/bin/generic_identity.rs)。

**隐式查询。** 自动 Deref/DerefMut、Drop 路径未显式出现在旧调用摘要中，造成闭合
查询漏根及非法依赖被错误接受。修复读取真实 adjustment 与原生 DropGlue，摘要位置
在 drop elaboration 后、常量分支优化前；关联字段、GAT、不透明字段及多个实例仍按
真实类型展开。`forget`、`ManuallyDrop`、引用不制造并不存在的析构查询，`if false`
保留已编译需求。core 隔离 cfg(test) Drop 钩子同时补齐正式契约。
测试入口为 [隐式查询契约](../cargo-nestrs/tests/query_implicit_contracts.rs)和
[core 编译契约](../nestrs-core/tests/compiler/)。

**raw 关联类型。** 自动绑定 overlay 曾将 `dyn r#trait<r#type = ...>` 写成
`type = ...`。修复从真实 ExistentialProjection 身份打印原始标识符，保留 const、
alias、私有路径、重导出、HRTB 与同实例投影；可见
[raw 类型 fixture](../cargo-nestrs/tests/fixtures/raw-types/main.rs)。

**历史验收。** 原触发/对照 Debug+Release 64 条符合预期；core 两种 profile 各
146、普通工具 233、compiler-driver/bridge 295、workspace 389、DI 36 外层通过，
另执行 3 个隔离运行期组合探针。开发中的失败迭代保留在 `iteration-1/`、
`iteration-2/`；一次 `BinaryHeap` 导入丢失也保留差异并恢复后重跑。
后续 `target/merge-review-20261002/review.md` 又发现 R07 的三个输入组合，故当轮
“可以进入合入审查”不能当作此后无需再核查的保证。

## R07：普通 dyn 对象、无关宽类型与括号 optional

**dyn 来源遗漏。** 普通非 provider 的 `Runner<u64>` 转为 `Box<dyn Run>` 后，
动态方法中的 `Repository<u64>` 查询曾丢失。修复由 `QueryUnsize` 摘要保留真实
unsize 源/目标，经跨 crate metadata 恢复有限具体来源，再结合 Virtual 调用和 rustc
义务求解实现。普通业务 impl 不因此注册为 provider，也不因擦除而执行/收集其所有
方法。兼容来源按链接单元保守合并，不承诺逐对象数据流分析。
[动态查询契约](../cargo-nestrs/tests/query_dynamic_contracts.rs)覆盖不同指针、上游
泛型、默认/父 trait、关联类型、auto trait 与未使用方法；当轮为 4 个运行正例和
8 个 check/build 负例。

**预算误拒。** 与 DI 无关的普通宽类型 impl 被提前施加 DI008。自动绑定先确认真实
provider 身份；查询载体发现与服务类型预算分开。有限普通大载体不提前拒绝，真实
服务仍受限制；持续泛型增长在归一化前拦截，总展开预算保留。独立审阅又发现
`Box::new<Runner<Wide>>` 的相同提前计数路径，继续补修。
[类型预算契约](../cargo-nestrs/tests/type_budget_contracts.rs)当轮覆盖 Debug/Release
30 个子案例。此时增长链仍按共享定义身份处理，R09 才进一步修正实际 impl 身份。

**括号 optional。** 显式 constructor 将 `(Option<T>)` 视为非 optional 并错误拒绝。
编译器与 IDE 改为透明穿过嵌套括号，只改写真实服务槽，保留 AST、限定路径、NodeId
及 span。[正式 fixture](../cargo-nestrs/tests/fixtures/constructors/src/bin/parenthesized_optional.rs)
覆盖普通/lazy、有值/缺席、字段/参数嵌套括号和 Singleton 同实例；IDE 验证 hover 和
原始源码跳转，lazy 不提前构造。

**验收中发现的测试问题。** 默认并行 workspace 曾失败。隔离诊断复现
`Text file busy (os error 26)`，来自 CLI fixture 并行写入/执行临时脚本。仅在
[CLI 测试](../cargo-nestrs/tests/cli.rs)增加 fixture 生命周期互斥和失败输出，未在
生产加入重试；32 线程完整 31 项测试连续 50 轮通过，正式并行 workspace 再通过。
第一次完整 compiler 验证也因并行 Cargo 重建共享工具、改变 feature fingerprint
而停止；恢复匹配工件后整套重跑，未将中断算作通过。

**当轮结果。** compiler-driver/bridge 300、workspace 390、DI 36 外层通过，原始
14 条与独立动态 4 条命令符合预期。原始证据为
`target/merge-three-defects-repair-20261002/REPORT.md`；后续独立复核
`target/merge-rereview-20261002/review.md` 确认这三项原始场景通过，但发现 R08。

## R08：投影父接口归一化与实际查询预算

**父接口投影。** `trait Child<T: Map>: Run<T::Target>` 的动态调用需要把
`<u8 as Map>::Target` 归一化为 `u64` 再匹配具体 impl。旧代码只擦除生命周期，
直接比较投影与结果类型，导致计划为空；静态调用或直接写 `Run<u64>` 的控制成功。
修复在相同 fully-monomorphized 环境归一化闭合 supertrait reference 和调用参数，
比较真实 DefId/类型身份，并保留 object-shape 的 Unsize 求解；归一化错误不静默当成
“不匹配”。[投影父接口 fixture](../cargo-nestrs/tests/fixtures/query-dynamic/src/bin/projected_super.rs)
为每条路径使用独立 Tag，避免静态对照掩盖动态遗漏；非法图核对唯一 DI001。

**粗筛不是实际查询。** 某个 `Run` impl 有查询会将 trait item 标为相关，旧实现在
`Instance::try_resolve` 选出真实空 impl 前便对整个调用参数套预算，误拒 `Wide::run`。
修复删除 Callable/Constant 全部参数的立即服务预算；相关性只决定是否继续发现，
真实 Service 类型、同链增长及 100000 总实例预算保留。空 impl、默认方法、helper
和关联常量不能仅因另一个实现查询服务而成为大服务。

交叉审阅还发现 `Repository<<Wide as Family>::Target>` 归一化为
`Repository<u8>` 时，输入 Wide 被错误计入服务大小。含 alias 的服务先归一化后再
检查结果类型；无 alias 继续在归一化前检查。Wide→u8 正例与 Tag→Wide 负例同时保留，
[预算契约](../cargo-nestrs/tests/type_budget_contracts.rs)从 30 扩为 48 个子案例。

**候选回归与最终证据。** candidate-1 虽仍准确报唯一 DI008，却遗漏原诊断中的
“DI 泛型类型不断增长或过于复杂”“递归泛型查询”说明，被既有 `aot_graph` 拒绝。
保留候选日志，停止相关验证，只修复生产诊断后重新冻结工具并重跑，未改测试掩盖回归。
最终 compiler-driver/bridge 301 项、workspace 390 项、DI 36 外层通过。
原始修复为 `target/query-followup-repair-20261002/REPORT.md`；其后的
`target/merge-third-review-20261002/review.md` 确认旧问题闭环，再发现 R09。

## R09：普通外部 helper 与真实实例的增长身份

**外部调用遗漏。** 不直接依赖 core、也没有 Nestrs 查询摘要的普通外部 helper，仍
可能通过泛型/trait/常量调用业务查询。旧分析提前停止，运行漏注册，非法依赖也可能
不被拒绝。修复对已闭合、相关且无摘要的定义按需读取原生 MIR，已有摘要仍优先；
读取 `mentioned_items`、`required_consts`，关联常量走 CTFE MIR，不求值函数地址。
[driver](../cargo-nestrs/src/bin/nestrs-driver.rs)为普通依赖保留 MIR metadata，
使 check 的 rmeta 与 build 的 rlib 提供一致关系。

早期候选仍遗漏 primitive Self 和 `Family::Target`。进一步使用真实签名输入/输出，
以及函数、父级、父接口、关联类型 bounds 的有限 DefId 闭包做相关性传播，覆盖参数、
返回、仅 body 构造、默认方法、嵌套关联类型、dyn、GAT 和 where 约束；粗筛本身不
注册服务，实际调用仍由 rustc 选择实现。

**不同 impl 误判增长。** 有限 `Start -> Wide` 调用因共享 trait item 被当成同一
函数递归扩大。增长 ancestry 改按真实 `InstanceKind` 比较，分别识别实际 impl、
常量和具体 DropGlue。真正方法、常量及关联投影的无限增长仍 DI008。
同一个真实泛型 helper 沿链重入且实参持续增长，仍可能保守拒绝，包括最终选择无查询
实现的情况；本轮没有宣称解决所有理论可终止的泛型展开。

**正式回归。** [外部查询契约](../cargo-nestrs/tests/query_external_contracts.rs)及
[无 core helper](../cargo-nestrs/tests/fixtures/query-external/plain-helper/src/lib.rs)
覆盖 96 条命令：8 个运行正例、88 个编译负例，额外 GAT/where 探针 20 条。
正式矩阵与额外探针合计 100 条负例，各只有一个 DI001；运行正例用独立服务和精确构造计数证明未执行分支也
贡献需求。[预算契约](../cargo-nestrs/tests/type_budget_contracts.rs)扩至 64 个
Debug/Release 子场景，有限不同 impl 正例真实运行，增长反例核对唯一 DI008。

**候选与验收。** candidate-4 被新增 `Family::Target` 探针推翻后停止并归档，
修复后所有编译器/工具验收用最终工件重跑。随后普通 Cargo 改变共享 CLI fingerprint
导致一次完整套件中断，恢复匹配工件后独占重跑；外部 runner 曾使用错误环境变量名，
另核对真实子进程选择的 bridge 并保留纠正记录，未把不明工件当作证据。
最终 compiler-driver/bridge 303 项、workspace 390 项、DI 36 外层通过，生产只改
查询收集器和 driver，39 个 core 生产 Rust 文件未变。原始证据为
`target/query-third-repair-20261002/REPORT.md`、`external/FINAL-REPORT.md`。

**后续遗漏。** `target/merge-fourth-review-20261002/review.md` 确认上述修复通过，
但发现元组 Clone 编译器适配代码没有沿实际实例继续分析，转入 R10。

## R10：元组与闭包 Clone shim 查询遗漏

**触发与根因。** `Runner<T>::clone` 查询已声明的 `Repository<T>`，应用克隆
`(Runner<u64>,)` 时，实际路径是元组 Clone → 字段业务 Clone → 服务查询。旧收集器
仅对 DropGlue 读取编译器生成的实例 MIR，把元组 Clone 当普通方法声明处理，因而
Debug/Release 都编译成功、计划为空、运行报 `ServiceNotRegistered`；缺失依赖的
tuple check/build 也被错误接受。直接 Clone 和数组 Clone 控制成功。

**修复。** [查询收集器](../cargo-nestrs/src/compiler/query_roots.rs)将
`ShimKind::Clone` 与 DropGlue 一并交给 `tcx.instance_mir`，沿字段真实调用继续
分析。该 MIR 已由固定 rustc 按具体类型生成，不再次代入 trait 声明参数；使用完整
实例身份去重，保留相关性筛选和增长预算。不手写字段展开，不执行用户 Clone 发现根，
不同元组/闭包不按共同 `Clone::clone` DefId 合并。

**正式回归。** [Clone 契约](../cargo-nestrs/tests/query_clone_contracts.rs)使用
[12 个独立 binary](../cargo-nestrs/tests/fixtures/query-clone/src/bin/)，覆盖本地及
无 core helper、元组、嵌套元组、闭包捕获、未执行分支、多个闭合实例以及
direct/array/no-clone 控制。独立 binary 避免直接查询控制组替遗漏路径注册同一服务。
矩阵共 68 条命令：24 运行正例、40 唯一 DI001 负例、4 不克隆控制，核对 28 份计划。

原始 14 条触发/控制在旧工具重新复现后，新工具 Debug/Release 均符合预期，6 份
原始计划包含对应服务。独立相邻探针 36 条还验证 Copy tuple、tuple clone_from、
稳定异步闭包、FnDef/FnPtr 和引用元组，并核对 20 份计划。不能认为所有 Copy 类型
都跳过自定义 Clone；固定 rustc 的数组优化基于 TrivialClone。未稳定 coroutine
本体只作源码审阅，未列作实际运行通过。

**当轮完整验证。** 普通 core/工具 380、compiler-driver/bridge 305、workspace
390、DI 36 个外层（UI 含 68 案例）通过；隔离 core 契约 12 案例、预算 64 子场景、
跨 crate 32、graph 25、宏 6 次运行与 all-targets、Chromium 11 项及原版 LSP
具体场景均通过。严格 Clippy、格式、差异和 Python 检查通过。各集合重叠不加总。

该轮生产只改一个查询分析文件，core 39 个生产文件未变，没有新增 unsafe。
701 文件起点变为 719 文件，修改 4、新增 18、删除 0，包含正式测试与文档。
原始证据为 `target/query-clone-repair-20261003/REPORT.md`、
`clone-contracts/REPORT.md`、`semantic/REPORT.md`。

## 第五轮独立复核：截至 2026-10-03 的历史状态

`target/merge-fifth-review-20261003/review.md` 对 R10 完成后的 719 文件完整工作区
重新构建工具、复测原始 14 条与 68 条正式 Clone 契约，并执行完整 Linux 回归，
没有发现新的可复现缺陷。当时审阅意见为该快照可进入 master 合入，未实际提交、推送
或合并；结论不能用于遗漏未提交修复和测试的旧 HEAD。

额外边界检查确认 tuple `clone_from` 保留查询；共享引用、借用闭包、Arc/Rc 克隆及
仅捕获未克隆的 owning closure 不制造内部查询。相同泛型闭包的 u16/u32 闭合分别
进入计划；增长为 `Runner<(T,)>` 的 Clone 路径唯一 DI008，同类型对照通过。
第五轮普通测试 380、compiler-driver 305、workspace 390、DI 36 外层，外部查询
96 条与预算 64 子场景均通过；隔离 core 外层从主集拆出执行，其 12 案例没有省略。

这次文档收敛保留上述历史结果，不将整理文档算作重新执行完整功能、性能或部署验收。
Windows/其他 target 没有此次结果，LSP 验证不覆盖所有编辑器 UI；功能正确性也不替代
真实 API、PostgreSQL/Redis 同机 2 GiB/2 核的容量测试。更多 MIR metadata 的构建成本
未量化，后续编译器修复也没有重新测量 runtime 内存或服务获取延迟。

今后新增修复应继续在本文追加详细记录，并在对应唯一指南更新当前行为；原始命令、
候选失败、工具哈希和临时核查材料留在 `target/`，避免再建立重复的阶段主文档。


## R11：统一 scope 创建初始化与移除独立预热 API

本节记录这一阶段当时的接口与验收；后续 R12 将默认／显式配置合并为 Option 参数，
删除两个 with_options 方法。这里保留当时写法，现行调用请看 R12 与 core README。

**背景与原行为。** 旧版 `create_scope()` 是同步方法，只建立 owner；即使 Scoped
服务标记 `#[lazy(false)]`，也要等查询或显式 `scope.warm_up().await` 才会构造。
独立预热以 Eager 作为未标注 Scoped 的默认策略，失败后返回 `ResolveError`，已交付
scope 的关闭仍由调用者负责。它与 root 的 build 配置、创建期初始化及失败清理契约不同。
维护者明确选择统一创建行为，删除公开及内部独立预热入口；这是有兼容性影响的 API
设计调整，不将旧版行为重新解释为一直存在的功能缺陷。

**处理方式。** `create_scope().await` 返回 `Result<ServiceScope<'_>, ScopeBuildError>`。
root 与 scope 的初始化策略独立、缺省均为 Lazy。`ServiceProviderOptions` 增加
`scope_initialization`；入口 manifest 的 `[nestrs-cli]` 增加 `scope-initialization`。
`create_scope_with_options(ServiceScopeOptions { initialization })` 可覆盖单次创建，
不修改容器默认值，也不增加 scope 独立的构造额度。Singleton 在 build、Scoped 在
scope 创建时应用同一服务级三态策略：未标注继承本次默认，true 跳过自主初始化入口，
false 即使默认 Lazy 也初始化。Transient 仍只按实际消费构造。

`Runtime::warm_up` 与公开 `ServiceScope::warm_up` 移除。
[Runtime](../nestrs-core/src/runtime/handle.rs) 的 `start` / `create_scope` 只在创建期
调用私有 `initialize_owner`；它持有尚未交付的 owner，不能对已交付 owner 再单独
初始化。Lazy scope 也等待协调器确认登记，避免交付已被关闭中的 runtime 拒绝的
scope。初始化仍复用查询请求、真实依赖展开、缓存和中央协调器，没有另一套预热命令
或构造调度器。服务图、字段级 lazy、每 root 构造并发上限和强 lease 契约保持一致。

**失败与取消。** scope 创建失败先等待本 scope 排空和关闭，再返回
`ScopeBuildError { error, dispose_error }`；它保留 `ResolveError` 与可能的
`DisposeError`，不会关闭仍由应用持有的 root 或其他 scope。创建 future 被取消时，
未交付 owner 的 Drop 发起幂等关闭，已接受任务继续排空清理；取消者没有等到清理
结束，也不能据此保证 Tokio runtime 已退出后的异步 cleanup。Singleton 依赖属于
root，scope 失败不撤销 root 的共享构造或失败缓存。

**编译期配置协议。** options sink 升级为 `plan_set_options_v3`，分别携带 root / scope
默认值；driver 核对存在性与完整签名，空图也拒绝旧 core。reflect 执行入口仍为
`__nestrs_reflect_v2`，反射 JSON 格式仍为 version 1，新增 `scopeInitialization`
配置字段。三个版本各有用途，不将本次 options sink 的变化描述成整个执行 ABI 或
JSON 同时升级。工具、core 和私有 bridge 需要配套重建。

**迁移与回归位置。** 同步调用改为 `.create_scope().await?`；原独立预热调用删除，
需要相同行为时明确选择 scope Eager。完整 options 字面量补上 `scope_initialization`。
结账示例用 `--scope-initialization lazy|eager` 替代 `--warm-up-scopes`。当前用法统一见
[core README](../nestrs-core/README.md#6-生命周期与预热)，不在本文再保存一份现行教程。

真实 driver 的[scope 初始化与失败清理契约](../cargo-nestrs/tests/fixtures/di/tests/runtime_scope_initialization.rs)
覆盖创建失败、成功实例清理、cleanup 失败保留和创建取消后的排空。
运行期策略和创建失败/取消由[初始化测试](../nestrs-core/tests/unit/runtime/initialization.rs)、
[请求取消测试](../nestrs-core/tests/unit/runtime/requests.rs)及
[门面测试](../nestrs-core/tests/unit/facade_api.rs)负责。真实项目配置与单次覆盖见
[启动配置契约](../cargo-nestrs/tests/startup_config.rs)，声明级覆盖与跨 crate 场景见
[provider lazy 契约](../cargo-nestrs/tests/provider_lazy_contracts.rs)；
[ABI 契约](../cargo-nestrs/tests/registry_abi.rs)与
[反射产物契约](../cargo-nestrs/tests/reflection_artifacts.rs)检查同步演进。
HTML 图只解释当前创建策略，不执行初始化；[浏览器回归](../tools/verify-graph-page.cjs)
检查 Scoped 的 Lazy / Eager / 继承提示，以及字段延迟与服务策略的区别。

**本轮实际验证。** 固定 Rust 1.98.0 的 Linux x86_64 环境下，普通 core / 工具测试
384 项、真实工具链 workspace 测试 394 项、DI runtime / UI 的 38 个外层测试均通过；
UI harness 包含 68 个子案例。完整 compiler-driver 契约按目标去重后复核通过 305 项，
其中包括承载 12 个隔离 core 案例的一个外层测试；类型预算另覆盖 Debug / Release
共 64 个子场景。25 条图导出命令与当前新导出页面的 11 个浏览器场景
均符合预期；普通与 compiler-driver 严格 Clippy、工作区及修改文件格式检查通过。
示例、跨 crate 调用迁移及四次基准构建、20 个短场景运行也通过，后者仅作功能检查。
这些集合存在重叠，不能相加；259 个仓库显式忽略的文档测试不计入通过数量。

compiler-driver 首轮的 provider-lazy 图测试曾出现一次目标目录创建 `ENOENT`。
原测试串行复跑和全新目标目录下的并发复跑均通过，目录监测未发现对应目录被删除
或移动；首次失败原因尚未确认，没有据此修改生产逻辑或跳过断言。原日志与复验
记录保留在本轮证据目录，不将复验通过改写为首次运行全绿。

本次没有重新测量性能或 2 GiB / 2 核容量；此前数字继续保留原日期与基线。
验证命令、日志和本次实际执行范围记录在 `target/scope-creation-20261003/`，
文档链接检查另存于 `target/scope-initialization-20261003/`；不把旧版完整矩阵或历史
平台结果当成本次自动通过证明。


## R12：以 Option 参数收敛容器与 scope 创建入口

**背景。** R11 已统一创建期初始化，但对默认配置与显式选项分别提供了 `build` /
`build_with_options`、`create_scope` / `create_scope_with_options`。维护者进一步要求
各类创建只保留一个入口，通过可选参数表达配置来源。本轮保留 R11 的独立 root /
scope 模式、服务级覆盖、初始化失败关闭、取消排空和共享构造上限。

**当前接口。** [公开门面](../nestrs-core/src/facade.rs)只保留
`ServiceProvider::build(options: Option<ServiceProviderOptions>)` 与
`provider.create_scope(options: Option<ServiceScopeOptions>)`。两个 with_options
方法直接移除，不提供兼容转发；返回类型仍分别为 `Result<ServiceProvider, BuildError>`
和 `Result<ServiceScope<'_>, ScopeBuildError>`，均须等待。

None 采用既有默认：build 使用最终入口 package 的编译配置，scope 使用当前 root
保存的 `scope_initialization`。Some 完整使用显式选项，不与项目配置逐字段合并；
scope 的 Some 只影响这一次创建。`Some(Default::default())` 明确选择库的 Lazy
基线，不等价于 None；root 的完整库基线还包含 scope Lazy 与 32 个构造名额。
服务级 `#[lazy]` / `#[lazy(false)]` 仍优先，API 收敛不改变冻结计划或字段延迟机制。

**迁移。** `build().await` 改为 `build(None).await`，`create_scope().await` 改为
`create_scope(None).await`；两个旧 with_options 调用分别改为 `build(Some(options))`
和 `create_scope(Some(options))`。作用域生命周期、错误来源、取消与关闭语义沿用 R11。
本轮没有再次修改 options sink v3、reflect 入口 v2 或 JSON schema 1。

**回归入口与记录。** [公开门面契约](../nestrs-core/tests/unit/facade_api.rs)、
[生产构建契约](../nestrs-core/tests/compiler_plan_required.rs)及
[真实启动配置契约](../cargo-nestrs/tests/startup_config.rs)分别覆盖新签名、普通 Cargo
与工具链行为、项目默认和显式覆盖；真实 scope 初始化契约继续验证失败及取消。
源码、文档、示例、生成测试程序和基准工具调用同步迁移，但未重新测量历史性能。
本轮原始证据位于 `target/option-creation-api-20261003/`，不把 R11 的通过数量归到
这次接口收敛。当前用法统一见[core 初始化说明](../nestrs-core/README.md#6-生命周期与预热)。

**本轮实际验证。** 固定 Rust 1.98.0 的 Linux x86_64 上，普通 core / 工具测试
384 项、真实工具链 workspace 测试 394 项通过；直接受影响的配置、ABI、反射、查询及
rustdoc 契约共 16 项通过，DI runtime / UI 38 个外层测试通过，包含 68 个 UI 子案例。
5 个有调用迁移的隔离 core 工程实际执行 8 个契约测试并通过；未重复执行未改动的
其余隔离工程及完整 Clone / 类型预算矩阵。示例、跨 crate、宏工具链和基准代码迁移
检查通过；基准仅作短场景功能运行，不产生性能结论。匹配工具重建、两套严格 Clippy、
格式及差异空白检查通过；当前 HTML 模板的 11 个浏览器场景、12 份 Markdown 的
471 条链接与锚点检查通过。测试集合有重叠，259 个显式忽略的文档测试不计入通过数；
未重新验证 Windows、跨 target 或完整 LSP。


## R13：IDE 字段定位支持格式化后的多行访问链

**触发与根因。** R12 之后的第六轮复核确认，IDE 夹具中的
`optional.delayed_present.as_ref()` 已被格式化为多行，但正式验证脚本仍查找
连续字符串 `optional.delayed_present.`。脚本在 default 冷启动通过后抛出
`ValueError: substring not found`，尚未发送该字段的 hover / definition 请求，
也无法完成后续编辑、诊断恢复、保存检查及 alternate / release 验证。原报告只在
target 中修正定位进行对照，不能作为正式脚本已通过的证据。

**修复。** [正式验证脚本](../tools/verify-ide.py)为四个 optional 字段共用的定位
增加空白与换行支持，按匹配字段名在原始源码中的绝对偏移生成 LSP 坐标，不压缩
源码，也不按连续片段重新查找位置。匹配必须唯一；缺失或歧义会报告接收者、字段名
及匹配数量。原有 Option / Injection / LazyInjection 类型断言和定义文件、行号
断言全部保留。IDE 夹具、生成器及 core 运行期实现没有为此修改。

**回归入口。** [验证器单元测试](../tools/test_verify_ide.py)增加 5 项字段定位回归，
覆盖单行、空格、点号前后换行、CRLF、接收者与字段名称前后缀干扰、缺失与歧义，
并直接定位当前仓库夹具的四个 optional 字段。定位回归与既有取消重试回归共
12 项通过。真实 LSP 仍通过正式 `tools/verify-ide.py` 命令独立验证；本轮原始日志
与起点快照保存在 `target/ide-field-locator-fix-20261003/`。

**本轮实际验证。** 使用固定 Rust 1.98.0 的 Linux x86_64 工具链及其原版
rust-analyzer，原失败命令 `python3 tools/verify-ide.py --skip-build --rust-analyzer
/root/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/rust-analyzer` 已退出 0。
default 配置下字段及工厂 hover、补全、定义跳转、未保存编辑、真实诊断与恢复、
保存检查及缓存稳定性全部通过；alternate 和 release 的项目模型与冷启动检查
通过；不支持的 cfg 仍被拒绝并保留原模型及设置。原始 IDE 夹具内容保持不变。
本次仅修复验证脚本和补充配套回归、文档，未重新运行完整 core / compiler-driver
矩阵，也未重新验证 Windows、其他 target、性能或服务器容量。
