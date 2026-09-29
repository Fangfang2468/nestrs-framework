# 跨 crate 使用依赖注入

Nestrs 的服务可以拆分到多个 Rust crate。接口、实现和使用接口的业务服务不必放在
同一个 crate；具体实现也可以保持私有。仍然使用 `#[injectable]`、`#[factory]`、
普通 `impl` 和查询宏，不需要为跨 crate 增加 `bind` 或注册调用。

整个应用通过同一套 `cargo nestrs` 工具构建，使用同一个 `nestrs-core` 版本。
容器收集的是**最终链接到程序中的注册集合**，并非磁盘上所有 workspace 成员。

## 把接口、实现和应用拆开

以订单存储为例，可以组织成三个 crate：

```text
order-contracts       公开业务接口；可以不依赖 Nestrs
order-infrastructure  依赖 contracts 和 nestrs-core；声明具体服务
order-app             依赖上述两个 crate、nestrs-core 和 tokio
```

接口库只写普通 Rust trait：

```rust
// order-contracts/src/lib.rs
pub trait OrderStore: Send + Sync {
    fn backend(&self) -> &'static str;
}
```

实现库声明服务并实现接口。这里的结构体和模块都可以是私有的：

```rust
// order-infrastructure/src/lib.rs
mod storage {
    use nestrs::injectable;
    use order_contracts::OrderStore;

    #[injectable]
    struct PostgresOrderStore;

    impl OrderStore for PostgresOrderStore {
        fn backend(&self) -> &'static str {
            "postgres"
        }
    }
}
```

应用只需知道公开接口：

```rust
// order-app/src/main.rs
use nestrs_core::{ServiceProvider, get_required_service};
use order_contracts::OrderStore;
use order_infrastructure as _;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::build().await?;
    let store = get_required_service!(provider, dyn OrderStore).await?;
    assert_eq!(store.backend(), "postgres");
    provider.dispose_async().await?;
    Ok(())
}
```

`use order_infrastructure as _;` 明确引用仅提供静态服务注册的库。若应用已使用该库的
公开类型或函数，通常已有引用；只在 Cargo.toml 中列出一个完全未使用的依赖，不应
被当成“该库的服务一定参与当前 binary”的保证。

`nestrs` 由工具提供，不在应用或实现库的 Cargo.toml 中添加宏库依赖。
手工组装项目后运行 `cargo nestrs init`，构建、运行和导图分别使用：

```sh
cargo nestrs check -p order-app
cargo nestrs run -p order-app
cargo nestrs graph -p order-app
```

同样的接口也可以放在另一个业务库服务的字段或 factory 参数中：

```rust
use nestrs::injectable;
use order_contracts::OrderStore;

#[injectable(lifetime = Scoped)]
pub struct Checkout {
    #[inject]
    store: dyn OrderStore,
}
```

这个业务库只依赖 contracts 和 core，不需要依赖具体存储库。最终应用同时链接业务库
和实现库后，构建容器时会把它们的需求与服务汇总。

## 服务选择和生命周期

跨 crate 不改变原有规则：

| 情况 | 行为 |
| --- | --- |
| 同一接口只有一个符合 key 的实现 | 自动选中 |
| 多个实现具有相同 key | 必须恰好有一个 `#[primary]` |
| `#[inject("audit")]` 或 keyed 查询 | 只选择对应 key，不回退到默认服务 |
| `Option<dyn Trait>` | 没有候选时为 `None`；有候选时仍验证歧义、环和生命周期 |
| 同一 concrete 的多个接口 | 使用同一个 provider；Singleton/Scoped 缓存仍共享 |
| 兄弟 crate 生成了同一条自动投影 | 幂等合并，不重复创建服务 |
| 不同 crate 各有一个同名结构体 | 保持不同类型身份，不按名字合并 |
| 一个接口有多个实现，但整个链接单元没有请求它 | 不因这个未使用接口产生歧义 |

已注册的具体服务依然全部参加验证，即使 Lazy 模式下没有查询它，也不能有缺失依赖
或非法生命周期。上表最后一项仅针对**未请求的接口投影**，不会放宽具体服务验证。
显式重复 concrete provider，以及内部显式 binding ABI 的重复声明仍然报错。

升级后，完整发现可能找出旧版本遗漏的上游候选。如果已确定的 `Repository<User>`
与 `Repository<Order>` 都实现同一个非泛型 `RepositoryPort`，它们会参与同一次
候选选择；需要用 primary、key，或 `RepositoryPort<T>` 这样的不同接口身份表达
业务选择，不能依赖旧版本漏掉其中一个。

## 私有实现为什么能使用

实现库编译时，工具在原模块中生成 Rust 编译器检查过的接口投影，并导出隐藏的描述
回调。下游使用这个描述，不需要访问私有实现类型。没有修改业务类型的可见性，也
没有手动创建 vtable 或扩大引用生命周期。

这些描述是潜在能力。只有最终程序中的查询根或服务依赖需要某个接口时，图编译器
才启用相应投影及必要的闭合蓝图。构造函数、factory、`Default`、`#[value]` 和
cleanup 不会在收集或分析时执行。容器构建先完成结构验证并冻结图，再按照
Lazy/Eager 策略实例化；后续查询也只使用冻结计划。

因此，`cargo nestrs check/build` 通过意味着 Rust 类型及生成代码通过检查。
完整链接单元的依赖结构检查发生在容器 build 或 `cargo nestrs graph`；数据库等
外部资源是否初始化成功，要等实际构造时才知道。

## 泛型与支持边界

- 仍然只处理可确定的闭合类型。直接查询 `Repository<User>`、服务字段依赖该类型，
  或源码中的闭合实现 `impl Store for Repository<User>` 都能提供有限类型信息。
  不会仅凭一个开放的 `impl<T> Store<T> for Repository<T>` 枚举所有可能的 `T`。
- 接口中的已确定泛型参数、关联类型和父接口由 Rust 类型检查器分析；Cargo 依赖别名
  和公开重导出不改变类型身份。生成位置必须满足正常 Rust 可见性规则。
- 上游预生成目录覆盖业务接口的合法 `Send`/`Sync` 组合。额外添加 `Unpin`、
  `UnwindSafe` 等 auto trait 会形成不同的 trait object 类型；若需要的精确形状
  尚未由上游生成，公开实现可由下游补投影，私有实现不能绕过可见性。不要将此视为
  所有可能 trait object 形状或无限泛型组合的反射能力。
- 服务库需要经过匹配的 `cargo nestrs` 构建并携带当前元数据。同一最终程序使用
  同一份 core；工具链不把多个不兼容的容器 ABI 混成一个图。
- 宏生成且无法定位合法源码插入点的模块、独立 doctest 中新增的自动绑定及跨 target
  图执行仍遵循 [工具链说明](NESTRS_CARGO_TOOLCHAIN.md) 的边界。

## 可运行回归项目

仓库提供 [cross-crate-binding](../cargo-nestrs/tests/fixtures/cross-crate-binding/README.md)
多 crate 项目，包含独立接口库、两个实现库、业务消费者库和最终应用。它验证真实
实例、共享地址、关闭计数和导出的图，业务源码没有手写 `bind`。

```sh
python3 tools/build-toolchain.py
python3 tools/verify-cross-crate-binding.py --skip-build
```

验证报告和实际 HTML 保存在 `target/nestrs-cross-crate-binding/`。脚本同时检查
debug/release 与 metadata-only check，并核对工具没有改写业务源码或 manifest。
