# Nestrs Core DI：当前实现与设计要求

> 状态：本文描述当前已落地的 v0–v6 Core DI 运行时。它面向两个读者：使用
> Nestrs 构建应用的开发者，以及维护宏与运行时边界的框架贡献者。
>
> 本文回答“现在怎样使用、为什么 API 要这样收口、运行时怎样保证生命周期安全”。
> 更细的版本路线、测试矩阵与内部激活细节见
> [NESTRS_ACTIVATION_RUNTIME_PLAN.md](./NESTRS_ACTIVATION_RUNTIME_PLAN.md)。

## 1. 设计结论

普通应用的 DI 体验应当只有三件事：

1. 用属性宏声明服务和依赖。
2. 从一个类型化 Root 构建容器。
3. 读取已构建的 Root，或只读查询已经构建好的 concrete 服务。

~~~rust
let provider = ServiceProvider::<AppRoot>::build()?;
let root = provider.root();
let users = provider.get::<UserService>();
~~~

这不是动态 service locator。容器不会在 get 时查找注册、物化泛型、创建服务或缓存新实例。
应用开发者不需要接触 Registry、Compiler、Arena、Provider 元数据或手写 linkme 注册。

根路径的普通 API 只有：

| 类型 | 用途 |
| --- | --- |
| ServiceProvider<Root> | 构建 Root 定向的 Singleton 容器、读取已提交服务、显式 shutdown |
| BuildError | 不透明的高层构建错误；通过 Display 与 Debug 提供带 provider 声明上下文的诊断 |

需要预声明静态 Scope 链时，应用才显式导入 nestrs_core::scope 中的高级 API。宏展开所需的
nestrs_core::__private 是隐藏 ABI，不是应用开发者应手写依赖的接口。

## 2. 模块边界与架构

下图把“应用可使用的入口”与“Core 内部如何实现”分开。图中 Registry、Compiler、Arena
与 Tokio 调度器是实现说明，不是根 API。

~~~mermaid
flowchart TB
    subgraph application ["应用 crate"]
        declaration["服务声明: #[injectable] / #[factory] / #[bind]"]
        rootApi["普通入口: ServiceProvider 与 BuildError"]
        scopeApi["高级入口: nestrs_core::scope"]
    end

    macro["nestrs-macro 属性宏"]

    subgraph core ["nestrs-core"]
        privateAbi["__private 隐藏宏 ABI"]
        slices["linkme Provider 与 TraitBinding 分布式切片"]
        registry["私有 ProviderRegistry"]
        compiler["私有 ActivationCompiler"]
        compiledPlan["不可变编译图与 Scope 计划"]
        runtime["私有激活运行时: 顺序激活或 Tokio JoinSet"]
        arena["私有 Arena: Singleton Scoped Transient 所有权"]
    end

    declaration --> macro
    macro -.->|"宏展开使用 ABI"| privateAbi
    macro -->|"生成 linkme 注册"| slices
    slices --> registry --> compiler --> compiledPlan --> runtime --> arena

    rootApi -->|"build / build_async"| compiler
    scopeApi -->|"静态链 build / create scope"| compiler
    rootApi -.->|"root / get 只读已提交服务"| arena
    scopeApi -.->|"get 只读当前、祖先或 Singleton 服务"| arena
~~~

这个边界有三个关键含义：

- 应用代码写的是服务类型和属性，不是 Provider、Registry 或 Arena。
- 宏可以通过隐藏 ABI 生成注册与构造适配代码，但该 ABI 不构成手写扩展点，也不应出现在入门示例。
- nestrs-core 不依赖 nestrs-bootstrap；Core 自己完成静态图编译、生命周期校验、激活和析构。

## 3. 普通应用：五分钟开始

默认示例位于
[example/nestrs-base/src/main.rs](./example/nestrs-base/src/main.rs)。下面是相同的完整最小程序：

~~~rust
use nestrs_core::ServiceProvider;
use nestrs_macro::injectable;

#[injectable]
struct UserRepository;

impl UserRepository {
    fn display_name(&self) -> &'static str {
        "Ada Lovelace"
    }
}

#[injectable]
struct UserService {
    #[inject]
    repository: UserRepository,
}

impl UserService {
    fn greeting(&self) -> String {
        format!("Hello, {}!", self.repository.display_name())
    }
}

#[injectable]
struct Application {
    #[inject]
    users: UserService,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::<Application>::build()?;

    println!("{}", provider.root().users.greeting());

    let users = provider
        .get::<UserService>()
        .expect("Application 的可达闭包已包含 UserService");
    assert_eq!(users.greeting(), "Hello, Ada Lovelace!");

    Ok(())
}
~~~

### 3.1 字段声明规则

在 source 中，依赖字段写成其业务类型，并标注 #[inject]。宏会生成运行时所需的内部适配；
应用代码不应手写 Inject<T>。

没有 #[inject] 的字段不是自动注入字段。它应由普通 Rust 初始化、Default 或 #[value(...)]
等非 DI 策略提供值。这样依赖边是显式的，Compiler 才能在构建前检查整个可达图。

对于 #[factory]，函数参数默认都是编译图中的注入参数；当需要 keyed 服务时，用 #[inject(key = ...)]
标明请求的 key。factory 参数同样不是运行时 resolver。

### 3.2 ServiceProvider 的合同

| 操作 | 合同 |
| --- | --- |
| build() | 收集当前链接单元的注册，编译 Root 可达闭包，并同步、依赖后序地激活它 |
| build_async().await | 使用当前 Tokio runtime 激活闭包；允许 async factory 与 cleanup |
| root() | 返回构建时指定的默认 key concrete Root |
| get<T>() | 只读查询已提交的默认 key concrete 持久服务 |
| shutdown(self).await | 消费 provider，并按逆提交顺序执行已声明的 cleanup 与 Rust 析构 |

Root 必须是一个已显式注册、默认 key 的 Singleton。每次 build 都会创建一套独立的 Arena 和
Singleton 实例集；Singleton 不是进程全局单例。闭合泛型的 fallback 只能从已经选中的 provider
依赖中触发，不能从 Root 类型反推并物化一个泛型 family。

### 3.3 get<T>() 的严格边界

get<T>() 的类型约束是 Send + Sync + 'static，故应用不需要也不会看到 Injectable 约束。
它只查默认 key 的 concrete、持久且已提交的实例：

| 查询目标 | 结果 |
| --- | --- |
| Root 可达闭包中的默认 key concrete Singleton | Some(&T) |
| 未在 Root 可达闭包中的服务 | None |
| keyed provider | None |
| trait 请求或 trait binding | None |
| Transient | None |
| 尚未创建的 Scoped 服务 | None |

因此，root() 是应用的主要类型化入口；get<T>() 只适合少量已知、已构建 concrete 服务的
只读观察、集成桥接或启动断言。不要用它替代字段注入，也不要把它包装成动态 resolve API。

## 4. 高级能力：静态多层 Scope

Scope 是有意从根 API 移出的高级能力。它不是在运行时任选一个类型来 resolve，而是在类型中
预先声明一条线性的 Scope 链。

~~~rust
use nestrs_core::scope::{ScopeLayer, ScopeProvider};
use nestrs_macro::injectable;

#[injectable]
struct AppConfig;

#[injectable]
struct AppRoot {
    #[inject]
    config: AppConfig,
}

#[injectable(lifetime = "scoped")]
struct RequestRoot {
    #[inject]
    config: AppConfig,
}

#[injectable(lifetime = "scoped")]
struct TransactionRoot {
    #[inject]
    request: RequestRoot,
}

type AppScopes = ScopeLayer<RequestRoot, ScopeLayer<TransactionRoot>>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ScopeProvider::<AppRoot, AppScopes>::build()?;
    assert!(provider.get::<AppConfig>().is_some());

    let request = provider.create_scope()?;
    assert!(request.get::<RequestRoot>().is_some());
    assert!(request.get::<TransactionRoot>().is_none());

    let transaction = request.create_child_scope()?;
    assert!(transaction.get::<AppConfig>().is_some());
    assert!(transaction.get::<RequestRoot>().is_some());
    assert!(transaction.get::<TransactionRoot>().is_some());

    Ok(())
}
~~~

ScopeLayer<Root, Child = ScopeEnd> 描述固定的下一层 Root。没有 create_child_scope::<T>()，
因此 API 不会退化为 locator。Compiler 会在 provider 构建时一次性编译 AppRoot 与整条静态链；
每次 create_scope 或 create_child_scope 只 eager 激活该层尚未拥有的 Scoped 闭包。

可见性也有明确边界：

- ScopeProvider::get<T>() 只返回已提交的 Singleton。
- Scope::get<T>() 依次读取当前层、精确祖先层、再读取 Singleton Arena。
- 查询不会返回 keyed、trait 或 Transient 服务，也不会创建后代 scope。
- child scope 借用 parent，因此必须先结束或 shutdown child，再结束 parent，最后结束
  ScopeProvider。

同一个 parent scope 可以创建多个 sibling child；它们复用祖先 Scoped 实例，但各自创建本层
首次拥有的 Scoped 实例。

## 5. 异步构建、Tokio 与 cleanup

同步入口刻意保持简单：可达图里若有 async factory 或 cleanup，build()、ScopeProvider::build()
以及对应的同步建 scope 入口都会返回 BuildError。

异步入口需要一个活动 Tokio runtime。下面的例子使用 current-thread runtime；它可以安全并发推进
future，但不会物理并行执行多个 worker。

~~~rust
use nestrs_core::ServiceProvider;
use nestrs_macro::{factory, injectable};

struct StartupState;

#[factory]
async fn startup_state() -> StartupState {
    StartupState
}

async fn cleanup_application() {
    // 释放异步资源，例如通知外部系统。
}

#[injectable(cleanup = "cleanup_application")]
struct AppRoot {
    #[inject]
    startup: StartupState,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tokio::runtime::Builder::new_current_thread()
        .build()?
        .block_on(async {
            let provider = ServiceProvider::<AppRoot>::build_async().await?;
            assert!(provider.get::<StartupState>().is_some());

            provider.shutdown().await;
            Ok::<(), Box<dyn std::error::Error>>(())
        })
}
~~~

若希望独立就绪节点有机会物理并行，调用方可以在 multi-thread runtime 内调用同一套 API，例如
new_multi_thread().worker_threads(2)。这只意味着运行时可以把互不依赖的工作分配给 worker；
不保证固定的 worker 分配、兄弟节点完成顺序或同步阻塞构造会自动加速。

异步设计要求如下：

- build_async、ScopeProvider::build_async、create_scope_async 与 create_child_scope_async
  都要求当前线程存在 Tokio runtime；缺失时返回 BuildError。
- current-thread runtime 合法，但只提供同线程并发；multi-thread runtime 才可能物理并行。
- class、同步 factory 与 async factory 都可能由 Tokio worker 调度；消费者只会在所有直接依赖
  成功提交后启动。
- ServiceProvider、ScopeProvider 与 Scope 仍不承诺 Send 或 Sync。不要把容器或 scope 移入
  detached tokio::spawn。
- 完整的多层 Scope、factory、trait binding、key、optional、泛型与 cleanup 组合案例见
  [example/nestrs-base/src/bin/advanced_scope.rs](./example/nestrs-base/src/bin/advanced_scope.rs)。

## 6. 从注册到 shutdown 的激活流程

~~~mermaid
flowchart TD
    start(["调用 build、build_async 或 Scope 对应入口"])
    collect["从 linkme 收集 Provider 与 TraitBinding"]
    compile["编译 Root 可达闭包并校验选择与生命周期"]
    mode{"同步入口？"}
    syncCheck{"可达图含 async factory 或 cleanup？"}
    syncActivate["依赖后序顺序激活"]
    runtime["Handle::try_current 要求活动 Tokio runtime"]
    schedule["协调器准备输入与 ready queue"]
    worker["JoinSet worker 执行已准备的 class 或 factory"]
    commit["协调器提交结果并解锁消费者"]
    more{"当前 phase 还有就绪节点？"}
    ready["返回已构建的 provider 或 scope"]
    read["root() 与 get<T>() 只读已提交实例"]
    shutdown["消费式 shutdown().await"]
    leases["等待关联 Arena lease 清空"]
    journal["按逆提交顺序: hook → Rust Drop → transient child shutdown"]
    failure["返回 BuildError，并只做普通 Rust Drop 回滚"]

    start --> collect --> compile --> mode
    mode -->|"是"| syncCheck
    syncCheck -->|"有"| failure
    syncCheck -->|"无"| syncActivate
    syncActivate -->|"成功"| ready
    syncActivate -->|"失败"| failure

    mode -->|"否"| runtime
    runtime -->|"无 runtime"| failure
    runtime -->|"有 runtime"| schedule --> worker --> commit --> more
    more -->|"有"| schedule
    more -->|"无"| ready
    worker -->|"worker 或构造失败"| failure
    commit -->|"提交失败"| failure

    ready --> read --> shutdown --> leases --> journal
~~~

异步路径中，协调器独占输入准备、结果提交、Transient 所有权、失败排序与 rollback。worker 只运行
已经准备好的 constructor 或 factory，并借助 Arena lease 保活它可见的服务存储。这样不会把
Arena façade 或裸 Inject 指针跨 await、跨线程交给 worker。

激活失败时，协调器停止调度新工作，收口已经启动的 worker，并按稳定编译图顺序选择一个
BuildError。factory 返回的用户 Err(E) 会保留既有 FactoryFailed 语义，不会作为用户错误值
从 BuildError 向外泄漏。

## 7. 编译器解决什么问题

注册由宏借助 linkme 在链接期收集。ProviderRegistry 按稳定 source 顺序收集原始 Class、Factory
与 TraitBinding；ActivationCompiler 只从请求的 Root（以及静态 Scope 链）开始遍历可达闭包。
无关注册不会阻止某个独立 Root 构建成功。

| 依赖请求 | 编译规则 |
| --- | --- |
| 默认 key concrete 类型 | 唯一候选直接选择；多个候选时仅唯一 primary 可消歧，否则报错 |
| Option 直接依赖 | 没有候选时写入 absent 输入并得到 None；歧义仍是错误 |
| trait 请求 | 通过 #[bind] 派生 concrete token，并保留类型化 trait projector |
| keyed 请求 | key 是 token 身份的一部分；消费者必须显式请求相同 key |
| 闭合泛型依赖 | 显式 closed provider 优先；没有候选才调用 ProviderDefinition fallback，结果按 token 缓存 |

上述行为发生在 build 阶段，而不是第一次解引用字段时。循环、必选依赖缺失、歧义、生命周期反转
以及不支持的能力组合都应尽早变成 BuildError。

## 8. 生命周期、可见性与 Transient 所有权

| 生命周期 | 实例创建频率 | 可作为公开 Root | 可由 get 查询 | 所有者 |
| --- | --- | --- | --- | --- |
| Singleton | 每次 provider build 的可达闭包中一份 | AppRoot | 可以，前提是默认 key concrete 且已可达 | Provider 的 Singleton Arena |
| Scoped | 每个对应 Scope 实例一份 | 仅 ScopeLayer 的 Root | 仅 Scope 当前层或精确祖先层可见 | 当前 Scope Arena |
| Transient | 每条静态注入边一份 | 不可以 | 不可以 | 直接消费者的 child Arena |

Transient 的语义是“每个注入边创建一个实例”，不是每次解引用时创建。同一 consumer 的两个
Inject<T>，以及两个不同 consumer 注入同一个 T，都会各自拥有不同的 transient occurrence。

持久消费者必须遵守以下方向：

| 消费者 | 可以注入 | 不可以注入 |
| --- | --- | --- |
| Singleton | Singleton、由它拥有的 Transient | Scoped，以及经由 Transient 间接捕获的 Scoped |
| Scoped | Singleton、当前层或祖先 Scoped、由它拥有的 Transient | 已声明为后代 ScopeRoot 的 Scoped |
| Transient | 继承其持久消费者的有效 owner | 任何会让其持久 owner 捕获后代 Scoped 的路径 |

factory 参数的 Transient 只存活在这次 factory activation frame 内。frame、context 与 future 销毁后，
临时 transient 子树才进行 Rust Drop；如果该临时子树任何 provider 配置 cleanup，编译器会拒绝
它，而不会静默跳过 cleanup。

## 9. cleanup、失败、取消与析构

cleanup hook 是零参数异步 hook，只由显式、消费式 shutdown().await 保证运行。它不是普通
Drop 的替代品。

| 事件 | 是否驱动 cleanup hook | 服务如何释放 |
| --- | --- | --- |
| 成功调用 shutdown().await | 是 | 逆 commit 顺序：当前 hook → 当前服务 Rust Drop → transient child shutdown |
| 普通 Drop | 否 | 仅 Rust Drop |
| 构建或建 scope 失败 | 否 | 已提交及暂存结果按 rollback 做 Rust Drop |
| build_async 或 create_scope_async 被取消 | 否 | 停止调度、abort 在飞 worker；lease 保活后再安全 Rust Drop |
| shutdown future 被取消 | 当前与后续 hook 不保证完成 | Arena 仍会安全析构全部服务 |

scope 的 shutdown 不会级联清理 parent Arena。带 cleanup 的多层 Scope 必须按 child → parent →
ScopeProvider 的顺序 shutdown。独立兄弟节点的析构相对顺序不构成公共合同；依赖关系保证
消费者先于其依赖清理。

## 10. 公开错误与诊断

BuildError 是无公开 variant、无公开底层 source 的错误类型。应用应记录或展示它的 Display /
Debug 文本，其中包含 provider 声明上下文；不要匹配 Compiler、Arena、Activation 或 Tokio
内部错误。

这个约束让后续可以调整图编译、Arena 存储或 worker 策略，而不要求普通应用依赖内部枚举的
稳定性。

## 11. 当前明确不支持的能力

以下限制是刻意的设计边界，而不是遗漏的公开入口：

- 动态 resolve、任意 token 查询、keyed 或 trait 的 get 查询。
- 动态注册、运行时任选 ScopeRoot、分支型 Scope 树或嵌套动态注册。
- Transient Root、lazy Provider<T>、每次解引用重新创建 Transient。
- 将 ServiceProvider、ScopeProvider 或 Scope 作为 Send / Sync 容器跨线程移动。
- tokio::spawn_blocking 自动包装同步 constructor 或 factory。
- nestrs-bootstrap 参与 Core 的 Registry、Compiler、Arena 或生命周期实现。
- cleanup 重试、超时、后台补偿与动态解析。

这些边界防止普通 API 从“声明服务、构建容器、读取实例”膨胀为底层运行时控制面。

## 12. 对应用与框架贡献者的设计要求

### 应用开发者

- 默认从 ServiceProvider 与字段 #[inject] 开始；能用 Root 类型化入口时，不引入 Scope。
- 仅在请求、事务、命令等确实需要分层生命周期时，显式选择 nestrs_core::scope。
- 把 get<T>() 当成只读的已构建查询，而不是替代注入或动态服务查找。
- 需要 async factory 或 cleanup 时，在活动 Tokio runtime 内使用异步入口，并显式 shutdown。

### 框架贡献者

- 不要重新从根路径公开 Provider、Registry、Compiler、Arena、registration、lifetime 或低层错误。
- 宏生成代码只能依赖 nestrs_core::__private 中的既定 ABI；普通示例和业务代码不应手写该路径。
- 新增能力必须保持 Inject<T>、Option<Inject<T>>、FactoryParameter frame 与 typed trait projector
  的 ABI 合同，除非经过单独的破坏性变更设计。
- 新增生命周期或调度能力前，先明确其 owner、可见性、rollback、取消与 shutdown 语义，并为
  无关注册隔离、失败、取消和逆序析构增加覆盖。
- 不要因为 Tokio worker 可以并行执行，就承诺容器可 Send、固定 worker 亲和性或兄弟节点顺序。

## 13. 延伸阅读与可运行案例

- [默认同步案例](./example/nestrs-base/src/main.rs)
- [Tokio、多层 Scope 与 cleanup 高级案例](./example/nestrs-base/src/bin/advanced_scope.rs)
- [激活运行时路线与验证矩阵](./NESTRS_ACTIVATION_RUNTIME_PLAN.md)
- [Provider、Registry 与 Compiler 设计问答](./NESTRS_REFLECTION_DESIGN_QA.md)
- [构造 ABI、ConstructionContext 与 Inject 决策](./NESTRS_DI_QA_HANDOFF.md)
