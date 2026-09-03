# Nestrs DI 设计讨论交接（Q&A）

> 生成时间：2026-09-03（Asia/Shanghai）
> 用途：把本轮关于 `#[bind]`、`Inject<T>`、`#[injectable]`、`#[factory]` 与激活运行时的设计讨论交给另一台电脑上的 Codex 接续。
> 性质：这是设计决策与当前源码快照，不表示其中的目标架构已经实现。

## 如何使用这份交接文档？

**Q：在另一台电脑上怎样让 Codex 接续？**

**A：** 打开同一仓库后，把本文件作为上下文附给新任务，并说明：

```text
请读取 NESTRS_DI_QA_HANDOFF.md，继续其中的 Nestrs DI 设计与实现。
先核对当前工作树和源码；不要把文档中的“建议架构”误认为已实现。
```

本文中的“当前状态”是生成时的只读快照；开始编码前必须重新检查源码、测试和工作树。

---

## 一、`#[bind]` 与反射元数据

### Q1：`#[bind]` 应当做什么？

**A：** `#[bind]` 标注一个正向 trait impl：

```rust
#[bind]
impl GetUser for SqlUserService {
    // ...
}
```

它的语义不是注册第二个独立 provider，而是声明一个类型级别的导出别名：

```text
(SqlUserService, key = K)
    ──以 trait 视图投影──> (dyn GetUser, key = K)
```

因此，`#[inject] dyn GetUser` 可以在组合阶段选择 `SqlUserService`；实际对象仍由 concrete provider 构造和持有。

### Q2：`#[bind]` 为什么不允许参数？

**A：** 当前语义完全由 Rust 的 `impl Trait for Concrete` 表达。支持 `#[bind(...)]` 会制造没有必要且难以解释的第二套绑定语义。因此宏必须在 AST 层直接拒绝任何参数：

```rust
#[bind(foo)]
impl GetUser for SqlUserService {}
// error: `#[bind]` 不接受参数
```

### Q3：报错“`#[bind]` 不支持负 trait impl”是什么意思？

**A：** Rust 中：

```rust
impl !GetUser for SqlUserService {}
```

是负 impl，含义是“此类型明确不实现该 trait”。它不能形成 `Concrete -> dyn Trait` 的注入绑定，所以 `#[bind]` 必须拒绝它。可绑定的只能是普通正向 impl：

```rust
impl GetUser for SqlUserService {}
```

### Q4：仅保存 `ConcreteType` 与 `dyn Trait` 的 `TypeId` 足够吗？

**A：** 只够在 `compile()` 阶段找候选，不够在运行时交付 trait object。

`dyn Trait` 是 fat pointer。通用运行时代码不能只靠 `TypeId` 从 `&Concrete` 恢复正确的 trait vtable。因此 `#[bind]` 最终还必须生成类型化 projector，概念上类似：

```rust
fn project(value: &SqlUserService) -> &dyn GetUser {
    value
}
```

但真正记录的是隐藏的 erased adapter：它从已就绪的 concrete slot 制作 `Inject<dyn GetUser>`。不要尝试用 `NonNull<()>` 保存这种投影；那会丢失 DST 元数据。

### Q5：bind 的 key、primary、lifetime 怎样处理？

**A：** 建议固定为“继承 concrete provider”：

```text
SqlUserService + key = "replica"
    #[bind] impl GetUser for SqlUserService
=> dyn GetUser + key = "replica"
```

无 key 的 `dyn GetUser` 不应意外匹配 keyed provider。`primary`、lifetime、实例所有权和 cleanup 也都来自 concrete provider，而不是 bind 自己重新声明。

---

## 二、`#[constructor]`、静态反射与自定义构造

### Q6：属性宏能从 `impl UserController` 的 `#[constructor] fn new(...) -> Self` 得到 `UserController` 吗？

**A：** 一个函数级 attribute macro 只会得到 `ItemFn`，看不到其外层 `impl UserController`。如果返回类型写 `Self`，它在函数 token 中也不是可直接生成 `TypeId::<UserController>` 的 concrete 类型。

Rust 编译器当然能验证构造函数本身是否正确，但 proc macro 不具有静态反射能力，无法可靠地将任意 impl 方法与对应 `#[injectable]` struct 关联成一个全局 ConstructorMeta。

### Q7：宏展开时能读取 linkme 收集到的数据吗？

**A：** 不能。proc macro 在编译期运行；`linkme` 分布式切片是最终二进制中的运行时静态收集结果。宏只能生成新的 slice item，不能在展开时枚举其他 crate 的 linkme 项。

### Q8：为什么不继续设计 `#[constructor]`？

**A：** 它会迫使系统解决“函数宏如何可靠取得 impl 所属 concrete 类型”的问题，却没有比 `#[factory]` 带来额外能力。当前结论是保留 deprecated 口子：

```rust
#[deprecated(
    since = "0.1.0",
    note = "`#[constructor]` Rust暂不支持静态反射，还无法实现该功能，先留下口子，需要自定义构造请先使用 `#[factory]`"
)]
```

自定义构造统一由模块级私有 `#[factory] fn ... -> Result<Concrete, Error>` 表达；宏可以直接从返回类型得到 concrete service identity。

### Q9：若 `#[injectable]` 自动构造与同类型 `#[factory]` 同时存在，如何选择？

**A：** 两者是相同 `ServiceIdentifier` 的两个 provider 候选。不要使用隐式“factory 永远覆盖 injectable”的规则；使用显式 `primary = true`：

```rust
#[injectable(lifetime = Scoped)]
struct UserController {
    #[inject]
    users: dyn GetUser,
}

#[factory(lifetime = Scoped, primary = true)]
fn create_user_controller(
    #[inject] users: dyn GetUser,
) -> Result<UserController, InitError> {
    Ok(UserController { users })
}
```

编译图根据唯一 primary 选择 factory。请注意：factory 一旦被选中，返回对象就是最终对象；框架不会在 factory 返回后“偷偷补写”字段。若 factory 要构造含依赖字段的 struct，它必须显式接收并放入那些依赖。

---

## 三、NestJS、Spring 与 Rust 的边界

### Q10：为什么 Rust 难以完全复刻 NestJS 的构造器参数属性？

**A：** TypeScript 支持：

```ts
class Test {
  constructor(private readonly a: A) {}
}
```

这同时声明构造器参数和类字段。Rust 没有这一语法糖：`fn new(a: A)` 不会自动成为 struct field。宏只能改写它看得见的 item；函数级宏又看不到 `impl` 所属类型。因此不能自然把“构造参数”和“结构字段”合并成 NestJS 风格的一处声明。

### Q11：Spring Boot 的构造器注入是否创建实例后再反射填字段？

**A：** 构造器注入不是。Spring 先反射选择构造器，解析参数，再调用构造器生成实例。字段注入是另一个后处理阶段，对实例字段做反射写入。

Rust 可以概念上实现“先构造、后字段填充”，但会遇到私有字段、未初始化状态、pin/移动、生命周期和 `unsafe` 安全边界。尤其如果字段最终存的是裸 `Inject<T>` 指针，后填充并不能消除其所有权与稳定地址问题。

### Q12：能否先把字段填 `None`，构造后再 `unsafe` 补上？

**A：** 技术上可以构造某些延迟初始化机制，但这不解决 API 设计根问题：

- `T` 未必能表示“未初始化”；
- 普通字段可能没有 `Option` 形态；
- 用户构造函数可能已读取字段；
- 需要阻止对象在半初始化状态逃逸；
- 注入 token 的来源生命周期仍必须被 runtime 持有。

因此不应把“先 None 再补写”作为 `#[constructor]` 与字段注入协同的主方案。

---

## 四、`Inject<T>`：旧思路、当前骨架与安全边界

### Q13：`Inject<T>` 的本质是什么？

**A：** 它不是 `T` 本身，也不应成为可写 setter；它是容器管理的、指向已激活服务的依赖 token。当前骨架是：

```rust
pub struct Inject<T: ?Sized> {
    ptr: NonNull<T>,
    _marker: PhantomData<T>,
}
```

它为 concrete 类型和 `dyn Trait` 提供统一的 token 外形，但目前没有构造器、`Deref`、`Clone`、`Copy`、`Send`/`Sync` 或 runtime 集成。

### Q14：为什么当前先只保留类型定义？

**A：** 因为公开这些能力需要先证明以下不变量：

- 被指向服务处于稳定地址；
- 服务在所有 token 使用期间存活；
- shutdown/cleanup 不会留下悬垂 token；
- `dyn Trait` 投影保留正确 vtable；
- 若实现 `Send`/`Sync`，跨线程访问仍受 slot/lease 所有权保护。

在这些不变量尚不存在时，留下 `todo!()` 的访问 API 只会制造可调用后 panic 的伪能力；仅保留 ABI 形状更安全。

### Q15：旧项目的 `Inject<T, Access>` 思路能否参考？

**A：** 可以借鉴“token 的可逃逸性和生命周期由 access mode 区分”的原则，例如 factory 参数可使用带 frame lifetime 的包装，而最终 struct 字段使用可持久 token。但不要直接搬旧实现的 `FactoryContractMeta`、动态 resolver 或旧身份模型。

当前新设计的依赖身份已经是 `ServiceIdentifier { type + key }`；同一份宏分析应同时生成 metadata、输入位置和 adapter 取参代码。

### Q16：factory 参数可以存入返回服务吗？

**A：** 这是必须明确的安全决策。

第一阶段可以保守地把所有 `#[factory]` 注入参数的依赖 lease 都附着到成功输出上，从而支持普通写法：

```rust
fn make_repo(#[inject] db: Database) -> Result<Repo, Error> {
    Ok(Repo { db })
}
```

但如果 `Inject<T>` 始终只是无生命周期的裸指针，编译器无法阻止用户把它送到容器外的后台任务。因此在开放 `Deref`、cleanup、`Send`/`Sync` 前，需要最终选择：

1. 让 `Inject<T>` 私有持有 lease/引用计数；或
2. 为只在 factory 调用期有效的输入提供 `FactoryInject<'frame, T>` / `FactoryRef<'frame, T>`。

不能把 `NonNull<T>` 当作 `Arc<T>` 使用。

---

## 五、服务身份、metadata 与构造 ABI

### Q17：为什么不再保留旧版 `FactoryContractMeta` 双重校验？

**A：** 当前 `ServiceIdentifier = ServiceType + ServiceKey` 已足以表达“这个字段/参数请求哪个服务”。宏应只分析一次：

```text
InjectionSpec
  ├─ DependencyRequest metadata
  ├─ ConstructionContext 的输入 position
  ├─ 字段/参数改写后的 Inject<T>
  └─ adapter 内的 ctx.take::<T>(position)
```

只要 `ConstructionContext` 不暴露 `resolve(ServiceIdentifier)`，而只交付 compile 阶段已绑定的固定位置输入，业务 factory 就不能在运行时把“声明的 A”换成“偷偷查询 B”。

运行时仍应验证 erased slot 与 requested type/projection 匹配；这是保护 `unsafe` 和 type erasure 的边界，不是第二份依赖声明。

### Q18：`ServiceDescriptor` 与字段/参数 metadata 如何分工？

**A：** 保持分层：

```text
ServiceDescriptor
  = provider 声明：提供什么、key、lifetime、primary、source、实现方式

ConstructionRecipe / DependencyRequest[]
  = 如何构造这个 provider：需要哪些输入、position、optional、调用哪个 adapter

CompiledProviderGraph
  = 已绑定结果：每个输入 position 实际读取哪个 ProviderId、如何投影交付
```

手工 instance provider 不一定有字段；factory 也不一定有 struct field。因此不要把 `field_name` 等 consumer 细节硬塞进 `ServiceDescriptor`。

建议的核心数据模型：

```rust
#[repr(transparent)]
pub struct InputPosition(pub u16);

pub struct ProviderMeta {
    pub identifier: ServiceIdentifier,
    pub lifetime: Lifetime,
    pub primary: bool,
    pub cleanup: Option<CleanupHook>,
    pub source: ServiceSource,
}

pub struct DependencyRequest {
    pub position: InputPosition,
    pub service_identifier: ServiceIdentifier,
    pub optional: bool,
    pub label: &'static str,
    pub source: ServiceSource,
}

pub struct ServiceRegistration {
    pub descriptor: ServiceDescriptor,
    pub dependencies: Box<[DependencyRequest]>,
}
```

### Q19：候选选择规则是什么？

**A：** v1 应固定成可诊断规则：

```text
0 个候选：
  optional => AbsentOptional
  required => MissingDependency

1 个候选：
  选择它

多个候选：
  恰有一个 primary => 选择它
  否则 => AmbiguousDependency
```

只有将来支持 `#[inject(all)]`、fallback、显式只取 primary 等语义时，才增加 `DependencySelection`。

---

## 六、宏展开到运行时实例化的完整链路

### Q20：`ConstructionContext` 是什么？

**A：** 它是“某个已编译 activation node 的输入包”，不是 service locator。它不应该有：

```rust
context.resolve::<T>()
context.resolve_by_key(...)
```

它只暴露隐藏宏 ABI：

```rust
impl ConstructionContext {
    pub unsafe fn take<T: Injectable + ?Sized>(
        &mut self,
        position: InputPosition,
    ) -> Result<Inject<T>, ActivationError>;

    pub unsafe fn take_optional<T: Injectable + ?Sized>(
        &mut self,
        position: InputPosition,
    ) -> Result<Option<Inject<T>>, ActivationError>;
}
```

`unsafe` 由宏生成代码使用，不交给业务代码；runtime 会检查 position、optional、slot ready 状态、类型及 bind projection。

### Q21：同步和异步 factory 要两套 context 吗？

**A：** 不要。只有一套 `ConstructionContext`。

```text
#[injectable]       → 同步 structural adapter
sync #[factory]     → 同步 factory adapter
async #[factory]    → async factory adapter

三者取输入         → 同一个 ConstructionContext
```

异步只影响 adapter 的返回形态：`Ready(ErasedService)` 或 `Pending(Future)`。async adapter 必须在创建 future 前同步取完所有参数；future 内不得再向 context 请求服务。

### Q22：推荐的 constructor ABI 是什么？

**A：** 概念形态如下：

```rust
pub enum ConstructorInvoker {
    Sync(SyncConstructor),
    Async(AsyncConstructor),
}

pub type SyncConstructor =
    fn(ConstructionContext) -> Result<ErasedService, ActivationError>;

pub type AsyncConstructor =
    fn(ConstructionContext) -> ActivationFuture;

pub type ActivationFuture =
    Pin<Box<dyn Future<Output = Result<ErasedService, ActivationError>> + Send>>;
```

`ErasedService` 私有持有 concrete allocation、真实 `ServiceType` 和稳定地址；成功后由 `ReadySlot` 持有。

### Q23：`#[injectable]` 的概念展开是什么？

**A：** 用户写：

```rust
#[injectable(lifetime = Scoped)]
struct UserController {
    #[inject]
    users: dyn GetUser,

    #[inject(key = "audit")]
    audit: Option<dyn AuditLogger>,
}
```

宏概念上改写为：

```rust
struct UserController {
    users: Inject<dyn GetUser>,
    audit: Option<Inject<dyn AuditLogger>>,
}

fn __construct(mut context: ConstructionContext)
    -> Result<ErasedService, ActivationError>
{
    let users = unsafe { context.take::<dyn GetUser>(InputPosition(0))? };
    let audit = unsafe {
        context.take_optional::<dyn AuditLogger>(InputPosition(1))?
    };

    Ok(ErasedService::new(UserController { users, audit }))
}
```

并通过 `::nestrs_core::__private::linkme` 记录 provider metadata、输入请求和 `__construct` 指针。

### Q24：`#[factory]` 的概念展开是什么？

**A：** 用户写：

```rust
#[factory(lifetime = Singleton)]
async fn create_user_client(
    #[inject] users: dyn GetUser,
) -> Result<UserClient, ClientInitError> {
    UserClient::connect(users).await
}
```

宏把函数参数改为 token，并生成 adapter：

```rust
async fn create_user_client(
    users: Inject<dyn GetUser>,
) -> Result<UserClient, ClientInitError> {
    UserClient::connect(users).await
}

fn __invoke(mut context: ConstructionContext) -> ActivationFuture {
    let users = match unsafe {
        context.take::<dyn GetUser>(InputPosition(0))
    } {
        Ok(value) => value,
        Err(error) => return Box::pin(async move { Err(error) }),
    };

    Box::pin(async move {
        let service = create_user_client(users)
            .await
            .map_err(ActivationError::factory)?;
        Ok(ErasedService::new(service))
    })
}
```

第一版 factory 建议只接受非泛型、模块级私有函数，并限定返回 `Result<Concrete, Error>`。先拒绝 `impl Trait`、引用输出、复杂模式参数和泛型 factory，确保每个 linkme 注册项都是单态的。

---

## 七、编译图、激活图与并发

### Q25：为什么不能只把依赖 DAG 拍平成 `Vec<Operation>`？

**A：** 拓扑序可以用于串行策略，但不能替代图：

- 并发调度需要知道哪些节点已无未满足前置依赖；
- singleton/scoped 的多个消费者需要汇合到同一个 slot；
- transient 在不同消费位置必须多次激活；
- 失败传播、cleanup 顺序、等待者和 single-flight 都依赖边关系。

因此保留 immutable `CompiledProviderGraph`，需要执行时再物化 `ActivationTaskGraph`。

### Q26：图节点应该是 `ServiceIdentifier` 吗？

**A：** 不应。编译图节点是 `ProviderId`，表示一个具体注册候选。运行时节点是 `ActivationId` / `ActivationKey`，因为 transient 必须按消费位置实例化：

```rust
enum ActivationKey {
    Singleton(ProviderId),
    Scoped { scope: ScopeId, provider: ProviderId },
    Transient { parent: ActivationId, input: InputPosition },
}
```

“每条边一个任务”只近似适用于 transient；singleton/scoped 的多条边会共享同一个 activation slot。

### Q27：single-flight 和“抢占”是什么关系？

**A：** 不应实现成抢占。slot 状态应是：

```text
Empty
  └─ 第一个请求成为 owner → Activating
       ├─ 成功 → Ready(service, child leases, cleanup)
       └─ 失败 → Failed(error)
```

其他请求遇到 `Activating` 时等待 owner 的结果。节点有多个依赖时，必须等待所有输入 ready，不能让某一条边单独“抢走”节点。

### Q28：异步构造是否等于并发实例化？

**A：** 不等于。async factory 只意味着某个节点返回 future。并发来自 scheduler 同时执行多个 ready node；先实现串行 drain ready queue，再加并发上限，不需要改变宏 ABI 或 compiled graph。

### Q29：runtime 应如何执行？

**A：**

1. 从 entry 或 eager singleton root 物化 activation task。
2. 为每个 `ResolvedInput` 获得或创建依赖 activation。
3. 所有依赖 ready 后，按 `InputPosition` 构造 `PreparedInput[]`。
4. 创建一次 `ConstructionContext`，调用 sync/async adapter。
5. 成功后 commit 到当前 `ReadySlot`，唤醒 dependents/waiters。
6. 失败时不启动后继；按反向依赖顺序 rollback/cleanup。

---

## 八、当前源码快照（生成本文件时）

### Q30：当前仓库已经实现到哪里？

**A：** 当前仍是 metadata 与宏校验骨架，尚无完整 DI runtime。

- `nestrs-macro/src/lib.rs`
  - `#[injectable]` 目前主要解析配置并校验 module scope/cleanup；尚未分析 `#[inject]` 字段或生成 adapter。
  - `#[factory]` 目前主要校验普通私有函数、非 `unsafe`/`extern`、无 `self`；尚未生成 metadata 或调用 adapter。
  - `#[constructor]` 已 deprecated。
  - `#[bind]` 已拒绝参数、负 impl、泛型 impl，并生成 basic bind metadata。
- `nestrs-macro/src/injection/injectable/field_analyze.rs` 与 `field_rewrite.rs` 已存在但当前为空，适合作为共享输入分析/改写的落点之一。
- `nestrs-core/src/inject_wrapper.rs` 只有 `Inject<T: ?Sized>` 的指针 token 骨架。
- `nestrs-core/src/metadata/injectable.rs` 有 `FieldInjection`、`StructComponent` 与 `REFLECT_METADATA_INJECTABLE`，但缺 provider 完整元数据、稳定输入位置和 adapter。
- `nestrs-core/src/metadata/factory.rs` 存在 `FactoryComponent`、`FactoryParameterInjection` 与 `REFLECT_METADATA_FACTORY` 的新骨架，但尚未接入完整构造 ABI。
- `nestrs-core/src/registration/service_descriptor.rs` 仍有无法使用的 `ServiceImplementation::ConstructFactory(fn())` 占位。
- `nestrs-core/src/registration/service_collection.rs` 当前只读取并打印 bind metadata；尚未收集 injectable/factory provider，更未 compile/activate。
- `InterfaceBinding` 目前只记录 concrete/trait `ServiceType`，尚无 trait projector。

### Q31：工作树是否干净？

**A：** 生成本文件时不是。已观察到以下改动，应视为用户或并行工作，不能覆盖：

```text
M  example/nestrs-base/src/main.rs
M  nestrs-core/src/lib.rs
M  nestrs-core/src/metadata/injectable.rs
M  nestrs-core/src/metadata/mod.rs
?? nestrs-core/src/metadata/factory.rs
```

本交接文档本身是新增文件。继续实现前先重新运行 `git status --short`，区分已有改动与新任务改动。

---

## 九、建议的实现顺序与验证

### Q32：落地顺序是什么？

**A：**

1. 冻结 source grammar：`#[inject]`、`Option<T>`、key、primary、factory 返回契约、bind key 继承。
2. 在 `nestrs-core` 定义隐藏 ABI：`ErasedService`、`ConstructionContext`、`InputPosition`、`ConstructorInvoker`、错误与受控 token 构造入口。
3. 扩充 provider metadata：完整 `ProviderMeta`、dependency request、adapter 指针；移除/替换 `ConstructFactory(fn())` 占位。
4. 扩充 `#[bind]` metadata，生成 concrete → trait 的 projector。
5. 实现宏共用 `InjectionSpec` analyzer；再分别完成 injectable 的 struct rewrite 和 factory 的 parameter rewrite/adapter generation。
6. 让 `ServiceCollection` 收集 linkme metadata，normalize 成 `ServiceRegistration`。
7. 实现 `compile()`：候选选择、Missing/Ambiguous/Optional、bind、cycle、lifetime 检查，产出 immutable DAG。
8. 先实现 singleton + 同步 structural/sync factory + slot commit 的串行 activation。
9. 加 async factory 的 Pending future 与 activation frame 保活。
10. 最后实现 scoped、transient child ownership、cleanup、ready-queue 并发。

### Q33：建议优先写哪些测试？

**A：**

- 宏 `trybuild`：非法 bind 参数/负 impl/泛型 impl；非法 injectable 字段；非法 factory 参数、泛型或返回类型；source rewrite 结果。
- metadata 集成测试：linkme 收集 injectable、factory、bind；key 与 optional 正确。
- compile 测试：MissingDependency、AmbiguousDependency、唯一 primary、optional absent、cycle、key 继承、lifetime inversion。
- runtime 测试：concrete 注入、trait 注入确实可调用、optional `None`、同步 factory、async factory、singleton single-flight、transient 每消费位置新建。
- 生命周期/cleanup 测试：输出释放前 child transient 仍有效，反向 cleanup 顺序正确。

---

## 十、不可违反的设计边界

### Q34：继续设计时最重要的禁止项是什么？

**A：**

1. 不要让 `ConstructionContext` 变成动态 service locator。
2. 不要重建旧版 `FactoryContractMeta` 的第二份依赖身份事实。
3. 不要只保存线性 activation plan 而丢掉 DAG。
4. 不要把 async factory 误当成并发调度。
5. 不要仅靠 `TypeId`/`NonNull<()>` 恢复 `dyn Trait`。
6. 不要在 `Inject<T>` 尚无所有权证明时添加 `Deref`、`Send`/`Sync` 或可逃逸构造器。
7. 不要依赖独立 `#[primary]` macro 自动与 provider macro 共享 metadata；应优先收敛为 `primary = true` provider config。
8. 不要让 factory 隐式覆盖 injectable；同 identity 候选必须通过 primary 或未来明确 override 规则选择。

### Q35：一句话总结最终架构？

**A：**

```text
宏只把同一份输入分析结果变成 metadata 与固定位置 adapter；
compile() 负责把 ServiceIdentifier 绑定为唯一 provider/input slot；
runtime 只激活 DAG、持有实例与 lease，并把已就绪输入交给 adapter。
```
