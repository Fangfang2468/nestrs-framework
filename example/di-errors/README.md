# DI 编译错误观察示例

这里有 **30 个故意无法通过编译的轻型项目**，统一在本页说明。每个项目只包含自身的
`Cargo.toml` 和 `src/main.rs`，直接依赖 `nestrs-core`，不需要外部服务或 Tokio 入口。
`main` 为空：最终入口的依赖图验证在编译时发生，不需要调用容器构建。

本组是[独立 Cargo workspace](Cargo.toml)，共享依赖、lockfile 和构建缓存，未加入
仓库根 workspace。正常的 `di-checkout` 运行、框架 `--workspace` 构建和图导出不会
被这些负例阻断。编号表示阅读顺序，不是错误代码。

## 运行与核对

先按[仓库说明](../../README.md)构建匹配的 CLI、driver 和私有 bridge，并将 CLI
加入 PATH。在仓库根目录执行：

```bash
cargo nestrs check --manifest-path example/di-errors/01-missing-concrete/Cargo.toml
cargo nestrs build --manifest-path example/di-errors/01-missing-concrete/Cargo.toml
```

也可以进入对应子目录执行 `cargo nestrs check`。**单个编译命令必须非零退出**，并且
给出表中预期的 Nestrs 诊断。普通 `cargo check` 缺少工具注入的宏环境，不用于观察
这些 DI 错误。对整组执行 `--workspace` 可能在首批失败后停止，不能代替逐例核对。

[verify.py](verify.py) 逐项目调用真实 `cargo nestrs check/build --locked`：

```bash
python3 example/di-errors/verify.py
python3 example/di-errors/verify.py --operation build
python3 example/di-errors/verify.py --case 18-lazy-scoped --offline
```

Windows 将 `python3` 换成 `python`；`--offline` 要求依赖已缓存。脚本核对错误编号的
数量与顺序、标题、用户源码主位置、涉及的类型/输入及 `help` 先于末尾 `cause`，并拒绝
编译器崩溃、额外 Rust 错误或缺工具链等无关失败。显示 `PASS` 且脚本退出 `0` 表示
**编译按预期失败**。原始日志和 `summary.json` 默认在
`example/di-errors/target/diagnostics/check/` 或 `build/`，可用 `--logs-dir PATH` 修改。

## 缺失与 key 匹配

项目链接直接定位到需要观察的源码；替换上述命令中的目录名即可复现。

| 项目 | 源码中的错误关系 | 预期诊断 | 修复方向 |
| --- | --- | --- | --- |
| [01-missing-concrete](01-missing-concrete/src/main.rs) | `OrderService.database` 请求未声明的 `Database` | `NESTRS-DI001` | 给 `Database` 声明 injectable 或返回该类型的 factory |
| [02-missing-trait](02-missing-trait/src/main.rs) | `Checkout.gateway` 的 `PaymentGateway` 没有 provider 候选 | `NESTRS-DI001` | 声明实现该 trait 的服务；普通 impl 会按需求自动绑定 |
| [03-missing-default-key](03-missing-default-key/src/main.rs) | 默认 key 的请求只有 `key="read"` 的声明 | `NESTRS-DI002` | 统一请求和声明的 key，默认 key 不会回退 |
| [04-missing-named-key](04-missing-named-key/src/main.rs) | `gateway` 请求 `"live"`，候选声明为 `"sandbox"` | `NESTRS-DI002` | 为 `"live"` 提供服务，或明确改用 `"sandbox"` |
| [05-key-kind-mismatch](05-key-kind-mismatch/src/main.rs) | `queue` 请求整数 `7`，声明使用字符串 `"7"` | `NESTRS-DI002` | 同时统一 key 的值与类型 |

## Trait 候选选择

| 项目 | 源码中的错误关系 | 预期诊断 | 修复方向 |
| --- | --- | --- | --- |
| [06-ambiguous-trait](06-ambiguous-trait/src/main.rs) | `CardGateway`、`BankGateway` 均满足默认 key 请求 | `NESTRS-DI003` | 选择唯一 primary，或按 key 区分实现 |
| [07-multiple-primary](07-multiple-primary/src/main.rs) | 同 key 的两个 gateway 都是 primary | `NESTRS-DI004` | 只保留一个 primary，或使用不同 key |
| [08-primary-other-key](08-primary-other-key/src/main.rs) | `"preferred"` 下的 primary 无法决定默认 key 的两个候选 | `NESTRS-DI003` | 在实际请求的 key 内明确选择 |
| [09-optional-ambiguity](09-optional-ambiguity/src/main.rs) | optional `MessagePort` 有两个候选 | `NESTRS-DI003` | 消除候选歧义；optional 只允许没有候选 |
| [10-lazy-ambiguity](10-lazy-ambiguity/src/main.rs) | lazy `MessagePort` 有两个候选 | `NESTRS-DI003` | 编译时选定唯一候选；延迟构造不会延迟类型选择 |

## 环与生命周期

| 项目 | 源码中的错误关系 | 预期诊断 | 修复方向 |
| --- | --- | --- | --- |
| [11-self-cycle](11-self-cycle/src/main.rs) | `Cache.parent → Cache` | `NESTRS-DI005` | 移除自依赖，或用普通业务数据表达父子关系 |
| [12-dependency-cycle](12-dependency-cycle/src/main.rs) | `OrderService ↔ PaymentService` | `NESTRS-DI005` | 移除一条服务依赖，或提取共同协作者 |
| [13-optional-cycle](13-optional-cycle/src/main.rs) | `Alpha → Option<Beta> → Alpha`，Beta 已存在 | `NESTRS-DI005` | 拆开实际存在的环；不能要求 optional 自动交付 None |
| [14-lazy-cycle](14-lazy-cycle/src/main.rs) | `Alpha → lazy Beta → Alpha` | `NESTRS-DI005` | 拆开完整依赖图中的环 |
| [15-singleton-scoped](15-singleton-scoped/src/main.rs) | Singleton `ApplicationCache → RequestSession` Scoped | `NESTRS-DI006` | 若消费者属于请求就改为 Scoped，否则分离请求级依赖 |
| [16-transitive-scoped](16-transitive-scoped/src/main.rs) | Singleton `Application → Formatter` Transient `→ RequestSession` Scoped | `NESTRS-DI006` | 调整实际所有权边界；中间 Transient 不消除 Scoped 要求 |
| [17-optional-scoped](17-optional-scoped/src/main.rs) | Singleton `Application` 的 optional trait 选中 Scoped `Session` | `NESTRS-DI006` | 调整消费者生命周期或依赖；optional 不改变目标生命周期 |
| [18-lazy-scoped](18-lazy-scoped/src/main.rs) | Singleton `Application → Intermediate` Transient `→ lazy Session` Scoped | `NESTRS-DI006` | 拆开应用级与请求级依赖；lazy 仍传播 Scoped 要求 |

## 构造方式、重复声明与泛型

| 项目 | 源码中的错误关系 | 预期诊断 | 修复方向 |
| --- | --- | --- | --- |
| [19-factory-missing](19-factory-missing/src/main.rs) | factory `application(_database)` 缺少 Database | `NESTRS-DI001` | 声明参数依赖；参数未在函数体使用也属于依赖 |
| [20-factory-cycle](20-factory-cycle/src/main.rs) | factory `first(Second)` 与 `second(First)` 形成环 | `NESTRS-DI005` | 拆开工厂参数依赖环 |
| [21-factory-scoped](21-factory-scoped/src/main.rs) | Singleton factory `application` 请求 Scoped `RequestSession` | `NESTRS-DI006` | 调整 factory 生命周期或拆开依赖 |
| [22-factory-lazy-missing](22-factory-lazy-missing/src/main.rs) | factory 的 lazy 参数 `missing` 没有 provider | `NESTRS-DI001` | 声明 Missing 服务；lazy 不允许缺少必选目标 |
| [23-constructor-missing](23-constructor-missing/src/main.rs) | `Application::new(_database)` 缺少 Database | `NESTRS-DI001` | 声明构造参数依赖；定位应指向原方法参数 |
| [24-duplicate-provider](24-duplicate-provider/src/main.rs) | 两个 factory 为同一 Database、默认 key 创建实例 | `NESTRS-DI007` | 保留一个创建声明或使用不同 key；primary 无法覆盖重复 provider |
| [25-generic-missing](25-generic-missing/src/main.rs) | `Application` 闭合 `Repository<User>` 后，其 database 缺失 | `NESTRS-DI001` | 修复泛型声明内部依赖；诊断同时关联闭合来源 |
| [26-generic-growth](26-generic-growth/src/main.rs) | 未执行分支中的 `growing::<(T,)>()` 持续增加查询类型复杂度 | `NESTRS-DI008` | 使用有限闭合类型，或用运行期数据结构表达递归 |
| [27-duplicate-binding](27-duplicate-binding/src/main.rs) | 隐藏协议对同一 `Service → Port` 显式绑定两次 | `NESTRS-DI009` | 移除重复显式绑定；普通业务使用自动绑定 |
| [28-orphan-binding](28-orphan-binding/src/main.rs) | 显式声明 `Service → Port` 投影，却没有 Service provider | `NESTRS-DI010` | 声明实例创建方；binding 自身不创建服务 |
| [29-provider-lazy-missing](29-provider-lazy-missing/src/main.rs) | 服务级 lazy 的 `DeferredApplication.missing` 缺少目标 | `NESTRS-DI001` | 补齐依赖；服务级 lazy 仅决定创建期自主初始化 |

`27`、`28` 专门观察隐藏的 `nestrs::bind` 内部协议。正常应用只写服务声明与
`impl Trait for Concrete`，不需要添加 bind。泛型展开上限按类型复杂度/实例数量
计数，不是普通依赖链深度；详细边界见[诊断指南](../../docs/NESTRS_DIAGNOSTICS_DESIGN.md)。

## 一次编译中的多个错误

[30-multiple-errors](30-multiple-errors/src/main.rs) 同时包含五处独立错误，按用户源码位置报告：

| 顺序 | 位置与原因 | 预期诊断 |
| --- | --- | --- |
| 1 | `OrderService.database` 缺少 Database | `NESTRS-DI001` |
| 2 | `OrderService.queue` 缺少 Queue | `NESTRS-DI001` |
| 3 | `Mailer.client` 请求 `"live"`，EmailClient 只有 `"sandbox"` | `NESTRS-DI002` |
| 4 | `Checkout.gateway` 有两个同 key gateway，没有唯一 primary | `NESTRS-DI003` |
| 5 | factory `report_service(_config)` 缺少 ReportConfig | `NESTRS-DI001` |

```bash
cargo nestrs check --manifest-path example/di-errors/30-multiple-errors/Cargo.toml
```

修复方向分别与缺失、key 和候选规则一致。接口歧义不会额外派生一条“缺失”错误；
每个独立错误仍保留自身的主位置、说明、修复提示和 cause。

## 如何理解这些结果

- 标题和主位置先说明用户源码的问题，完整类型、内部槽位及原因放在最后的 cause；
  两个代表输出与错误分类集中在[诊断指南](../../docs/NESTRS_DIAGNOSTICS_DESIGN.md)，
  不为每个项目复制一份容易过时的 stderr。
- [原生 JSON 诊断测试](../../cargo-nestrs/tests/diagnostics.rs)另逐一编译前 29 个项目，
  核对源码片段、完整类型高亮和关联位置；五错误示例由本目录 verifier 核对。
- `InvalidMetadata` 是工具内部不变量错误，不能由正常业务声明合法制造。本组不伪造
  内部槽位；相关协议由工具测试覆盖，见[fixture 索引](../../cargo-nestrs/tests/fixtures/README.md)。
- 根查询 Scoped、factory 返回 Err、构造 panic、初始化或关闭失败属于运行期契约；
  合法的 `get_service::<Missing>()` 缺席也不是必选注入错误，不能用本组负例替代这些测试。
