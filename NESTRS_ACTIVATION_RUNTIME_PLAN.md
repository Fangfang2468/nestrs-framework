# Nestrs 激活运行时：异步执行与并行实例化方案

> 状态：设计计划（activation runtime 尚未实现）
>
> 相关文档：
>
> - `AGENTS.md`：项目架构共识与 Git/Commit 规范
> - `NESTRS_REFLECTION_DESIGN_QA.md`：Provider 模型与实施路线（含 Provider registry、
>   Compiler、Activator、ServiceProvider/Scope 的六个阶段）
> - `NESTRS_DI_QA_HANDOFF.md`：构造 ABI、`ConstructionContext` 与 `Inject<T>` 的早期决策
>
> 适用范围：`nestrs-core` 的构造/激活 ABI 与未来的 activation runtime
> （`nestrs-bootstrap` 或独立的 `nestrs-runtime`）。

---

## 一、目标

在不牺牲「factory 参数不可逃逸」这条编译期防线的前提下，让服务实例化满足：

1. **可运行在 tokio 上**：用户写的 `async fn` factory 可以直接 `await` tokio API；
2. **同一会话内并发**：多个互相独立的构造可以重叠等待（I/O 并行）；
3. **按依赖图并行**：在需要时把无依赖关系的节点分派到多个 worker 线程真正并行。

非目标（本阶段不做）：把 activation runtime 绑定到某个执行器、把租约/引用计数引入
`Inject<T>`、实现 scope/transient 语义（那属于 ServiceProvider/Scope 阶段）。

---

## 二、现状：本方案依据的代码事实

| 事实 | 位置 |
| --- | --- |
| `FactoryFuture<'frame> = Pin<Box<dyn Future<Output = Result<ErasedService, ActivationError>> + Send + 'frame>>` | `nestrs-core/src/registration/provider.rs` |
| `CleanupFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>` | 同上 |
| `FactoryConstructor` / `AsyncConstructor` 均为 `for<'frame> fn(FactoryConstructionContext<'frame>) -> …` | 同上 |
| `FactoryActivationFrame::new()`、`FactoryInvoker::invoke` 均 `pub(crate)`，外部不可构造 frame | 同上 |
| `Inject<T, Access>` 手写 `unsafe impl Send/Sync`；`Inject<String>: Send + Sync` 已验证 | `nestrs-core/src/inject_wrapper.rs` |
| `Arena` 持有 `NonNull<u8>` 分配，**实测 `!Send + !Sync`** | `nestrs-core/src/arena.rs` |
| `Arena` 只增不改：没有删除或替换已提交服务的 API | 同上 |
| 析构按**提交的逆序**执行，因此「依赖先提交、消费者后提交」必须成立 | 同上 |
| `nestrs-core` 声明了 `tokio` 依赖但当前未使用 | `nestrs-core/Cargo.toml` |

> 「实测」指用一个编译探针验证 `assert_send::<Arena>()` / `assert_sync::<Arena>()` 会因
> `NonNull<u8>` 失败，而 `Inject<String>` 两者都满足。

---

## 三、冲突的准确定位

表面说法「`Inject` + `Arena` 与 tokio 冲突」并不准确。实际是三个独立约束：

| 约束 | 现状 | 与 tokio 的关系 |
| --- | --- | --- |
| (a) `Inject<T>` 指向 Arena 内存 | 裸指针 + 借用周期保证 | **无关**：token 本身已是 `Send + Sync` |
| (b) `FactoryFuture<'frame>` 借用 activation frame | 非 `'static` | 只与 **detached spawn** 冲突 |
| (c) `Arena` 含 `NonNull` | `!Send + !Sync` | 只与「跨 await / 跨任务持有」冲突 |

两条必须记住的结论：

1. **移动只发生在 await 处**。一个 poll 期间 future 是 pinned、不会被移动；执行器只能在
   future 返回 `Pending` 之后把它搬到别的线程。因此「跨 await 的状态必须是 `Send`」，
   而「`&mut Arena` 不能跨 await」正是这条规则的推论。
2. **并发 ≠ 并行**。同一任务内 poll 多个 future 是并发（重叠等待）；把 future 交给
   `tokio::spawn` 才是并行（多核同时推进）。

---

## 四、一次激活的三段式形状

```text
poll #1 ┌─ 同步段：借 &Arena 准备输入（ConstructionContext + tokens）
        ├─ await 段：invoker.invoke(context, &frame).await     ← async factory 真正在这里跑
        └─ 同步段：借 &mut Arena 提交 ErasedService
```

规则（贯穿所有方案）：

- **`&mut Arena` / `&Arena` 不跨 await**；跨 await 的只有 `ConstructionContext`、
  `Inject<T>` token 与 `&FactoryActivationFrame`，它们都是 `Send`/`Sync`。
- 提交顺序仍须满足拓扑约束：依赖先提交，消费者后提交，逆序析构才成立。

---

## 五、三个方案

### 方案 ①：同任务并发（v1 默认）

在一个任务内用 `FuturesUnordered` / `join!` / `select!` 同时推进多个激活，提交在
两次 poll 之间完成。

- 不需要 `'static`，不需要引用计数，不改动任何现有类型；
- 多线程 tokio runtime 亦可（激活 future 是 `Send`，只是不 spawn）；
- 收益：重叠 I/O 等待；上限：没有多核并行。

### 方案 ②：结构化并行（推荐的真并行方案）

**关键技巧：把 frame 的创建挪进 worker 任务内部**，future 就不再借用外部作用域：

```rust
tokio::spawn(async move {
    let frame = FactoryActivationFrame::new();               // 任务内部创建
    let context = FactoryConstructionContext::from_bound(context, &frame);
    constructor(context).await                               // 结果回传会话任务
})
```

于是该 future 是 `Send + 'static`，可以真正 `spawn`；而用户 factory 拿到的仍是
`Inject<T, FactoryParameter<'local>>`，**逃逸防线完整保留**（4 个
`factory-parameter-escape` UI 用例继续有效）。

需要收口的两条不变量：

1. **结构化并发**：所有 worker 必须在会话使用或析构 `Arena` 之前 join 完成；
   建议用 RAII guard（Drop 中 join）或显式 `join_all` 收口，并在 core 内部保留
   **唯一一处 `unsafe` 边界**，附完整 safety 论证。
2. **Arena 只增不改**：提交只新增分配，不删除、不替换已提交服务（现有设计已满足）。
   依赖先于消费者提交，worker 只解引用已提交的依赖，因此 worker 的读取与会话任务的
   提交之间不存在别名冲突。

代价：需要自制 scoped spawn（tokio 没有稳定版作用域任务），并承担一处 `unsafe`
与 join 纪律的维护成本。

### 方案 ③：租约化（真并行的"最灵活"形态）

让 token 从「裸指针 + 编译期借用」升级为「指针 + 保活句柄（Arc/lease）」，future 变
`'static`，可自由 `spawn`、可后台化。

- 代价一：factory 参数逃逸从**编译期禁止**退化为**运行期保活**，上述 4 个 UI 用例需要
  改写语义；
- 代价二：析构顺序从 Arena 逆序变为引用计数驱动，与现有 `Arena` 的释放模型需要重新协调；
- 收益：激活可以脱离会话任务存活（后台预热、跨请求复用）。

### 对比

| 方案 | 真并行 | 需要租约/Arc | 逃逸防线 | 主要代价 |
| --- | --- | --- | --- | --- |
| ① 同任务并发 | ✗ | 不需要 | 完整 | 仅重叠等待，无多核收益 |
| ② 结构化并行 | ✓ | 不需要 | 完整 | core 内一处 `unsafe` + join 纪律 |
| ③ 租约化 | ✓ | 需要 | 降级为运行期 | 防线弱化、析构模型重做 |

---

## 六、推荐路线

```text
v1  方案 ①：同任务并发
    写 resolver/activator 时直接采用；先测量真实负载，不预先引入 spawn。

v2  方案 ②：结构化并行
    当出现「并发激活受 CPU 或阻塞型构造限制」的真实需求时实现；
    按依赖图宽度分派无依赖节点，会话任务负责 prepare / spawn / join / commit。

v3  方案 ③：租约化
    仅在需要「激活跨会话存活」或「后台预热」时评估；届时应作为独立的架构决策
    （与 NESTRS_DI_QA_HANDOFF.md Q16 的 lease 讨论合并决策）。
```

执行器归属：

- `nestrs-core` 保持零执行器依赖（`std::future` + frame）；
- tokio 由 `nestrs-bootstrap`（或 `nestrs-runtime`）引入，负责持有 runtime/handle、
  single-flight 的同步原语、激活超时与 cleanup 的 spawn；
- `CleanupFuture` 已是 `'static + Send`，可直接 `tokio::spawn`；
- `nestrs-core/Cargo.toml` 中当前未使用的 `tokio` 依赖应移除，待 runtime crate 落地时
  再在需要它的 crate 中声明。

---

## 七、测试计划（实现 v2 时）

| 目标 | 测试形态 |
| --- | --- |
| 真并行 | 两个无依赖的 async factory 各 `sleep(d1)`/`sleep(d2)`，断言总耗时接近 `max(d1, d2)` 而非 `d1 + d2` |
| 析构顺序 | 并行提交后仍满足「消费者先于依赖析构」 |
| 逃逸防线 | 现有 4 个 `factory-parameter-escape` UI 用例保持编译失败 |
| 失败回滚 | 某个 worker 返回 `Err` 时会话不提交该节点，也不泄漏已构造的中间结果 |
| 提交顺序 | 依赖未就绪时消费者不得提交（拓扑约束由调度器保证） |
| single-flight（若实现） | 同一 token 的并发请求只触发一次激活 |

---

## 八、不可违反的约束（Invariants）

1. `&mut Arena` / `&Arena` 绝不跨 await；跨 await 的状态必须 `Send`。
2. worker 的 frame 由 worker 自己创建；`FactoryActivationFrame::new()` 不得对外暴露。
3. 所有 worker 必须在 Arena 使用/析构前 join；`unsafe` 边界只允许存在于 core 内部。
4. Arena 保持「只增不改」；一旦引入删除/替换 API，方案 ② 的并发前提立即失效。
5. 提交顺序必须满足依赖先于消费者。
6. `Inject<T, FactoryParameter<'frame>>` 不得获得 `'static` 形态；任何为并行而让 token
   变成 `'static` 的做法都必须同时给出新的逃逸防护方案（方案 ③ 的代价）。

---

## 九、待决问题

1. scoped spawn 的实现形态：自研 RAII guard，还是评估 `async-scoped` 之类的 crate？
2. 并行度上限与调度策略：按图宽度限流？是否需要 `Semaphore` 控制同时激活数？
3. 阻塞型构造（同步 factory 内的重计算）是否需要 `spawn_blocking` 通道？
4. single-flight 的共享进行中状态用 `tokio::sync::OnceCell` / `watch` 还是自研？
5. 是否给运行时提供「预激活（prewarm）」入口，它是否要求方案 ③？
