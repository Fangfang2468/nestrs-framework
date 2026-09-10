# Nestrs DI 反射与激活架构设计 QA

> 生成时间：2026-09-04
>
> 性质：基于当前源码与后续设计讨论整理的设计文档。
>
> 状态：当前仓库并未完整实现本文描述的最终架构；本文用于统一设计认知，并为后续实施提供路线。
>
> 相关文档：
>
> - `AGENTS.md`：项目架构共识与 Git/Commit 规范
> - `NESTRS_DI_QA_HANDOFF.md`：较早的 DI 设计交接记录，部分内容已过时
> - `nestrs-core`：DI 运行时与反射 ABI
> - `nestrs-macro`：过程宏与元数据生成
> - `example/nestrs-base`：当前最小示例

---

## 一、当前实现现状

### Q1：当前代码实现了什么？

**A：**

当前仓库保留了 `#[injectable]`、`#[bind]` 与 `#[factory]` 的 Provider 宏、统一
linkme 注册 ABI 与 Arena 构造 ABI，但尚未提供服务
activation runtime，因此还不是可端到端实例化的 DI 框架。

现有 workspace 成员：

```text
example/nestrs-base
nestrs-core
nestrs-macro
```

尚未存在：

- `nestrs-bootstrap`
- `nestrs-logger`
- `nestrs-config`

核心功能来自 `nestrs-core` 与 `nestrs-macro`：

```text
#[injectable] / #[bind] 宏
        ↓
生成 AST、构造 adapter、Provider 定义
        ↓
linkme 收集
        ↓
供未来 Provider-first activation runtime 消费
```

当前测试覆盖的 ABI 包括：

- Arena 稳定地址
- 逆序析构
- `Inject<T>` 读取
- 预绑定构造输入
- `#[bind]` 的 typed trait 投影
- 闭合泛型 Provider callback
- Class、Bound 与 Factory Provider 的统一 linkme 收集
- 同步与异步 Factory invoker ABI

但以下能力尚未完整落地：

- `Provider` 只描述注册意图，尚未执行 provider 选择或激活
- `Lifetime` 只被记录，没有被运行时执行
- cleanup 会被记录为 async hook，但尚未被调用
- 没有 Provider activation / resolve runtime
- 没有全局注册表 / 编译图
- 没有 async 构造和并发激活
- 没有 scope 和 transient 语义

---

### Q2：`Provider` 为什么当前没有被 activation runtime 消费？

**A：**

你的观察正确。旧的递归激活器已经移除；
当前没有运行时激活路径：

```text
linkme 收集的 `Provider`
        ↓
等待未来 Provider-first activation runtime
```

Provider ABI 位于：

```text
nestrs-core/src/registration/provider.rs
```

`Provider::Class`、`Provider::Factory`、`Provider::Bound` 与 `Provider::Alias`
统一描述注册/构造 ABI，尚无 activation runtime 直接消费它们。Class / Factory
provider 共同保留：

- 服务身份
- `ProviderCommon`（生命周期、primary、source、cleanup）
- `InjectionSpec` 依赖列表
- 结构体构造 adapter 或 factory invoker

也就是说，它们是未来 Provider 模型可复用的“宏生成注册对象”，但不能被误认为
当前已经可解析服务的运行时容器。

---

## 二、注册意图与反射的区别

> **重要澄清：反射层与注入语义层不能混为一谈。**
>
> 当前代码中的 `InjectionSpec`、`prepare_input`、`Constructor`、`FactoryInvoker` 等，
> 属于“DI 注入语义”和“provider 注册信息”，
> 它们不是通用的“类型反射”。
>
> 本文后文为了与现有代码衔接，暂时把 `ComponentReflection` 当作“provider 注册 +
> 注入语义”的工作名；最终模型建议改名为 `RegisteredProvider` / `ProviderReflection`，
> 避免与通用类型反射混淆。

> **最终修正（2026-09-05）：**
>
> 前面大量讨论以 `Type / ComponentReflection` 为主线，隐式地把 DI 建模成
> “type-centric”。但 Rust 的基础单元不是 class，模块级自由函数、trait impl、
> 泛型定义和实例都可以是 provider。
>
> 因此本设计的最终核心概念不是：
>
> ```text
> Type -> ComponentReflection
> ```
>
> 而是：
>
> ```text
> Provider
>     = ProviderCommon
>     + InjectionSpec[]
>     + Constructor | FactoryInvoker | Bound projector
> ```
>
> `Type / ReflectionRegistry` 保留为“已注册类型的辅助查询能力”，
> 不再承担 provider 注册与 DI 执行的主载体。
>
> 本章之后的 `ComponentReflection`、`Reflect`、`Type::of<T>()` 等表述，
> 均应理解为“辅助反射层 / 类型信息层”，而不是 DI 的核心注册模型。

## 最终模型确认：Provider Metadata System

Nestrs 的 DI 基础单元不应是 `Type`，而应是：

```text
Provider
```

Provider 元数据统一描述：

```text
token / ServiceIdentifier
implementation / Provider variant
dependencies / InjectionSpec[]
invoker / ProviderInvoker
lifecycle / Lifetime
selection / primary + key
source / ServiceSource
```

当前 Provider 形态：

```rust
pub enum Provider {
    Class,
    Factory,
    Bound,
    Alias,
}
```

开放泛型不作为 `Provider` 枚举变体注册；它通过 `ProviderDefinition::provider()` 与
注入点中的已单态化 callback 在需要时生成闭合 `Provider::Class`。

这个模型与 NestJS 的 Provider 概念非常接近：

| NestJS | Nestrs 建议 |
|---|---|
| `@Injectable()` | `#[injectable]` |
| `useClass` | `Provider::Class` |
| `useFactory` | `Provider::Factory` |
| `useValue` | 尚未实现 |
| `useExisting` | `Provider::Alias` |
| `InjectionToken` | `ServiceIdentifier` |
| `@Inject` | `#[inject]` |
| `@Optional` | optional injection |
| module providers | `ProviderRegistry` |
| `Reflector` | `TypeRegistry`（辅助） |

因此最终不是：

```text
Type Reflection + Type-centric DI
```

而是：

```text
Provider Metadata + Macro registry + DI runtime
```

反射层保留为：

- 类型查询
- 诊断
- 辅助工具
- 可选的高级 API

不承担核心注册和构造决策。

### 进一步修正：不再使用扁平 ServiceDescriptor

ASP.NET Core 的 `ServiceDescriptor` 本质上是：

```text
ServiceType + ImplementationType + Lifetime
```

它把“类型”和“实现方式”放在同一个扁平结构里，适合 C# 的 Type-centric DI。

Nestrs 如果继续保留一个统一的 `ServiceDescriptor`，即使字段再丰富，
也仍然会让人回到：

```text
Type -> Descriptor -> Provider
```

的正确方向应该是：

```text
ServiceIdentifier = token
Provider = 不同 provider 变体组成的元数据
```

当前核心 ABI：

```rust
pub struct ProviderCommon {
    pub lifetime: Lifetime,
    pub primary: bool,
    pub source: ServiceSource,
    pub cleanup: Option<CleanupHook>,
}

pub enum Provider {
    Class {
        provide: ServiceIdentifier,
        common: ProviderCommon,
        dependencies: Vec<InjectionSpec>,
        constructor: Constructor,
    },
    Factory {
        provide: ServiceIdentifier,
        common: ProviderCommon,
        dependencies: Vec<InjectionSpec>,
        invoker: FactoryInvoker,
    },
    Bound {
        trait_type: ServiceType,
        concrete_type: ServiceType,
        key_policy: BoundKeyPolicy,
        prepare_required: PrepareInput,
        prepare_optional: PrepareInput,
        source: ServiceSource,
    },
    Alias {
        provide: ServiceIdentifier,
        target: ServiceIdentifier,
        source: ServiceSource,
    },
}
```

这里：

- `ServiceIdentifier` 是 provider token
- `Provider` 是 provider 元数据的 sum type
- 各类 provider 使用自己的结构
- 不再必须存在一个“万能 ServiceDescriptor”
- `ProviderRegistry` 直接收集 `Provider`
- 宏为每个 provider 生成 `fn() -> Provider`

这样：

```text
NestJS provider
    =
token + 不同类型的 provider metadata
```

而 Nestrs 的差异只是：

```text
provider 由宏自动生成
linkme 自动收集
用户不需要手动注册
```

### 已完成迁移：metadata 模块已由 Provider 替代

原 `nestrs-core/src/metadata` 的 DTO 已删除；其语义已直接迁移到 Provider
体系：

| 旧 metadata DTO | 新归属 |
|---|---|
| `StructComponent` | `Provider::Class` |
| `FieldInjection` | `InjectionSpec` |
| `ComponentDefinition` | `ProviderDefinition` + `ClosedProviderCallback` |
| `FactoryComponent` | `Provider::Factory` |
| `FactoryParameterInjection` | `InjectionSpec` |
| `InterfaceBinding` | `Provider::Bound` |
| `REFLECT_METADATA_*` | 单一 `REFLECTED_PROVIDERS` |

迁移后，宏可以直接生成：

```rust
fn __nestrs_provider() -> Provider {
    Provider::Class { ... }
}

fn __nestrs_provider() -> Provider {
    Provider::Factory { ... }
}
```

linkme 只需要：

```rust
#[distributed_slice]
pub static REFLECTED_PROVIDERS: [fn() -> Provider] = [..];
```

因此：

```text
nestrs-core/src/metadata（已删除）
    ↓
nestrs-core/src/registration/provider.rs
```

开放泛型不会生成 `Provider::Generic`；它由 `ProviderDefinition::provider()` 在
已经单态化的注入点生成闭合 `Provider::Class`。

注意：`ServiceType`、`ServiceIdentifier`、`ServiceKey`、`Lifetime`、
`ServiceSource` 仍属于身份/注册公共模型，放在 `registration` 或 `provider`
中继续保留，不应随 metadata 删除。

### Q3：为什么要把注册意图和反射数据分开？

**A：**

因为两者职责不同：

```text
注册意图
    表达：我要注册什么、生命周期是什么、是否 primary、来源是什么

反射数据
    表达：怎么构造、有哪些依赖、泛型如何闭合、trait 如何投影
```

当前 `Provider` 把共享声明属性收拢到 `ProviderCommon`，并把构造依赖收拢到
`InjectionSpec`；它不等同于通用类型反射。

典型混合点：

```rust
pub enum ServiceImplementation {
    Instance(Box<dyn Injectable>),
    ConstructFactory(Constructor),
}
```

`ConstructFactory(Constructor)` 把“注册方式”和“构造函数”放在了一起。

建议拆成：

```rust
pub struct ServiceDescriptor {
    pub identifier: ServiceIdentifier,
    pub lifetime: Lifetime,
    pub primary: bool,
    pub source: ServiceSource,
    pub kind: ServiceKind,
}
```

以及：

```rust
pub struct ComponentReflection {
    pub descriptor: ServiceDescriptor,
    pub dependencies: &'static [DependencyReflection],
    pub constructor: Constructor,
    pub generic: Option<GenericReflection>,
    pub cleanup: Option<CleanupReflection>,
}
```

这样：

- `ServiceDescriptor` 是稳定的注册记录
- `ComponentReflection` 是运行时可执行的反射记录
- 两者由同一个宏生成

---

### Q4：字段归属应该怎么划分？

**A：**

| 当前字段 | 建议归属 |
|---|---|
| `service_identifier` | `ServiceDescriptor` |
| `lifetime` | `ServiceDescriptor` |
| `primary` | `ServiceDescriptor` |
| `source` | `ServiceDescriptor` |
| `kind` | `ServiceDescriptor` |
| `constructor` | `ComponentReflection` |
| `field_injections` | `RegisteredProvider.dependencies` |
| `component_definition` | `RegisteredProvider.generic` |
| `prepare_input` | `DependencyReflection.prepare_input` |
| cleanup | `RegisteredProvider.cleanup` |
| bind 投影 | `BindingRegistration` |

### Q3A：反射层和注入语义层分别包含什么？

**A：**

```text
反射层
    TypeInfo
    TypeId
    name
    fields
    generic_definition
    generic_arguments
    interfaces

注入语义层
    ServiceDescriptor
    Lifetime
    primary
    key
    DependencyRequest
    FactoryInvoker
    Constructor
    BindingRegistration

执行层
    Activator
    ConstructionContext
    ErasedService
    Arena / ServiceScope
```

`Type::of::<T>()` 属于反射层。

`#[inject]`、`#[factory]`、`ComponentDefinition` 属于注入语义层。

两者可以有关联：

```text
反射层回答：这个类型是什么
注入语义层回答：这个 provider 怎么注册、怎么注入、怎么构造
```

但不应把它们合并成同一个“反射系统”。

### Q3B：为什么 factory 不能只通过类型反射接入？

**A：**

`#[injectable] struct UserService` 的 provider 是类型 `UserService`，因此可以通过：

```rust
impl Reflected / ComponentDefinition for UserService
```

直接把构造信息和类型绑定。

`#[factory] fn create_user_service(...) -> UserService` 的 provider 是函数，
不是 `UserService` 类型本身。

因此 factory 需要的是：

```text
FunctionProviderRegistration
    ├── ServiceDescriptor
    ├── FactoryParameterSpec[]
    └── FactoryInvoker
```

而不是：

```text
impl ComponentDefinition for 输出类型
```

正确模型是：

```rust
pub enum ProviderInvoker {
    Injectable(Constructor),
    Factory(FactoryInvoker),
    Instance(InstanceValue),
}
```

`Activator` 据此决定调用 injectable constructor 还是 factory adapter。

### Q3C：为什么 Java/C# 函数天然融入反射，而 Rust 不行？

**A：**

这是一个语言范式差异，不是实现缺陷。

Java 和 C# 中，绝大多数用户函数都定义在类/对象里：

```java
// Java
class Repositories {
    public static Repository<User> create() { ... }
}
```

```csharp
// C#
class Repositories {
    public static Repository<User> Create() { ... }
}
```

因此反射系统可以沿：

```text
Class / Type
    ↓
GetMethods()
    ↓
MethodInfo.Invoke()
```

从类型对象自然延伸到方法。

Rust 允许模块级自由函数：

```rust
fn create_repository() -> Repository<User> { ... }
```

自由函数：

- 不属于任何 struct
- 没有所属类型 `TypeId`
- 不能通过 `Reflect for T` 绑定
- 返回值类型只是运行结果，不是函数自身的身份

因此 factory 不能走：

```text
Type -> ComponentReflection
```

而必须走：

```text
Function -> FactoryRegistration
```

这也是为什么最终模型必须是：

```text
Provider-centric
而不是
Type-centric
```

---

### Q5：宏是否还要生成构造函数？

**A：**

需要，而且构造函数是反射系统能运行起来的核心。

反射系统解决：

```text
知道我有哪些 provider
知道每个 provider 需要什么
知道如何选择 provider
```

构造函数解决：

```text
真正创建一个具体类型的实例
```

例如：

```rust
fn __nestrs_construct(
    mut context: ConstructionContext,
) -> Result<ErasedService, ActivationError> {
    let repository = context.take::<Repository<User>>(InputPosition(0))?;

    Ok(ErasedService::new(
        UserService {
            repository,
        }
    ))
}
```

Rust 没有运行时反射构造能力，因此：

- struct 字段类型、private 字段、`#[value]` 表达式只能由宏在编译期处理
- 必须预生成构造 adapter
- 运行时通过函数指针调用它

所以：

```text
宏生成 ComponentReflection
    +
宏生成 Constructor
    =
Rust 版 Activator.CreateInstance
```

---

## 三、Java/Spring 式泛型注入可行性

### Q6：Rust 能否实现 Java/Spring 式泛型注入？

**A：**

可以实现“实用部分”，但不能达到 `ResolvableType` 的完整等价能力。

可实现的典型场景：

```rust
#[injectable]
struct Repository<T> { ... }

#[injectable]
struct UserService {
    #[inject]
    repository: Repository<User>,
}
```

当前 `Repository<User>` 与 `Repository<Order>`：

- 是不同 Rust 类型
- 拥有不同 `TypeId`
- 拥有不同 `ServiceIdentifier`
- 宏可以分别生成具体化回调

因此闭合泛型 DI 是可行的。

---

### Q7：分层来看，Rust 的泛型 DI 能做到什么程度？

**A：**

| 能力 | Rust 可行性 | 实现方式 |
|---|---|---|
| 普通具体类型注入 | ✅ | `TypeId` + `ServiceIdentifier` |
| `Repository<User>` 闭合注入 | ✅ | `TypeId<Repository<User>>` |
| 多个闭合泛型实例区分 | ✅ | `TypeId` 不同 |
| Open Generic 模板注册 | ✅ 有限 | `ProviderDefinition` + 注入点闭合 callback |
| 泛型 trait 绑定 | ✅ 有限 | 宏为闭合类型生成投影 |
| key / primary + 泛型 | ✅ | `ServiceIdentifier` |
| 嵌套泛型结构匹配 | ⚠️ 有限 | 宏生成的 `TypeReflection` |
| Java `? extends` / `? super` | ❌ | Rust 无运行时泛型变型 |
| 运行时从 TypeId 反推泛型参数 | ❌ | 无运行时类型结构 |
| 运行时 MakeGenericType | ❌ | 只能编译期闭合 |

---

### Q8：为什么 Rust 不能像 Java 一样运行时反推泛型？

**A：**

因为 `TypeId` 只是：

```text
一个不透明的类型身份 hash
```

它不能回答：

- 这个类型叫什么
- 泛型定义是什么
- 泛型参数是什么
- 有哪些字段
- 有哪些方法
- 实现了哪些 trait

`type_name` 也只是一个字符串，且：

- 格式不保证稳定
- 不是标准反射 API
- 不能可靠解析

因此 Rust 的泛型信息必须由宏在编译期写入注册表。

---

## 四、C# 风格 Type / Reflect / Activator

### Q9：可以基于 TypeId 封装一个 C# 风格的 Type 吗？

**A：**

可以。

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Type {
    type_id: TypeId,
}
```

`Type` 不保存元数据，元数据存放在全局注册表：

```rust
pub struct ReflectionRegistry {
    types: HashMap<TypeId, TypeInfo>,
    components: Vec<ComponentReflection>,
    bindings: Vec<BindingReflection>,
}
```

API 可以设计为：

```rust
let ty = Type::of::<UserService>();

ty.name();
ty.fields();
ty.generic_arguments();
ty.interfaces();
```

---

### Q10：这与 C# `System.Type` 的本质区别是什么？

**A：**

| 维度 | C# `System.Type` | Rust `Type` |
|---|---|---|
| 元数据来源 | CLR 运行时元数据 | 宏编译期生成 |
| 任意类型 | 运行时自动可用 | 只对注册/标注类型可用 |
| 泛型参数 | 运行时反推 | 宏编译期填充 |
| 构造实例 | `Activator.CreateInstance` | 调用宏生成的 Constructor |
| 字段读取 | 反射字段 | 不提供任意字段读写 |
| 跨库三方类型 | 自动可用 | 需要宏/derive 参与 |

---

### Q11：`Type::of::<T>()` 和 `Type::from_type_id()` 的区别是什么？

**A：**

```text
Type::of::<T>()
    = 编译期已知 T
    = 通过宏生成的 Reflect trait 获取 TypeInfo

Type::from_type_id(type_id)
    = 运行时才知道 TypeId
    = 通过 ReflectionRegistry 查表
```

对泛型：

```rust
Type::of::<Repository<User>>()
```

可以工作，因为调用点知道 `T = User`，Rust 可以单态化。

但：

```rust
Type::from_type_id(TypeId::of::<Repository<User>>())
```

只有当 `Repository<User>` 的元数据已经被注册时才能成功。

如果只存在一个 `GenericReflection`：

```text
Repository<T>
```

运行时无法从 TypeId 反推出 `User`。

---

### Q12：Activator 应该承担什么职责？

**A：**

`Activator` 是实例创建器：

```rust
pub struct Activator {
    registry: &'static ReflectionRegistry,
}

impl Activator {
    pub fn create<T: Reflected>(
        &self,
        context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        let component = <T as Reflected>::component();
        (component.constructor)(context)
    }

    pub fn create_by_type(
        &self,
        ty: Type,
        context: ConstructionContext,
    ) -> Result<ErasedService, ActivationError> {
        let component = self
            .registry
            .get(ty.type_id)
            .ok_or(ReflectionError::TypeNotRegistered)?;

        (component.constructor)(context)
    }
}
```

这对应 C#：

```text
Activator.CreateInstance(type, args)
```

区别在于 Rust 的 `Type` 只能创建“已注册且已有 Constructor”的类型。

---

### Q13：这套系统算动态反射吗？

**A：**

在 DI 使用层面，可以认为是：有限制的动态反射。

更准确地说：

```text
元数据 = 编译期静态生成
查询   = 运行时动态获取
执行   = 运行时动态 dispatch
```

因此可以表述为：

```text
运行时可见的静态反射
```

或：

```text
注册组件范围内的动态反射
```

它不等同于 C# 的完整动态反射：

- 不能反射任意未注册类型
- 不能从任意 TypeId 反推泛型参数
- 不能运行时读写任意字段
- 不能运行时生成新代码

---

## 五、懒构造

### Q14：图分析是否意味着启动时实例化所有服务？

**A：**

不是。

图分析和实例化是两件事：

```text
图分析
    启动时收集、选候选、检测循环、生成 DAG
    不创建实例

实例化
    第一次 get<T>() 或显式 prewarm 时才执行
```

启动阶段推荐流程：

```text
ReflectionRegistry::collect()
        ↓
Provider-first 注册与候选选择
        ↓
compile()
        ↓
CompiledProviderGraph
```

此时没有任何服务实例。

真正激活发生在：

```rust
provider.get::<UserController>()
```

---

### Q15：当前 Arena 是否是长期容器？

**A：**

不是。当前 `Arena` 只是稳定地址存储与析构顺序的低层构造 ABI。它可保存
已经构造的实例，但不公开服务选择、递归依赖解析或缓存生命周期策略；
旧的递归激活器也不再存在。因此它不是长期容器，更不是可直接调用的服务实例化
入口。

未来引入 `ServiceProvider` 等 Provider-first activation runtime 后，才应由它持有：

```rust
pub struct ServiceProvider {
    plan: CompiledProviderGraph,
    singleton_slots: HashMap<ServiceIdentifier, ReadySlot>,
    scoped_slots: HashMap<ScopeId, HashMap<ServiceIdentifier, ReadySlot>>,
}
```

然后：

```text
singleton：第一次请求构造并缓存
scoped：每个 scope 第一次请求构造
transient：每次请求构造
```

---

### Q16：字段级懒注入如何实现？

**A：**

需要单独设计 `LazyInject<T>`。

当前：

```rust
pub struct Inject<T: ?Sized> {
    ptr: NonNull<T>,
}
```

它指向“已经构造好的服务”。

字段级懒代理：

```rust
pub struct LazyInject<T: ?Sized> {
    identifier: ServiceIdentifier,
    provider: &'static ServiceProvider,
    cell: OnceLock<NonNull<T>>,
}
```

第一次 `Deref` 时调用：

```text
provider.get::<T>()
        ↓
查找 ComponentReflection
        ↓
Activator::create()
        ↓
Arena/Scope 存储
        ↓
缓存到 OnceLock
```

`LazyInject<T>` 需要单独处理：

- 生命周期
- 线程安全
- `dyn Trait` fat pointer
- `Option<LazyInject<T>>`
- 与 Arena/Scope 的析构顺序

---

## 六、ASP.NET Core 对照

### Q17：为什么 ASP.NET Core 不做全局依赖图？

**A：**

ASP.NET Core 实际上做了“图”，但它是懒构建的 CallSite 图：

```text
ServiceDescriptor
        ↓
CallSiteFactory
        ↓
ServiceCallSite
        ↓
CallSiteChain 循环检测
        ↓
缓存
```

它没有在启动时构建全部服务 DAG，原因包括：

1. C# 有运行时 `System.Type`，可以延迟反射构造函数
2. factory lambda 可能隐藏依赖
3. open generic 无法提前枚举所有闭合类型
4. 服务可以在启动过程中动态/条件注册
5. 全部图构建成本高
6. 未使用的服务不应导致启动失败

`ValidateOnBuild` 可以开启启动时校验：

```csharp
new ServiceProviderOptions
{
    ValidateOnBuild = true,
    ValidateScopes = true,
}
```

但默认关闭。

---

### Q18：封装受限反射后，能否照搬 ASP.NET Core 的大部分架构？

**A：**

可以。

映射关系：

| ASP.NET Core | Nestrs 建议 |
|---|---|
| ASP.NET Core 服务集合接口 | 未来 Provider registry |
| `ServiceDescriptor` | `Provider` |
| `ServiceLifetime` | `Lifetime` |
| `ServiceProvider` | `ServiceProvider` |
| `ServiceCallSite` | 未来的 provider 构建计划 |
| `CallSiteFactory` | `Compiler` / `GraphBuilder` |
| `CallSiteChain` | `ResolveChain` / `ActivationChain` |
| `CallSiteRuntimeResolver` | `Activator` |
| `Type` | `Type` + `ReflectionRegistry` |
| `ConstructorInfo.Invoke` | `Constructor(context)` |
| `Activator.CreateInstance` | `Activator::create` |
| `Type.MakeGenericType` | `ProviderDefinition::provider()` |
| `IServiceScope` | `ServiceScope` |
| `IServiceScopeFactory` | `ServiceScopeFactory` |
| `ValidateOnBuild` | `ServiceProviderOptions.validate_on_build` |
| `ValidateScopes` | `ServiceProviderOptions.validate_scopes` |

不能直接照搬的部分：

- `System.Type`
- `Activator.CreateInstance`
- `ConstructorInfo`
- 任意类型自动反射
- 运行时 MakeGenericType

这些在 Rust 中需要替换为：

- `Type + ReflectionRegistry`
- `Constructor`
- 宏生成的 `Reflect` impl
- 编译期 `component_definition::<T>()`

---

## 七、建议的统一架构

### Q19：最终架构是什么样的？

**A：**

```text
宏
  ├── Provider::{Class, Factory, Bound, Alias}（注册意图）
  ├── InjectionSpec[]（依赖声明）
  ├── Constructor（构造 adapter）
  ├── ProviderDefinition + ClosedProviderCallback（开放泛型闭合）
  └── typed bound projector（trait 投影）
          ↓
linkme
          ↓
REFLECTED_PROVIDERS
          ↓
Provider registry（未来）
          ↓
Compiler / GraphBuilder
          ↓
CompiledProviderGraph
          ↓
ServiceProvider
          ↓
Activator
          ↓
ServiceScope / Arena
```

---

### Q20：推荐的核心类型是什么？

**A：**

```rust
pub struct ServiceDescriptor {
    pub identifier: ServiceIdentifier,
    pub lifetime: Lifetime,
    pub primary: bool,
    pub source: ServiceSource,
    pub kind: ServiceKind,
}

pub struct DependencyReflection {
    pub position: InputPosition,
    pub identifier: ServiceIdentifier,
    pub optional: bool,
    pub label: Option<&'static str>,
    pub prepare_input: PrepareInput,
    pub component_definition: Option<ComponentDefinitionCallback>,
}

pub struct ComponentReflection {
    pub descriptor: ServiceDescriptor,
    pub dependencies: &'static [DependencyReflection],
    pub constructor: Constructor,
    pub generic: Option<GenericReflection>,
    pub cleanup: Option<CleanupReflection>,
}

pub struct BindingReflection {
    pub concrete: ServiceType,
    pub trait_type: ServiceType,
    pub prepare_required: PrepareInput,
    pub prepare_optional: PrepareInput,
}

pub struct Type {
    type_id: TypeId,
}

pub struct ReflectionRegistry {
    types: HashMap<TypeId, TypeInfo>,
    components: Vec<ComponentReflection>,
    bindings: Vec<BindingReflection>,
}

pub struct Activator {
    registry: &'static ReflectionRegistry,
}
```

---

### Q21：宏应该展开成什么？

**A：**

对普通 `#[injectable]`：

```text
impl Reflected for UserService
    ↓
ServiceDescriptor
ComponentReflection
Constructor
        ↓
linkme 注册函数
```

对泛型 `#[injectable]`：

```text
impl<T> Reflected for Repository<T>
    ↓
GenericReflection
ComponentReflection
具体化 callback
```

对 `#[factory]`：

```text
impl Reflected for UserService
    ↓
factory adapter / Constructor
```

---

## 八、设计原则

### Q22：反射系统应该遵守哪些硬边界？

**A：**

1. 反射数据必须是纯静态的
2. `TypeInfo` 不包含 `NonNull`、`ArenaServiceRef`、`Inject<T>`、scope 状态
3. `Type` 只保存 `TypeId`，不保存运行时实例
4. `ReflectionRegistry` 只保存元数据和函数指针
5. `ServiceProvider` / `Scope` / `Arena` 保存运行时状态
6. `TypeId` 只作为查询 key，不用于解析泛型结构
7. 泛型闭合必须由宏在编译期生成
8. 不依赖 `type_name` 作为可靠反射 API
9. 不读取任意 struct 的字段布局
10. 不设计“任意类型全量运行时反射”
11. 只反射“注册过的 component”
12. 默认懒构造，启动时只做图分析和校验

---

## 九、实施路线

### Q23：建议按什么顺序实施？

**A：**

**第一阶段：拆分注册意图与反射**

- 定义 `ServiceDescriptor` 为纯注册意图
- 引入 `ComponentReflection`
- 移除 `ServiceImplementation::ConstructFactory(Constructor)`
- 将 `FieldInjection` 改造为 `DependencyReflection`

**第二阶段：建立 ReflectionRegistry**

- 定义 `TypeInfo`
- 定义 `Type`
- 定义 `ReflectionRegistry`
- 定义 `Reflect` trait
- 使用 `OnceLock` 实现全局只读注册表
- 增加 ABI 版本字段

**第三阶段：建立 Activator**

- 定义 `Activator`
- 封装 Constructor 调用
- 提供 `create<T>()`
- 提供 `create_by_type(Type)`
- 与 `ErasedService` / `Arena` 对接

**第四阶段：建立 Compiler**

- 统一收集 injectable / factory / bind
- 候选选择
- primary / key 规则
- 缺失依赖诊断
- 二义性诊断
- 循环检测
- 泛型闭合

**第五阶段：建立 ServiceProvider / Scope**

- `ServiceProvider`
- `ServiceScope`
- `ServiceScopeFactory`
- singleton / scoped / transient slot
- 懒构造
- 可选 `prewarm()`

**第六阶段：接入异步与并发**

- async factory
- ActivationFuture
- activation frame
- ready queue
- single-flight
- 失败回滚
- cleanup

**第七阶段：扩展高级能力**

- `LazyInject<T>`
- `IEnumerable<T>` 风格集合注入
- decorator
- 可选/条件注册
- 插件发现

---

## 十、待确认问题

### Q24：还有哪些设计点需要决策？

**A：**

1. `ReflectionRegistry` 使用全局 `OnceLock`，还是由未来 Provider registry 持有？
2. 未来 Provider registry 是否允许运行时手动添加 provider？
3. 是否支持 `resolve_by_type(TypeId)`，还是只暴露 `resolve::<T>()`？
4. 泛型注册采用“宏闭合回调”还是“泛型 family + 运行时匹配”？
5. `LazyInject<T>` 是独立类型，还是扩展 `Inject<T>`？
6. 是否加入 ABI 版本校验？
7. 是否支持 `no_std`？
8. `Err` 大小和诊断格式是否重新设计？
9. 是否需要完整 AST 注册发现，还是仅 linkme？
10. `type_name` 是否仅用于诊断，不用于匹配？

---

## 十一、总结

### Q25：一句话总结最终架构？

**A：**

```text
宏生成注册意图和静态反射
linkme 负责发现
ReflectionRegistry 负责查询
Compiler 负责依赖图分析和校验
Activator 负责调用预编译 Constructor
ServiceProvider / Scope / Arena 负责生命周期和实例存储
```

最终目标不是完整模拟 C# 动态反射，而是：

```text
C# 风格的 Type / Activator API
    +
Rust 宏驱动静态反射
    +
注册 component 范围内的运行时反射
    +
默认懒构造
```

这套设计可以很好地对齐 ASP.NET Core DI 的架构，同时保留 Rust 编译期类型安全和静态元数据优势。
