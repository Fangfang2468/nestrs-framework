# nestrs-config：独立配置库与框架集成设计

> 状态：独立配置运行时已实现，可在普通 Cargo 项目中使用；Nestrs 配置宏、自动校验、字段脱敏宏与 DI 集成仍是未来设计。
> `nestrs-config` 不依赖 `nestrs-core`、`cargo-nestrs` 或 Tokio。默认启用 JSON/TOML，dotenv 为可选 feature。
> 当前保持 `publish = false`。此前设计评审的历史记录保留在第 14 节。

本文是配置主题的唯一文档入口。第 0 节说明已经实现的独立库；第 3～5 节的普通运行时
规则已经落实。涉及 configable、config_value、声明式校验、sensitive 和 DI 分派的内容
仍为拟议契约，不表示当前可以编译使用。源码规范见 [AGENTS.md](../AGENTS.md)。

## 0. 独立配置库：当前用法

普通项目通过路径依赖使用本库，并为自己的模型启用 Serde derive：

```toml
[dependencies]
nestrs-config = { path = "../nestrs-framework/nestrs-config" }
serde = { version = "1", features = ["derive"] }
```

以下示例完全通过普通 Cargo 编译，不需要 Nestrs 工具链或运行时：

```rust
use nestrs_config::{ConfigError, ConfigLayer, ConfigPath, Configuration, LayerValue, Origin};
use nestrs_config::sources::{Environment, Memory};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HttpOptions {
    port: u16,
    #[serde(default)]
    allowed_origins: Vec<String>,
}

fn main() -> Result<(), ConfigError> {
    let mut defaults = ConfigLayer::builder(Origin::named("defaults"));
    defaults.insert(ConfigPath::parse("app.http.port")?, LayerValue::unsigned(3000))?;
    let config = Configuration::builder()
        .add_source(Memory::new(defaults.build()?))
        .add_source(Environment::from_iter("APP__", [("APP__APP__HTTP__PORT", "8080")]))
        .build()?;
    let http: HttpOptions = config.get_required("app.http")?;
    assert_eq!(http.port, 8080);
    assert!(http.allowed_origins.is_empty());
    Ok(())
}
```

同一示例保存在 [examples/basic.rs](examples/basic.rs)，运行 `cargo run -p nestrs-config --example basic` 即可验证。

普通 Deserialize 完全由模型作者控制；独立库允许正常 Serde 属性。这不意味着未来
configable 会开放任意 Serde helper，也不自动执行校验规则或接管业务模型 Debug。

### 0.1 已交付接口

- `Configuration::builder().base_dir(...).add_source(...).build()`：顺序加载一次并交付不可变快照。
- `Configuration::get<T>` / `get_required<T>`：同步读取、创建新的 `T`，不执行 I/O 或共享类型化缓存。
- `ConfigService`：同一 crate 中固定定义，仅包含两个泛型读取方法，可通过具体类型或泛型静态调用。
- `ConfigPath::root()/parse()/key()/index()`：点路径或显式字面 key/数组索引；读取扩展为 `get_path`、`get_required_path`。
- `section/section_path`：拥有共享快照的 `ConfigSection`；相对 get/get_required、对应显式路径、子 section、explain 和 `bind<T>()` 均可用。
- `explain/explain_path`：只返回来源；`ConfigOrigins` 区分 Unknown、Single、Merged，`HistoryStatus::Unavailable` 明确不提供覆盖历史。
- `ConfigSource`、`LoadContext`、`ConfigLayer` 与 `LayerValue`：可由第三方实际构造的来源扩展入口。

`Configuration` 与 `ConfigSection` 都可 Clone、Send、Sync。克隆共享已有快照，原文件
或环境改变不会改变已交付数据。它们的 Debug 不输出原始值；取得的普通 Rust 对象
保持自己的 Debug，显式读取仍返回真实值。

### 0.2 来源构造与 features

| 来源 | 构造方法 |
| --- | --- |
| 内存 | `Memory::new(layer)` |
| JSON/TOML | `Json::file(path)` / `Toml::file(path)`，文件来源支持 `.optional()` |
| 进程环境 | `Environment::with_prefix(prefix).separator(separator)` |
| 显式环境输入 | `Environment::from_iter(prefix, pairs).separator(separator)`，键值接受 Into<OsString> |
| dotenv | `DotEnv::file(path).prefix(prefix).separator(separator).optional()` |

默认 features 为 `json`、`toml`；`dotenv` 需显式启用。`default-features = false`
仍保留内存、环境、来源扩展、路径、合并、Serde 绑定、section 与 explain。环境分隔符
默认为 `__`，dotenv 默认前缀为空；不自动加载 cwd 的文件或 `.env`。

自定义来源实现 `ConfigSource::load(&LoadContext) -> Result<ConfigLayer, ConfigError>`。
`LoadContext::base_dir/resolve_path` 使用本次固定目录。通过 `ConfigLayer::builder(origin)`
的 `insert(ConfigPath, LayerValue)` 和 `build()` 创建一层；插入仅接受对象 key 段，
数组通过 `LayerValue::array` 整体构造。相同路径及显式父/子路径不能重复插入；隐式父对象
允许继续插入兄弟字段。`ConfigLayer::from_value(value, origin)` 接受完整对象，
`ConfigLayer::empty()` 表示无值、无来源的空层。

`LayerValue` 提供 null/boolean/integer/unsigned/float/string/text/array/object 构造器；
float 和 object 返回 Result，分别拒绝非有限数及重复键。`with_origin` 设置节点来源。
Text 是环境式按目标转换的标量，String 是已有类型的字符串。该输入类型仅供扩展官方
来源，不是第三方 ConfigService 实现必须采用的统一值模型。

`Origin::file/environment/named` 创建定位元数据，`at(line, column)` 接受一基行列并
返回 Result。未指定子节点位置时只继承来源身份，不伪造行列。explain 汇总最终保留
子节点的来源，已完全被覆盖的来源不作为历史保留；组合对象无唯一位置时不假造位置。

### 0.3 格式、预算与安全边界

JSON 检测每层重复键（包括解码后同名的转义键），保留整数精度并拒绝超出 i64/u64
值域的整数。TOML 保留可用节点位置，拒绝原生日期时间和非有限浮点。环境与 dotenv
文本只在定向标量绑定时转换；普通文件字符串不自动转成数字。Serde flatten/untagged
缓存文本后可能无法再次执行定向转换，详见第 5.2 节。

dotenv 支持逐行 KEY=VALUE、可选 export、空行/注释、单引号与双引号。双引号支持
`\n`、`\r`、`\t`、`\\`、`\"`；无引号外围空白被裁剪，空白后的 # 开始注释。
变量引用保留原文，不插值、不执行 shell、不修改进程环境。跨行引号、续行、未知转义、
缺少等号、重复键和未闭合引号明确报错。这是受限 dotenv 子集，不承诺 shell/dotenvy 全兼容。

公开预算常量为每文件 16 MiB、根深度 0 的最大节点深度 64、每层和合并结果最多
100,000 个节点（包含根）。文件在读取阶段限制字节，树转换/构造及合并检查结构预算。
TOML 原生 parser 在转换前建立其解析树；这些预算不等于解析器瞬时分配的硬上限。

`ConfigError` 仅保留受控类别、结构化位置及可选来源，不保存任意原始错误文本；
`Error::source()` 返回 None。动态 Map 绑定错误使用稳定条目序号，省略含键的来源；同一次绑定访问动态 Map 后，
后续容器级或其他错误也可能保守省略来源，仍保留安全位置。
快照、层、值、source、builder、section 的 Debug 不显示原值。来源展示名和显式查询
路径是定位元数据，调用方不能把凭据写入其中；库不拦截用户代码主动打印或 panic hook。

### 0.4 验证入口

```sh
cargo test -p nestrs-config
cargo test -p nestrs-config --no-default-features
cargo test -p nestrs-config --no-default-features --features json
cargo test -p nestrs-config --no-default-features --features toml
cargo test -p nestrs-config --no-default-features --features dotenv
cargo test -p nestrs-config --no-default-features --features json,dotenv
cargo test -p nestrs-config --no-default-features --features toml,dotenv
cargo test -p nestrs-config --all-features
cargo clippy -p nestrs-config --all-features --all-targets -- -D warnings
cargo fmt -p nestrs-config -- --check
```

测试位于 `tests/`；独立 `tests/fixtures/standalone` 项目仅依赖重命名后的本库和 Serde，
由集成测试运行普通 Cargo 并检查依赖树。框架宏和 driver 不在独立库验收范围内。

## 1. 目标与范围

配置体系分为共同协议、工具链声明与具体客户端。官方客户端按顺序加载和合并来源；
其他客户端实现同一类型化读取协议，自行组织存储、解析与绑定。工具链为根配置模型
生成 Deserialize、受控 Debug，以及类型化读取和校验调用；普通嵌套类型
按真实字段关系参与自动校验，不因此改写其绑定或 Debug，也不自动注册成服务。
消费者通过 config_value 取得配置对象或字段。

```text
具体配置客户端 / 第三方适配器
    → get_required::<配置模型>(path) 返回普通 T
    → 工具链连接 schema 校验及对象/字段交付
    → #[config_value] 普通对象值 / 字段值

服务依赖：消费者 → 所选配置服务实现（具体注入/分派方式待评审）
配置声明：#[configable] → 根模型与地址，不生成服务节点
嵌套校验：真实字段类型 → 普通嵌套对象及规则，无额外类型标记
```

### 1.1 整体目标（独立运行时已交付，工具链部分待实现）

- 有序配置源、结构化值、明确的覆盖规则、来源追踪和不可变 `Configuration`。
- 内存、TOML、JSON、只读环境变量和可选 `.env` 来源。
- 公共协议仅 get/get_required 两个类型化操作；官方客户端可额外提供 section/bind/explain。
- 固定公共 trait 身份、类型绑定与安全错误的行为约定；不强制统一 ConfigValue 或存储模型。
  具体注入和分派方式仍待评审，不为应用生成不同身份的公共 trait。
- `#[configable]` 声明 prefix、自动反序列化、默认未知字段检查及校验元数据，不生成 DI provider。
- 配置字段仅开放 `#[default]`、`#[default(表达式)]`、`#[label("配置键")]` 与
  sensitive，以及下述校验规则；不把任意 Serde 属性作为 configable 的公开声明接口。
- 声明式校验：数值/存在性/字符串格式、集合元素、跨字段及条件组合、自定义 rule，
  由工具链统一收集并连接；不要求业务实现或启用 Validate。
- 不提供 config_object，也不增加 nested/ValidateNested 启用标记；工具链沿真实字段
  类型自动深入普通嵌套对象，仅在本地及可分析的可达子类型均无规则时省略校验。
  无外层宏的校验属性接入仍待设计；自动级联不自动接管嵌套类型的反序列化和 Debug。
- `#[config_value("app.database")]` 绑定整对象；`#[config_value("app.database.port")]`
  绑定字段，`#[config_value(T::field)]` 为显式字段选择器。真实 DI 依赖指向读取门面，
  配置模型不接受 inject；对象所有权及字段复制基线见第 7.5 节。
- 敏感配置脱敏：`#[sensitive]` / `#[sensitive("******")]` 保留字段原类型，控制
  配置对象的受控 Debug；原始配置诊断与完整错误链独立采用安全表示，详见第 9 节。
- 普通 factory 接入及真实编译器契约；无需先实现 bootstrap。

### 1.2 当前已经交付的内容

单一 nestrs-config library 已提供第 0 节的独立运行时。协议与官方客户端同 crate 定义，
公共读取 trait 不引用内部节点或来源扩展值。尚未实现配置宏、自动规则、字段脱敏宏及 DI
接入；不提供这些能力的占位 API。workspace 的 default-members 保持原值。

### 1.3 不在第一版范围

bootstrap、logger 实现、管理端点、远程配置中心、自动文件监听、热更新、动态 provider
替换、运行期扩图、DI 集合解析、复杂 profile 激活和递归 import、配置文本中的占位符/表达式求值、
新的配置 CLI 命令。YAML 也留待后续明确需求；不宣称兼容 Spring/Nest/.NET 的全部语法。

## 2. 配置声明、公共协议与客户端分层

配置模型是数据，配置客户端是服务：inject 获取真正的服务，config_value 绑定配置
对象或字段。公共接口先表达“按路径取得目标类型”，分派和实现细节单独设计。
独立运行时接口已实现；配置宏仍未实现，普通 Cargo 验收不替代 Nestrs 编译集成验收。

### 2.1 工具链与客户端各自负责什么

| 层次 | 责任 |
| --- | --- |
| cargo-nestrs 的 codegen 与私有 bridge | 处理根配置声明，生成其 Deserialize/受控 Debug、校验及消费者调用；普通类型规则需新增合法属性采集入口，不能靠根宏跨类型读取 token |
| cargo-nestrs 编译器适配 | 沿真实字段类型连接自动级联、核验跨 crate 元数据，静态选择对象或字段，接入最终确定的服务分派方案 |
| 公共运行期契约与声明支持 | 定义唯一 ConfigService trait、类型约束、安全错误和声明规则支持；不规定原始配置树 |
| 官方客户端及第三方适配器 | 自行加载、存储、解析、缓存并按 T 转换；实现公共 get/get_required 行为 |
| nestrs-core | 执行冻结服务计划，维护输入、实例、lease、初始化和关闭 |
| 未来 bootstrap / logger / 管理端点 | 应用装配 / 日志事件策略 / 配置查看权限 |

公共 trait 在 nestrs-config 中固定定义，不为每个应用生成不同身份的公共 trait；
其类型约束和错误不引用客户端节点。当前与官方客户端同 crate，Serde 是独立库的普通
依赖；未来工具链生成模型与调用的支持路径另待评审，不新增公开宏 package。

官方实现可以复用自己的节点、快照和绑定器，也可以共享通用实现代码；这些代码复用
选择不升级为所有后端必须实现的协议。把 get_value 移到另一个公共 ConfigBackend
或用结构体隐藏它，只解决业务可见性，仍强制客户端采用节点适配，不能据此宣称满足
“后端只需实现两个类型化读取方法”的边界。

独立配置库不依赖 core 或 bootstrap。未来 DI 集成保持 core 在下层、bootstrap 在上层；
core 不依赖配置实现，应用运行时不链接 CLI/rustc，工具链也不扫描部署文件来决定服务图。

### 2.2 公共 trait 只定义两个类型化方法

以下是独立库已经提供的公共协议；ConfigError::missing 使用相同的安全错误约定：

```rust
pub trait ConfigService: Send + Sync {
    fn get<T>(
        &self,
        path: &str,
    ) -> Result<Option<T>, ConfigError>
    where
        T: DeserializeOwned;

    fn get_required<T>(
        &self,
        path: &str,
    ) -> Result<T, ConfigError>
    where
        T: DeserializeOwned,
    {
        self.get::<T>(path)?
            .ok_or_else(|| ConfigError::missing(path))
    }
}
```

公共操作只有两个；get_required 可以使用上述默认实现，也允许后端在保持同等行为时
自行实现。不另加 get_value、snapshot、section、explain、来源查询或底层绑定接口。

| 结果 | get<T> | get_required<T> |
| --- | --- | --- |
| 路径不存在 | Ok(None) | Missing 错误 |
| 值存在且能转换为 T | Ok(Some(value)) | Ok(value) |
| 路径非法、读取失败或类型不匹配 | Err，不伪装成缺失 | Err |
| 路径存在且为 null | 按 T 转换，外层不视为缺失 | 按 T 转换 |

例如 get::<Option<String>> 遇到显式 null 返回 Some(None)，路径缺失才返回外层 None。
精确数值、缺失/null、路径和错误安全属于行为契约；可以用适配器一致性测试检查，
无须因此要求统一 ConfigValue、Origin、ConfigSnapshot 或公共树遍历接口。

DeserializeOwned 明确限定可读取类型。后端须尊重目标 Deserialize 所表达的
字段名称、默认值和未知字段检查；它可以使用自己的反序列化器或原库泛型读取 API。
不能声称任意 T 无约束就能构造，也不要求使用同一个物理节点模型。若将来需要独立于
Serde 的转换协议，要另行评审类型能力，不能以接口收敛为由默默取消当前类型约束。

本版 path: &str 采用第 4 节的快捷路径语义；类型化完整对象读取也可覆盖包含特殊键
或数组的内容。官方显式 ConfigPath 等能力属于扩展，不能未经评审塞回公共 trait。
安全错误类型仍须共用或适配，但不能要求每个后端伪造文件位置、来源树和覆盖历史。

### 2.3 可直接成立的调用与尚未成立的注入

上面的 trait 是合法的原生 Rust 泛型接口。具体后端实现它后，可以这样静态调用：

```rust
fn database_port<C>(config: &C) -> Result<u16, ConfigError>
where
    C: ConfigService + ?Sized,
{
    config.get_required::<u16>("app.database.port")
}
```

T 的选择和调用由编译器单态化，客户端可自行将配置转换成 T。这个示例不要求公共原始
节点，也不暗示把配置模型注册成服务。直接调用客户端时，泛型 trait 本身无需工具链
特殊处理；它如何与无后端类型的业务注入声明组合，仍须选择具体方案。

当前两方法 trait **不满足 dyn 兼容性**。不能把下面写法当作已成立的业务示例：

```rust
// 对第 2.2 节的 trait，普通 Rust 会拒绝这个对象类型：E0038。
fn unsupported(config: &dyn ConfigService) {}
```

同样不能承诺字段 inject dyn ConfigService 或 Box<dyn ConfigService> 已可工作。
现有注入宏把字段改成 Injection<dyn Trait>，并不能消除内部 trait 的 dyn 限制。
为方法增加 Self: Sized 可以排除它的动态分派，但也不能再通过 dyn 调用该方法。

<a id="config-facade-dyn-syntax"></a>

### 2.4 重点：impl dyn 的语法与适用前提

此前讨论的 `impl dyn ConfigService + '_` 属于 **trait 对象类型的固有实现**。
这个语法要求对应 trait 本来就能形成 dyn 对象；它不是让任意泛型 trait 自动支持
动态分派的语法。因此它不能用来实现第 2.2 节的新 ConfigService，旧代码示意已经撤回。
以下另用 ObjectReader 演示语言机制，名称和方法都不属于配置公共协议：

```rust
trait ObjectReader {
    fn text(&self) -> &str;
}

impl dyn ObjectReader + '_ {
    fn parse<T: std::str::FromStr>(&self) -> Result<T, T::Err> {
        self.text().parse()
    }
}
```

| 写法 | 含义 |
| --- | --- |
| impl ObjectReader for Concrete | 让具体类型实现 trait 的 text 方法 |
| impl dyn ObjectReader + '_ | 为对象类型增加固有方法，不是替每个实现类型实现 trait |
| dyn ObjectReader | 隐藏具体类型，通过动态分派调用 trait 方法的对象类型 |
| + '_ | 对象生命周期占位写法，避免把这个固有实现仅限定于 static 对象 |

这里的 `+ '_` 可以显式写成：

```rust
impl<'a> dyn ObjectReader + 'a {
    // 同一个固有实现适用于不同对象生命周期。
}
```

而在这个位置省略对象生命周期：

```rust
impl dyn ObjectReader {
    // 此处对象生命周期默认为 'static。
}
```

相当于针对 dyn ObjectReader + 'static 实现。static 约束隐藏的实现所携带的借用，
不意味着调用者持有 &'static 引用、不要求对象永远存活，也不表示 DI Singleton。
+ '_ 不延长任何借用，也不放宽 Nestrs 注册服务的 static 要求。其他位置的对象默认
生命周期可能由外层引用推导，不能把这个 impl 位置的规则推广到所有 dyn。

在 ObjectReader 示例中，parse::<u16> 按 T 静态单态化；内部 text() 才通过虚表调用。
trait 本身没有泛型方法，所以能形成对象。这与在 ConfigService trait 本身声明
get<T>/get_required<T> 是不同情况；把新的泛型方法再复制进 impl dyn 并不能修复 E0038。

固有方法只属于相应对象类型，不自动成为 Concrete 或一般 C: ObjectReader 的方法；
可以借用为 &dyn ObjectReader 再调用。固有 impl 必须与 trait 定义处于同一 crate，
不能要求第三方给外部对象类型增加固有方法。Deref 包装只帮助方法查找，不改变 dyn
兼容性。此前语言探针还验证了显式附加 auto trait 的对象并不自动获得那组固有方法。
这些语言机制不构成配置注入已实现的证据，原始实验范围见第 14 节。

固有 impl 也不隐藏基础 trait 方法。若公共 trait 声明 get_value，业务就能调用它；
doc(hidden) 仅影响文档，前导下划线没有访问控制作用，trait 方法不能用 pub(crate)
单独变为私有。本轮通过收敛公共契约撤回该方法，不以文档隐藏代替边界设计。

语言依据见 Rust Reference 的 [inherent implementations](https://doc.rust-lang.org/reference/items/implementations.html#inherent-implementations)、
[dyn compatibility](https://doc.rust-lang.org/reference/items/traits.html#dyn-compatibility) 和
[default trait object lifetimes](https://doc.rust-lang.org/reference/lifetime-elision.html#default-trait-object-lifetimes)。

<a id="config-facade-visibility"></a>

### 2.5 分派方式是独立待决问题

用户已确定两方法边界；此前基于 get_value 的门面 A/B 不再作为现行待选公共接口。
需要选择的是如何调用这个类型化后端，而不是要求后端重新暴露底层：

| 路线 | 可行范围与代价 |
| --- | --- |
| 原生静态分派 | 具体实现或 C: ConfigService 可直接调用；业务服务/适配器的后端类型、闭合泛型和注入形式需要明确 |
| 工具链有限类型适配 | 可研究收集实际闭合 T，为已选 concrete 后端生成 get::<T> 调用入口；尚未设计或实现，不能视为现有 DI 自动支持 |
| 带内部擦除的结构体门面 | 普通结构体可隐藏实现，但不能直接保存 Box<dyn 此泛型trait>；需要另外评审擦除/接入协议，不能仅把 get_value 藏起来便称只需两个方法 |

若工具链保留源码中的 dyn ConfigService 写法，却在类型检查前改成专用句柄，它就
不再是标准 Rust 对该泛型 trait 的对象化。这是新的声明/生成能力，须明确与既有
原生 Rust 检查边界的关系；尚未授权实现这种特殊语义，也不默认选择这条路线。
需要覆盖跨 crate 泛型、私有目标类型、有限闭合类型集合、上游已编译消费者和缓存
兼容性，不回写 rlib、不伪造 vtable、不放宽类型/借用检查，也不引入运行期扩图。

本稿后续用“所选配置服务输入”描述生成逻辑，不预设它已经是可注入的 dyn 对象。
宏调用示例表达目标语义，实际输入类型、候选选择和适配器生成须在实现前评审完成。
不能因为调用名与现有 DI 查询相似，就宣称这一步已由现有查询分析自动解决。

### 2.6 类型化读取与宏校验的衔接

configable 声明带 prefix 的根数据模型，嵌套普通对象按真实字段类型自动参与校验，
无需 config_object 或逐层启用标记。自动分析不生成配置 provider，也不改变普通
类型的既有服务声明。config_value 的目标执行次序是：

```text
取得所选配置服务输入
    → 调用 get_required::<完整所属模型>(prefix)
    → 客户端执行根模型生成的 Deserialize，以及嵌套类型自身的 Deserialize
    → 工具链连接根模型及可达嵌套对象的校验
    → 交付普通对象或选中字段
```

字段选择也先取得和校验完整所属模型，不绕过跨字段规则。工具链不要求客户端返回
中间树，也不需要理解它的解析器。普通 get<T> 遵循 Deserialize 的 default/label/strict，
不自动调用独立生成的业务规则；config_value 自动连接该校验，无需 Validate 开关。

第一版仍提议同步或异步准备客户端、交付后同步读取固定版本；这是可观察行为约定，
不要求统一快照类型。每次查询访问远端的异步协议另行设计，不在内部 block_on。
普通 factory 负责创建真正客户端；仅准备它不代表校验所有 schema。绑定/校验随
消费者构造发生，未消费模型仅接受编译期检查；配置模型不接受服务 lazy 标记。
Singleton 客户端可让 root 内消费者使用同一原始版本，但不保证独立绑定的默认表达式
只运行一次；多 root 实例独立。第 6/7/8 节继续约定这些数据语义，分派实现仍待定。

## 3. 官方客户端的配置源与 builder

### 3.1 对外用法

以下是已经实现的官方客户端 API。Configuration 是该客户端的具体读取/存储实现，
不再是生成代码必须依赖的服务类型。第三方实现无需复用该 builder 或 ConfigSource；
本阶段官方类型与公共读取协议位于同一 crate，后续框架接入不在本轮范围：

```rust
use nestrs_config::{ConfigError, Configuration, sources::{Environment, Toml}};

fn load_configuration(environment: &str) -> Result<Configuration, ConfigError> {
    Configuration::builder()
        .add_source(Toml::file("application.toml"))
        .add_source(
            Toml::file(format!("application.{environment}.toml")).optional(),
        )
        .add_source(Environment::with_prefix("NESTRS__").separator("__"))
        .build()
}
```

- `builder()` 从空配置开始；添加来源不立即读取或修改来源。
- `add_source` 消费并返回 builder，保存拥有所有权的 source；后添加的来源优先。
- `build(self)` 同步返回 `Result<Configuration, ConfigError>`；失败不交付部分快照。
- `.base_dir(path)` 显式指定相对文件路径的基准；未指定时在 build 开始捕获当前目录一次。
  显式相对 base_dir 也在此时基于捕获目录解析。配置库不搜索父目录，不改变进程工作目录。
- `.optional()` 只忽略指定文件的 `NotFound`；权限、目录误用、编码和语法错误照常失败。
- builder 中的错误来源和非法选项按来源添加顺序报告，初版采用首错返回。
- 环境名称由调用方明确选择。示例不是完整的 profile 系统，也不隐式读取命令行参数。
  拼接文件名的环境名称应由应用从允许的名称中选择，不能直接当成任意路径输入。

推荐组合顺序为：内存默认值 → 基础文件 → 环境文件 → `.env` → 进程环境 → 命令行解析
结果 → 显式内存覆盖。真正的优先级以 `add_source` 顺序为准，没有隐藏的查询回退层。
命令行参数先由应用自己的解析器处理，config 不与 Clap 重复消费同一份参数。

### 3.2 来源扩展契约

ConfigSource 只扩展官方客户端的数据来源。它与第 2 节“替换整个客户端”的公共读取
协议处于不同层次；实现 ConfigSource 不等于实现统一服务门面。
以下为已经导出的 Rust 签名：

```rust
pub trait ConfigSource: Send + Sync {
    fn load(&self, context: &LoadContext) -> Result<ConfigLayer, ConfigError>;
}
```

builder 接收 `ConfigSource + 'static`，可保存异构 source；该 trait 无泛型方法，是对象
安全的同步协议。`LoadContext` 只提供本次加载固定的基础目录等读取上下文；source 返回
独立 `ConfigLayer`，不修改低优先级层、不自行决定跨来源优先级。

`ConfigLayer` 保存根对象和每个节点的来源。自定义来源通过受检查的层构造接口创建值、
分段路径与 `Origin`；重复路径及父子形态冲突必须报错。已实现构造器见第 0.2 节，
自定义来源可以保留与内置来源相同的安全来源信息。

每次 build 按顺序各调用一次 source。环境源读取本次输入；内存源保存调用方提供的值。
库不承诺多个文件在同一个物理瞬间被原子读取。`Configuration` 的不可变性描述的是
成功构建后的一份逻辑快照，clone 共享该快照且不会重新加载来源。

### 3.3 同步 I/O 边界

官方客户端第一版 `build()` 是同步文件读取与解析，不要求调用方先启动 Tokio。它不自动创建后台
线程，也不把普通 `get` 变成 I/O。当前 core 对同步 factory 直接调用构造函数，见
[`runtime/worker.rs`](../nestrs-core/src/runtime/worker.rs)；同步配置 factory 会占用
当时的构造 worker，不能宣称已经自动隔离阻塞。

需要隔离阻塞时，应用可以使用异步配置 factory：把拥有所有权的 builder 或环境名称
移入 `spawn_blocking`，在里面调用同一个同步 build。不得将 factory frame 借用带进
要求 `'static` 的 closure。JoinError 应映射成不包含任意 panic payload 的安全配置错误，
不在本阶段为此重写 core 调度器；panic hook 本身的输出不由 config 保证。

公共门面的第一版提议是“同步或异步准备，交付后同步读取固定版本”。远程适配器可以在
异步 factory 中完成网络请求并准备后端，再通过 get/get_required 同步读取；这两个
方法不隐藏文件/网络 I/O，也不在内部 block_on。官方同步 ConfigSource 仍不是原生
异步 source 协议；每次查询访问远端需要另定异步读取接口，不因 DI 查询有 await 就视为已经支持。准备门面成功也不等于
所有配置 schema 已绑定或校验，见第 6.2/8 节。

### 3.4 文件与环境来源

| 来源 | 第一版约定 |
| --- | --- |
| 内存 | 默认值、程序显式覆盖和测试注入；不修改其他已构建快照 |
| TOML / JSON | 保留结构化类型与来源；根必须是对象，文件按 UTF-8 解析 |
| 环境变量 | 只读一次本次来源，显式 prefix 与 separator，不写回进程 |
| `.env` | 解析成独立文本来源，使用与环境源一致的键映射，不执行 shell、不展开变量、不写回进程 |

以下是官方客户端内部值域与来源约定，不要求第三方构造同一种节点。文件中的整数
保留整数语义，不经 f64 中转；官方内部模型区分有符号/无符号整数、有限浮点、
布尔、字符串、对象、数组及显式 null。超出支持数值范围直接报错。TOML 原生日期时间
和非有限浮点不在第一版跨来源值域内，须明确拒绝；需要日期时间时使用字符串和显式
Serde 适配器，不进行隐式、有损的格式归一化。

官方客户端的值节点还须区分“文本来源的待转换标量”与“已经有类型的字符串”。环境和 .env 使用
前者，文件字符串使用后者；该语义随节点合并、覆盖和路径视图一起保留。自定义来源可
通过层构造接口显式选择，不根据 Origin 的展示名称猜测绑定行为。内部用枚举还是独立
标签属于实现选择，不能因共用 String 存储就丢失这种区别。

环境变量映射示例：`NESTRS__APP__DATABASE__POOL_SIZE` → `app.database.pool_size`。
前缀按给定拼写过滤；仅环境键的路径段进行 ASCII 小写转换，单下划线保留。文件键和
普通配置路径区分大小写，不把 `pool-size`、`pool_size`、`poolsize` 自动视为同名。
环境变量值保持文本，到绑定时再按目标类型转换；字符串 `"null"` 不是显式 null。

同一环境层中规范化后撞名、空路径段、父键与子键冲突、非 Unicode 键值均报错，不按
操作系统枚举顺序偷偷选择一个值。提供 `Environment::from_iter(...)` 一类显式输入
方式，使测试不必修改全局环境；具体构造方式见第 0.2 节。

第一版环境来源只表达对象路径与文本叶子；纯数字段也是对象 key，不隐式转换数组索引。
数组由文件或内存层整体提供，不自动按逗号拆字符串，也不猜测字符串中的 JSON。

## 4. 路径、合并与缺失

公共 get/get_required 的 path: &str 使用以下快捷路径规则。路径说明“读取哪里”，
不规定后端必须如何保存数据；诊断位置也不能仅靠拼接字符串保存身份。

| 字符串路径情况 | 行为 |
| --- | --- |
| 空字符串 | 表示根对象 |
| 普通点路径 | 每段都是对象 key；items.0 的 0 也是对象 key，不表示数组索引 |
| 开头/结尾的点或连续点 | 无效路径；不折叠空段，不猜转义 |
| 包含 `[` 或 `]` | 无效路径；第一版不提供括号索引或转义语法 |
| 对象 key 不存在 | Missing；get 返回 None，get_required 报错 |
| 在标量/null 上继续取字段 | 类型/形态错误，不伪装成 Missing |

两个方法在调用时报告非法路径。含点、括号或空字符串的字面 key，以及数组元素，
可以通过读取所属对象、Map 或数组，再按 Rust 类型访问；公共两方法不承诺单独
寻址每一种特殊键。config_value 的特殊字段仍可用类型选择器，见第 7.2/7.4 节。

官方客户端另提供显式 ConfigPath，以字段段区分字面 key、以索引段表示数组元素。
零段表示根，索引越界为 Missing，容器形态不匹配为错误。读取入口使用 get_path/get_required_path、section_path/explain_path，
不改变公共 trait 的 &str 签名，也不增加通用 get_value。
section/explain 及分段读取均为官方客户端扩展，不能写成第三方必须提供的入口。

`prefix` 必须显式填写；`prefix = ""` 表示绑定整个根对象，非空 prefix 使用对象路径。
第一版属性参数只接受路径字符串，不支持通配符、数组索引、变量插值或表达式求值。

以下合并规则属于官方 builder/ConfigSource 的行为；替换整个客户端无需实现该来源
协议或暴露合并树。第三方仍须遵守公共读取的缺失、类型和错误约定。

| 输入情况 | 合并结果 |
| --- | --- |
| 相同路径的标量 | 高优先级值替换低优先级值，具体类型由绑定检查 |
| 两个对象 / Map | 按键递归合并；高层空对象不清空已有对象 |
| 两个数组 | 高优先级数组整体替换，包括数组元素内的对象 |
| 高层缺失 | 保留低层值 |
| 任意值与显式 null | 高层 null 整体替换；高层非 null 也可以替换低层 null |
| 空字符串 | 真实字符串，不等于缺失 |
| 高层空数组 | 替换为零元素数组 |
| 对象、数组、非 null 标量之间形态冲突 | 合并失败，不保留隐含子节点 |
| 同一 source 重复 key 或互相冲突的路径 | source 加载失败 |

环境文本与文件标量可以覆盖同一路径，例如文本 `"32"` 覆盖整数 `16`，之后由目标
`u32` 解析。环境文本不能覆盖对象或数组并被悄悄当成对应结构。

覆盖示例（运行时行为）：

```toml
# application.toml
[app.database]
host = "localhost"
port = 5432
pool_size = 16
min_idle = 2

[app.http]
port = 3000
allowed_origins = ["http://localhost:3000", "http://localhost:5173"]
```

```toml
# application.production.toml
[app.database]
host = "db.production.internal"
pool_size = 64
min_idle = 8

[app.http]
port = 8080
allowed_origins = ["https://example.com"]
```

再添加环境来源 `NESTRS__APP__DATABASE__POOL_SIZE=32`，绑定结果应为 host 使用生产文件、
port 保留 5432、pool_size 为 32、min_idle 为 8；allowed_origins 只有生产地址。

## 5. 查询、绑定和验证

### 5.1 普通 Rust API

通用读取只依赖两个方法。以下静态泛型调用展示已经能由 Rust 表达的接口形状；所用
配置读取 API 已实现；DI 如何交付该输入见第 2.5 节，仍不支持泛型 trait 的 dyn 分派：

```rust
fn read_database_port<C: ConfigService>(config: &C) -> Result<u16, ConfigError> {
    config.get_required::<u16>("app.database.port")
}
```

| 公共 API | 拟议结果 |
| --- | --- |
| `get<T>(path)` | `Result<Option<T>, ConfigError>`；T: DeserializeOwned，仅路径缺失返回 None |
| `get_required<T>(path)` | `Result<T, ConfigError>`；缺失是错误，可复用 get 的默认实现 |

普通 get/get_required 使用目标类型的 Deserialize。若 T 为 configable 类型，其生成
实现会应用 default、label 和未知字段策略；泛型读取本身不会自动调用独立生成的
校验函数。get_required::<u16>("app.database.port") 没有配置模型身份，也不会取得
DatabaseOptions::port 的声明默认值。config_value 才会静态选择 schema，调用后端
读取完整所属对象并执行自动校验后交付值。常规宏消费不需要校验启用开关。

不公开 bind_validated 或要求业务实现 Validate。若希望将来普通 get::<配置类型> 也
自动校验，应单独评审统一 ConfigDecode 协议或将规则纳入 Deserialize 的影响；不能
假装泛型 T: DeserializeOwned 自带规则发现，也不能先 blanket impl 再为配置类型
特化而绕过 Rust 重叠实现规则。完全手写装配时，调用方自行组织业务检查。

必选分组不存在时读取失败，不因目标字段全有默认值而自动创建整个缺失分组。
存在的空对象可以使用目标类型上的绑定默认规则。显式 null 交给目标反序列化类型处理；
普通配置结构体不接受 null，Option<T> 可以接受。

| 调用 | 路径缺失 | 显式 null |
| --- | --- | --- |
| get::<String> | Ok(None) | 类型错误 |
| get::<Option<String>> | Ok(None) | Ok(Some(None)) |
| get_required::<Option<String>> | Missing 错误 | Ok(None) |

可选读取不能吞掉类型或来源错误。后端自己的 getter 若将缺失与 null 合并，适配层
必须恢复这项区别，不能简单转发后宣称符合契约；无需对外新增 exists 或节点读取方法。

**官方客户端扩展。** 以下能力已在 Configuration 实现，不进入 ConfigService：

| 官方扩展 | 拟议结果 |
| --- | --- |
| section(path) | Result<ConfigSection, ConfigError>；立即取得必需子树，非法路径/缺失在此时报错 |
| section.get<T>(relative_path) / get_required<T> | 从已取得的同一子树读取，错误位置补齐父路径 |
| section.bind<T>() | T: DeserializeOwned；从该子树创建新 T，不自动执行独立生成的配置校验 |
| explain(path) | Result<ConfigExplanation, ConfigError>；无原值的已知来源信息，缺失报错 |

ConfigSection 是普通子树视图，不是 DI 服务。创建时固定已读取的节点及路径，后续
绑定使用同一子树；可保存 null/标量节点，对标量继续读取子路径仍为形态错误。
explain 只报告官方客户端实际记录的信息。最终来源不等于完整覆盖历史；未记录历史
时标记不可用，缺少最终来源时标记未知，不能伪造文件位置或返回原值。生成的通用
绑定/校验流程不调用这些扩展，不要求第三方保存同形节点或诊断视图。

### 5.2 绑定规则

- 公共读取以 T: DeserializeOwned 为转换契约，由各后端执行目标类型的 Deserialize；
  不由共同层接收统一节点再绑定。以下文本来源细则约束官方来源，以及声称兼容这些
  来源语义的适配器；不要求所有后端提供 TOML/env 等来源或相同内部标签。
- 官方客户端需提供适合文本来源的反序列化器；不能假设普通 String value 自然能被
  Serde 当作 u16 或 bool。其他后端可用自身机制实现同等读取行为。
- 文本绑定整数时按十进制检查语法、符号和范围；bool 只接受 `true` / `false`；不默认
  把 `yes`、`on`、`1` 当作 true。字符串目标保留文本，不先猜类型。
- 上述转换适用于反序列化器直接收到目标标量类型的请求。`deserialize_any` 对文本
  保持字符串语义；普通手写类型或嵌套字段类型中的 Serde flatten、untagged 等可能
  先缓存为字符串，后续绑定不再经过 config 的文本转换入口。这些组合不保证文本到
  数字/bool 的自动转换；需要显式
  字段适配或以字符串中间结构绑定后转换，不通过“先猜类型”改变其他 String 字段。
- 文件中的结构化类型按目标类型检查，不把任意文件字符串都强制转成数字。需要特殊
  表示时通过普通自定义类型或手写 factory 适配；不得把整数经浮点中转或默默截断。
- 数组任一元素失败即绑定失败，不跳过坏项。绑定总是创建新对象，不往已有 Vec 追加。
- configable 的字段命名与缺失值只按第 6 节的 label/default/Option 规则处理，不开放
  任意 Serde helper。普通手写 Deserialize 类型仍遵守自身的 Serde 规则；这不等于
  configable 开放 rename/alias/flatten 等声明。绑定默认值属于目标对象，不回写
  Configuration，也不伪造一个文件来源给 `explain`。
- `Duration`、URL、日期等通过字段类型自己的 Deserialize，或普通类型与手写 factory
  提供明确适配，不在 configable 字段上开放 Serde with 等 helper；不承诺普通 Rust
  类型自动接受 `"5s"` 等自定义单位。普通 `bind` 尊重类型自身的未知字段策略。

### 5.3 自动收集与声明式校验

校验规则是业务声明，收集和执行连接是工具链职责。configable 根模型及普通类型上
合法归属于 Nestrs 的规则声明都参与编译期检查，不要求 validate 开关、Validate
trait 或 config_object。是否在某次配置消费中执行，由根模型的真实字段类型关系决定。
普通类型上的规则不因未被配置消费就免于语法和类型检查。

不提供 configable 的 validate 参数，也不要求 validate_with、derive(Validate)
或手写 impl Validate。若实现内部使用执行协议，它只属于生成机制，不通过用户
手写的同名 trait 或 validate 方法发现规则，不新增公开宏 package 或运行期注册表。

普通类型无需外层配置标记是本轮的目标语法；它需要新的合法属性采集/消费能力，
不能仅靠后期类型分析处理未知属性。具体编译阶段及标准展开边界见第 6.5.3 节。

#### 5.3.1 收集、生成与执行

1. 工具链在声明所属 crate 合法收集根模型和普通类型的字段/类型规则，保留实际
   cfg/feature、宏卫生、源位置和声明顺序。校验属性必须在标准流程拒绝未知属性
   之前被识别或消费；不能扫描源码字符串、按名字吞掉其他宏的 helper。
2. 获得真实类型后，从 configable 根模型沿字段、受支持容器和上游类型关系分析
   可达对象。没有本地规则仍继续向下；仅在可分析的整个可达子结构都无规则时省略
   校验。类型身份使用真实 Ty/DefId，不从字段名字猜测，也不因参与分析而注册服务。
3. 在类型所属 crate 生成并检查合法的规则执行及级联能力，记录跨 crate 所需的
   类型关系/规则 metadata；消费 crate 连接使用点，最终入口核验与 DI 计划的关系。
   不能等下游出现根模型才回写上游 rlib、访问私有字段或补生成反序列化实现。
4. 运行期消费者适配器调用 get_required::<完整所属模型>(prefix)，后端执行根模型
   和嵌套类型各自的 Deserialize，完成默认值等绑定行为，再按第 5.3.2 节顺序执行
   字段规则、自动级联和类型规则，首错返回。失败不交付对象/字段或当前消费者，
   安全错误归入消费者构造失败；已执行的默认表达式/规则不承诺回滚。config_value
   不提供延迟句柄；其他服务延迟依赖消费者时，沿用 core 的延迟服务失败语义。
   没有规则仍按目标 Deserialize 执行类型、缺字段和未知字段等绑定检查。

规则发现不替嵌套类型提供 Deserialize，也不扩张其未知字段策略。外部叶子、无法
传递规则能力的边界和递归类型图须明确处理，不能把缺少 metadata 当作已证明无规则。

已声明但参数非法、类型不适用或未知的校验 helper 必须编译失败，不能当作无规则
忽略。真实 cfg 排除的字段/规则不参与收集、类型约束或执行。保留其他属性的标准
Rust 解析和诊断，不凭同名字符串消费其他宏的任意属性。

编译期只检查声明和生成代码。check/build/graph 不执行默认值或配置校验，也不
验证部署值；自动校验发生在实际消费者的 config_value 绑定过程中。声明存在就检查
规则语法/类型，但未被消费的 schema 不因此执行运行期校验。消费者的 Lazy/Eager
策略决定何时构造；显式 configable 模型不接受服务 lazy 标记，普通类型的既有
服务声明边界见第 6.2 节；启动集中校验也在该节另列待决。

#### 5.3.2 扩展规则与组合

以下都是拟议 API，不是已实现的宏。工具链统一收集字段及类型上的规则；不增加
validate 开关、公开 Validate trait 或运行期装饰器注册。字段 helper 支持裸名及
nestrs 路径，无需逐个导入。根模型的规则由 configable 与工具链处理；普通嵌套
类型的字段及类型规则由第 6.5.3 节待设计的声明入口采集，不再要求外层配置类型宏。

##### 存在性、数值与有限取值

| 写法 | 适用类型与语义 | 参考 |
| --- | --- | --- |
| `#[required]` | Option 必须为 Some；不代表字符串非空，也不要求值必须来自文件而非 default | Jakarta NotNull / class-validator IsDefined |
| `#[not_empty]` | String、Vec、固定数组、受支持 Map 长度不为零；空白字符串仍可通过 | NotEmpty / ArrayNotEmpty |
| `#[not_blank]` | String 至少含一个非空白字符；不裁剪或改写值 | NotBlank |
| `#[min(1)]` / `#[max(128)]` | 数值的包含端点下界/上界 | Min / Max |
| `#[greater_than(0)]` / `#[less_than(1.0)]` | 数值的排除端点下界/上界 | DecimalMin/Max 的 exclusive 能力 |
| `#[positive]` / `#[non_negative]` | 数值 > 0 / >= 0 | Positive / PositiveOrZero |
| `#[negative]` / `#[non_positive]` | 数值 < 0 / <= 0 | Negative / NegativeOrZero |
| `#[finite]` | f32/f64 不能为 NaN 或正负无穷 | IsNumber 对非有限数的限制 |
| `#[multiple_of(1024)]` | 整数必须为指定正整数的倍数 | IsDivisibleBy |
| `#[one_of("dev", "test", "prod")]` | String 或整数/bool 的有限允许集合 | IsIn |
| `#[none_of("root", "admin")]` | String 或整数/bool 的有限禁止集合 | IsNotIn |
| `#[is_true]` / `#[is_false]` | bool 必须为 true / false | AssertTrue / AssertFalse |

min/max、排除端点和正负规则支持 Rust 原生整数及 f32/f64，边界按真实字段类型检查，
不把整数统一转为浮点。浮点规则采用该类型的有限浮点值比较；应用上述数值比较规则
时 NaN 和正负无穷均失败，不假装具有十进制精确计算语义。multiple_of 第一版限整数，
除数必须为可表示的正整数，不能为零；实现必须避免极值运算溢出。边界使用字面量
（有符号数可为负），不执行用户函数；长度边界为非负整数。同一作用目标、同一规则
列表内相同规则与相同参数的完整重复，以及不可表示参数、空允许集合和可直接证明
的上下界冲突均诊断，不承诺求解任意规则组合的可满足性。不同参数的规则可以取
交集，when/each 也可分别声明多组；外层 min 与 when 内的 min、集合 length 与 each
内的元素 length 属于不同规则作用域，不误判为重复。

one_of/none_of 的候选必须属于同一支持类别；String 与字符串字面量按内容比较，
整数和 bool 按准确类型比较，不读取环境变量或进行隐式数值/字符串转换。enum 字段
自身的绑定限制可直接表达封闭取值集，不再提供 is_enum/is_string/is_int/is_boolean
等重复 Rust 类型检查的规则。

Option 的统一语义是：required 检查外层是否 Some，其他普通值规则在 None 时跳过，
Some(v) 对内部类型执行。显式 default 填入的值同样被检查；空串/空集合不等于缺失。
这与 Jakarta NotBlank/NotEmpty 自身拒绝 null 的行为有意不同。required 只用于 Option；
非 Option 的必填仍由绑定规则保证。规则只检查结果，不回退默认值或改变字段类型。

##### 字符串、长度与格式

| 写法 | 拟议语义 |
| --- | --- |
| `#[length(min = 1, max = 100)]` | String 的 Unicode 标量值数，或集合元素数；上下界至少一个 |
| `#[byte_length(max = 4096)]` | String 的 UTF-8 字节长度，区别于 length |
| `#[matches(r"[a-z][a-z0-9_-]*")]` | 按选定 Rust regex 方言完整匹配整个字符串 |
| `#[contains(".")]` / `#[starts_with("app-")]` / `#[ends_with(".example")]` | 大小写敏感的字面子串/前缀/后缀检查，不是正则 |
| `#[ascii]` / `#[alphanumeric]` | 所有字符属于 ASCII / ASCII 字母或数字；空串须另用 not_empty 拒绝 |
| `#[email]` | 单一 ASCII 邮箱地址格式；不验证邮箱存在或投递能力 |
| `#[url]` / `#[url(schemes("https", "redis"))]` | 含非空 host 的绝对 URL；默认只允许 http/https，可指定 scheme 白名单 |
| `#[ip]` / `#[ip(version = 4)]` | 可由 std::net::IpAddr 解析的地址；可限定 4 或 6，不接受端口或 CIDR 后缀 |
| `#[hostname]` | ASCII DNS 标签形式，可为 localhost 或 punycode 标签；不查 DNS |
| `#[cidr]` | IPv4/IPv6 地址加合法前缀长度；不要求 host bits 为零，不改写地址 |
| `#[uuid]` / `#[uuid(version = 4)]` | 8-4-4-4-12 十六进制形式；可限制版本及对应 RFC variant |
| `#[semver]` | SemVer 2.0.0 格式，包含合法的 prerelease/build 部分 |
| `#[rfc3339]` | 带明确时区偏移的日期时间字符串，使用下述受支持子集 |

本版集合覆盖 Vec、[T; N]、标准 HashMap<String, T> 与 BTreeMap<String, T>；不凭同名
类型或任意 len 方法推断支持。String 的 length 按 chars().count() 计数，不等于
UTF-8 字节数或用户感知字形数。not_blank 按 Rust 字符空白分类判断，不做 trim 写回。

matches 使用 Rust regex 的受支持语法和 Unicode 规则，语义等价于为整个模式增加
全串边界；默认大小写敏感，可在模式内用受支持的 inline flags。不承诺兼容 Java/JS
正则的回溯引用、look-around 等语法。字面模式在工具侧检查，非法模式编译失败；
工具与运行期的解析版本/选项必须一致，运行期复用已编译模式，不按每个值重复编译。

格式规则采用明确的配置用子集，不宣称与上游同名规则逐字等价：email 使用 ASCII
点分 atom local-part 与 DNS 形式 domain，拒绝展示名、注释、引号 local-part、地址
字面量和国际化邮箱；hostname 允许单标签及一个末尾根点，标签 1..=63 字节、非首尾
连字符，总长度去掉末尾点后至多 253 字节，不隐式执行 IDNA 转换。url 允许凭据部分，
适用于标为 sensitive 的连接串，但不尝试连接或判定目标可信；使用选定解析器的
语法与显式 scheme/host 约束。uuid 默认只约束规范分组形式，允许 nil/max；指定
version 时要求匹配版本和 RFC variant。rfc3339 接受有效公历日期及明确时区的时间，
最多纳秒小数精度，拒绝无偏移时间、未知偏移 -00:00 和闰秒，不隐式使用本机时区。
所有格式规则都不修改原值；具体解析器版本、边界测试语料与依赖 features 见第 11 节。

##### 集合整体与元素

| 写法 | 拟议语义 |
| --- | --- |
| `#[unique]` | Vec/数组中的元素按 Eq + Hash 判重；报告首次重复位置，不显示元素值 |
| `#[contains_all("read", "write")]` | Vec/数组包含全部指定候选；按元素值比较 |
| `#[contains_none("debug", "unsafe")]` | Vec/数组不包含指定候选 |
| `#[each(email)]` | 对 Vec/数组的每个元素检查 email |
| `#[each(not_blank, length(max = 64))]` | 每个元素按给定顺序执行多条规则 |
| `#[each_key(matches(r"[a-z][a-z0-9_-]*"))]` | 对 Map 的字符串键检查规则 |
| `#[each_value(min(1), max(100))]` | 对 Map 的每个值检查规则 |

集合自身的 not_empty/length/unique 与元素规则分别声明；有 each 不意味着集合必须
非空。each 的内部仅接受本节已定义的规则语法，不是任意 Rust 语句。允许对嵌套
容器组合 each/each_key/each_value；在内部写 required 可要求 Option 元素为 Some。
contains_all/contains_none 的候选先限字符串、整数或 bool 字面量及相应元素类型，
不按对象地址比较；unique 对不满足 Eq + Hash 的类型（如普通浮点）明确拒绝。

Vec/数组按索引遍历；Map 无论底层 HashMap 或 BTreeMap，都按原始字符串键的 UTF-8
字节字典序确定校验次序，排序不修改配置。每个元素内部仍按规则顺序返回首错。
元素索引进入安全错误位置；Map 键本身可能是待校验的敏感值，不直接插入公开错误
路径，使用稳定条目序号和 key/value 角色定位，详见第 5.3.3 节。

##### 跨字段、条件与对象级组合

| 写法 | 放置位置与拟议语义 |
| --- | --- |
| `#[compare(le = pool_size)]` | 字段；与同一对象的真实 Rust 字段比较，支持 eq/ne/lt/le/gt/ge |
| `#[when(tls_enabled, required, not_blank)]` | 字段；条件成立时才执行括号内的规则 |
| `#[when(mode == "prod", min(2))]` | 字段；按绑定后的字段值决定是否执行局部规则 |
| `#[at_least_one_of(username, token)]` | 类型；指定 Option 字段至少一个为 Some |
| `#[at_most_one_of(password, token)]` | 类型；指定 Option 字段至多一个为 Some |
| `#[exactly_one_of(password, token)]` | 类型；指定 Option 字段恰好一个为 Some |
| `#[all_or_none(username, password)]` | 类型；指定 Option 字段全部为 Some 或全部为 None |

compare 引用真实字段标识符（含 raw identifier），不把字符串当字段名，不受 label
重命名影响；第一版仅比较当前对象直接的非 Option 字段。eq/ne 使用准确类型的相等
比较，顺序关系要求双方可比较；数值比较仍拒绝非有限浮点。不自动解包右侧 Option，
复杂可选值关系使用对象级 rule。cfg 排除的目标、不存在字段或不兼容类型编译失败。

when 的条件先限定为当前对象的 bool 字段、其 ! 否定，或直接字段与字符串/整数/bool
字面量的 ==/!= 比较；不接受任意函数调用或求值脚本。字段名按真实类型检查，不读取
原始配置树。条件只门控括号内部的校验，不影响同字段的其他规则、自动子对象级联，
也不关闭反序列化、必填、未知字段或默认值处理。Option 的 required 在此仍检查外层
是否 Some，不能被“None 跳过普通值规则”提前跳过。

类型级字段组合只检查绑定后的 Option 存在性，包括 default 产生的 Some；不是检查
原始文件是否写过键。列表至少两个互异、有效的直接字段，字段必须是 Option。
字段名在编译期解析，错误定位到当前对象，并只列静态约束，不包含字段取值。

##### 自定义规则与执行顺序

固定目录不能覆盖全部业务格式，因此增加一个实际约束入口：

下例是有独立地址的根配置示例；同样的 rule 也可用于普通嵌套类型，属性接入边界见第 6.5 节。

```rust
#[configable(prefix = "app.transfer")]
pub struct TransferOptions {
    #[rule(check_chunk_size)]
    pub chunk_size: u32,
}

fn check_chunk_size(value: &u32) -> Result<(), &'static str> {
    if value.is_power_of_two() {
        Ok(())
    } else {
        Err("块大小必须是 2 的幂")
    }
}
```

rule 可以写在字段或类型上，
字段函数接收 &T（Option 为 Some 时接收内部值），类型函数接收整个 &Self 类型；
均返回 Result<(), &'static str>。它就是一条具体规则，声明即收集，不是启用其他
规则的开关，不恢复 validate_with。路径为真实可见的同步安全 Rust 函数；不注入
服务，不自动 await，不转换任意 Error，也不把 String 消息透传为默认诊断。
同一函数可被多个字段/配置类型复用；无运行期自定义宏注册或公开校验 trait。
函数作者仍需保证返回的静态消息不含秘密，工具不证明任意用户代码无副作用。

所有字段规则读取已完成绑定和默认值填充的对象。按字段声明顺序执行：当前字段
的显式规则（含 each/when/rule）按书写顺序运行，然后按真实类型关系自动级联该字段
中的可分析对象；最后执行类型上的组合规则/rule，按声明顺序返回首错。when 的内部
规则不能访问未定义的 each 外部上下文；本版不允许把 compare/when 或类型级字段
组合（at_least_one_of/at_most_one_of/exactly_one_of/all_or_none）直接写进 each、
each_key 或 each_value。需要元素级跨字段关系时，在元素类型自己的字段或类型上
声明规则，无需增加 config_object。rule 内部的任意业务逻辑
不由工具重排或分析，也不承诺聚合全部失败。

自动级联见第 6.5 节。时间相对性规则 past/past_or_present/future/future_or_present、
十进制 digits、验证分组、异步 I/O、复杂组合元注解和运行期规则注册暂不列入本版：
时间规则还需冻结日期类型、时钟、时区和精度契约，digits 需要明确十进制类型，不能
用二进制浮点的小数显示冒充精确校验。已有 rule 可承接明确的业务检查；外部资源
连接仍属于相应服务初始化，不宣称格式校验已验证资源可用。

#### 5.3.3 错误路径与安全文案

自动规则失败只携带安全的结构化位置、规则标识及静态说明，不保存配置对象或被拒绝
的值。configable 直接接管的字段使用 label 后的配置键并按字面段构造，适配器再补
根 prefix。普通嵌套类型默认记录 Rust 逻辑字段段；其自身可能使用 rename、flatten
或手写 Deserialize，不能只凭 Rust 字段名推导实际输入键。只有映射已由工具合法
掌握并证明时才显示对应配置键，否则明确标为模型字段位置，不冒充可读取的配置路径。

对象位置按实际使用点追加，Vec/数组追加索引；字段名与输入键一致时，例如
app.backends[2].host。含点 label 不能拼接后重解析为多层路径。unique 只指向首次
重复元素索引，不输出元素内容。类型化读取后的模型结构可以与输入结构不同，不能
据此反推数组位置或字段在原始文件中的行列。

Map 键本身可能就是被校验的值。公开错误位置使用父字段、稳定条目序号与 key/value
角色，例如 app.headers{entry#2}.key，而不附带原始键。序号来自本节规定的有序遍历；
它是诊断定位，不是可传给 Configuration::get 的配置路径，不与静态 ConfigPath
混为同一访问语法。通用生成校验只拥有成功读取的 T，不能凭真实键反查配置来源。
后端若在读取/绑定错误中附带来源，该信息不得在 Display/Debug/source 链中泄露键。
来源字段若编码了动态键（如完整环境变量名或原始键路径），也须省略，只保留安全的
来源类别、展示名和可用行列；错误不得保留供调用方绕过遮罩读取的 raw-key 字段。

安全错误构造仍可服务于内部规则及高级手写 factory，拟议入口如下：

```rust
// 为防止运行期值被插进默认诊断，第一版仅接受静态规则文案与相对路径。
impl ValidationError {
    // 点路径；空串定位当前 section。
    pub fn new(path: &'static str, rule: &'static str) -> Self;
    // 显式对象字段段；空切片定位当前 section。
    pub fn at_fields(path: &'static [&'static str], rule: &'static str) -> Self;
}
```

该错误类型不参与规则发现，也不要求配置结构体实现某个 trait。初版单次校验返回
首错，不宣称已聚合全部错误。自动集合规则内部增加带类型的位置段以表达索引和
Map 条目；手写工厂仍可使用上述静态位置入口。动态位置的精确公开 Rust 表示在
第 11 节选型，不以含糊的点路径字符串伪装数组索引或 Map 键。
`new("a.b", rule)` 与 `at_fields(&["a", "b"], rule)` 定位嵌套字段；
`at_fields(&["a.b"], rule)` 定位含点的字面 key，不得混为同一节点。两种入口都转换为
统一的结构化诊断位置；不因此要求所有后端实现官方 ConfigPath 读取接口。非法快捷
点路径转换为安全的路径错误，不猜测另一节点的位置。

生成适配器使用 schema prefix 补齐逻辑路径，保留规则、声明和消费位置。成功的
get<T> 只返回 T，所以随后生成的校验错误默认没有配置文件行列、来源集合或覆盖历史。
后端读取/反序列化失败可以在安全 ConfigError 中提供已知的可选来源信息；这不代表
成功读取后工具链仍能获得它。不能为此要求 explain/source_for/get_value 或隐式侧通道。

高级手写 factory 必须显式提供逻辑路径上下文，框架不能从 ValidationError 或多次
读取中猜测；具体安全封装入口留在第 11 节选型。对象级或跨字段约束使用空相对路径。
实际配置来源可缺省，默认值字段不伪造原始位置；手写流程自行掌握的来源也只能作为
经安全处理的可选信息，不升级为公共后端的必备能力。

固定规则字符串由业务作者负责不包含秘密；该接口不接收 `format!(...)` 产生的运行期
自由消息，也不把任意 Error 透明转成配置错误。自定义代码主动输出或 panic 的边界见第 9 节。

## 6. `#[configable]` 配置声明

### 6.1 常用语法

```rust
use nestrs::configable;

#[configable(prefix = "app.database")]
pub struct DatabaseOptions {
    #[not_blank]
    pub host: String,
    #[min(1)]
    pub port: u16,
    #[default(16)]
    #[min(1)]
    #[max(128)]
    pub pool_size: u32,
    #[default]
    #[compare(le = pool_size)]
    pub min_idle: u32,
    #[hostname]
    pub replica_host: Option<String>,
}

#[configable(prefix = "app.http")]
pub struct HttpOptions {
    pub port: u16,
    #[default]
    #[each(url)]
    pub allowed_origins: Vec<String>,
}
```

`configable` 组合自动 Deserialize、默认 `deny_unknown_fields`、配置地址及校验声明，不再
提供同义的 `configuration_properties` 属性。配置类型保持普通数据结构，不附加
`injectable`，不把数据字段改成 `Injection<T>` 或秘密包装类型。用户请求标准 Debug
时，含 sensitive 字段的配置按第 9 节生成受控 Debug；未请求则不自动新增该实现。
不自动生成 Clone/Serialize。

| 参数 | 默认 | 约定 |
| --- | --- | --- |
| `prefix` | 必填 | 字符串；空串明确表示根，非空为对象路径 |
| `deny_unknown_fields` | true | 拒绝该分组的未知字段，可显式 false |

校验不是 configable 参数；有声明规则就由工具链自动处理，没有则跳过额外校验。
validate = true/false 与 validate_with 不属于本版声明，使用时应诊断而非忽略。

第一版只接受模块作用域内、具名字段、无泛型参数的结构体；泛型、枚举、元组结构和
借用字段等扩展不写成默认支持。绑定结果需要 `DeserializeOwned + Send + Sync + 'static`。
重复参数、未知参数、错误字面量应定位用户属性报错。

### 6.2 配置模型不是服务

configable 只声明带 prefix 的配置数据模型，不生成 DI provider。它没有
Singleton/Scoped/Transient 生命周期、服务 key/primary，也不接受服务级 lazy；
不会创建 Injection<DatabaseOptions>。普通嵌套类型因字段引用参与校验，不需要
另一种配置声明，不因可达关系获得服务身份、独立 prefix 或服务策略。

声明模型的配置消费一律使用 config_value。工具链应对 inject/普通服务查询请求
configable 类型给出针对性诊断，提示配置值绑定入口。仅仅“不自动生成 provider”
不足以实现这个规则：将配置声明类型同时标为 injectable，或用手写 factory 将其作为
服务成功类型注册，也拟诊断为角色冲突。普通服务可以包含配置值，普通未标记数据类型
的既有 DI 能力不受这条配置声明规则影响。普通类型即使被根模型引用或声明了校验
规则，也不会因此全局禁止服务注册；但普通服务注入不会获得配置绑定语义。诊断阶段
及真实类型检查列入第 11/12 节。

拟议绑定流程由消费者适配器连接：

```text
DI 交付所选配置服务输入（分派方式待决）
    → 后端 get_required::<完整所属模型>(prefix) 完成绑定和默认值
    → 有声明时执行完整 schema 校验 → 交付普通对象/字段 → 完成消费者构造
```

即使只选择一个字段，也绑定和校验完整所属模型，不跳过其他必需字段、compare、when
或类型级组合规则。没有规则时省略校验，绑定检查保持不变。

配置错误归入实际消费者的构造失败，沿用其 owner 的失败传播/清理规则；不虚构配置
Singleton 的独立失败缓存。对象是否缓存、重复绑定次数与所有权基线见第 7.5 节。

消费者在 root/scope 初始化中被选中时，其配置绑定也会执行；只主动初始化读取门面
并不自动绑定所有 schema。未使用的声明仍接受编译期检查，但没有“启动时执行所有
默认值/规则”的承诺。若要集中校验全部或特定模型，须另定配置准备流程；这是待评审
能力，不能恢复配置 provider 或继续让配置模型沿用 #[lazy(false)]。

公共 get/get_required 与官方扩展 bind 保持第 5.1 节的语义，不按用户手写的 Validate
发现规则。手写服务 factory 可自行组织配置读取与检查；不为 configable 对象创建
第二种服务交付入口。
config_value 暂不扩展为 factory/constructor 参数 helper，详见第 7.5 节。

### 6.3 字段声明：default 与 label

配置字段采用以下绑定/诊断规则，以及第 5.3 节的校验规则；业务不需要学习或直接
标注 Serde helper：

| 写法 | 拟议含义 |
| --- | --- |
| 非 Option 字段，无 default | 缺失即报错，不因为字段类型实现 Default 就自动填充 |
| Option<T> 字段，无 default | 缺失为 None |
| `#[default]` | 缺失时使用字段类型的 Default::default()，要求真实字段类型实现 Default |
| `#[default(16)]` | 缺失时求值一个 Rust 表达式，结果按字段类型检查 |
| `#[label("pool-size")]` | 使用指定的精确配置键，Rust 字段名保持不变 |
| `#[sensitive]` / `#[sensitive("******")]` | 控制配置对象的受控 Debug，详见第 9 节 |
| 字段/类型校验规则及自动级联 | 由工具链统一收集，参数、组合与错误位置见第 5.3 节 |

标签、默认值与敏感标记相互独立，可以组合：

```rust
#[configable(prefix = "app.database")]
pub struct DatabaseOptions {
    #[default(String::from("localhost"))]
    pub host: String,

    #[label("pool-size")]
    #[default(16)]
    pub pool_size: u32,

    #[default]
    pub min_idle: u32,

    pub replica_host: Option<String>,

    #[sensitive]
    pub password: String,
}
```

本例是第 6.1 节类型的替代声明，不是同时定义的第二个同名配置类型。第一版不提供
字段上的 configable(default/name)、config_field 或通用 serde(...) 作为同义写法。

本节 default 与 label 由外层 configable 消费，不单独导入，不是能独立应用于
任意字段的过程属性宏。支持裸名与 nestrs::default / nestrs::label 路径。普通嵌套
类型按自身 Deserialize 绑定；无外层宏的 default/label 生成入口尚未设计，不能
因为自动发现校验就宣称这些绑定属性已在所有可达类型上生效。
configable 放在请求派生的属性之前，展开时消费这些标记，不留给原生派生误解。
Rust 枚举变体上的原生 #[default] 仍由 Rust 的 derive(Default) 解释；工具不能按名字
全局拦截或改变它。必须覆盖真实展开、派生顺序与作用域隔离，不以文档约定代替检查。

default 的具体规则：

- 裸标记使用类型默认值；带括号时接受一个非空 Rust 表达式，可带尾逗号。空括号、
  多个表达式及同一字段重复 default 都报错；元组值须写成一个元组表达式。
- 表达式在生成的同步默认函数中，以该字段类型为预期类型，遵守原生名称解析、
  可见性和宏卫生。没有 self、其他配置字段或调用方局部变量的隐式上下文，不注入
  DI 输入，不隐式 await，也不自动添加 Into、From 或字符串解析。
- `#[default(16)]` 可用于 u32；String 用 `#[default(String::from("localhost"))]`。
  `#[default("localhost")]` 的值是 &str，不能据此自动填入 String。函数默认值写
  `#[default(default_pool_size())]`，不把字符串当作函数名或自动调用函数项。
- 只在运行期绑定确实需要该缺失字段的默认值时求值；字段有合法输入时不执行，
  配置内容或类型错误时不作为兜底。显式 null 不是缺失；Option 可绑定为 None，
  其他类型按自身规则接受或拒绝。显式 default 优先于 Option 的缺失 None 规则。
- 每次绑定是独立构造，不全局缓存默认表达式。默认函数可以包含用户代码；不保证
  其纯度或跨绑定结果恒定，不规定各缺失字段的默认求值顺序，也不承诺失败绑定
  从未执行其他字段默认值。check/build/graph 不为绑定执行默认表达式。
- 标记只控制绑定缺失值，不自动为结构体实现 Default，也不改变用户另行请求的
  derive(Default) 或手写 Default 的语义。缺失必选 section 仍按第 5.1 节报错，
  不能通过字段默认值自动创建分组；生成的默认值不回写原始配置树。

label 只接受一个字符串字面量（含原始字符串），允许尾逗号；裸标记、空括号、
非字符串、多参数或重复 label 报错。它替换当前字段在该对象中的键名，不是前缀、
路径表达式、输入别名或显示标题。相同结构体中两个字段的最终键名冲突必须诊断；
含点、括号或空键名仍是一个字面键，不能拆成路径，快捷地址限制见第 7.2 节。
原 Rust 字段名不会隐式保留为输入别名。label 不修改 Rust 字段名、Debug 字段名或
显式 Serialize 的字段名。

cfg/cfg_attr 以真实展开后仍存在的字段和属性为准；被排除的字段不贡献绑定规则、
默认表达式、键名冲突或字段地址。不相关的其他派生及其 helper 不因本节被全局删除。

### 6.4 Serde 边界与生成路径

用户仍可添加 Clone、Serialize 等其他 derive；configable 负责 Deserialize，常规
用法不再次派生。标准 Debug 与 sensitive 的协作、真实身份和顺序要求见第 9.2 节。
明确重复的 Deserialize/配置声明给出诊断；不承诺识别所有重命名 Deserialize derive
别名，隐蔽冲突仍由 rustc 检查。其余 derive 保留，不能因脱敏悄悄改变用户序列化语义。

configable 类型本身及其直接字段上不接受用户编写的 serde(...) helper，发现后明确
诊断，不静默忽略或透传。字段规则仅采用第 6.3 节；未知字段策略只由 configable 的
deny_unknown_fields 参数控制。第一版不开放 flatten、skip、alias、rename_all、
with、deserialize_with、from/try_from、transparent 或 remote 等等价声明，也不把
仅用于序列化的 Serde helper 作为例外。关闭 strict 只允许未知输入键，不开放这些能力。

嵌套对象、数组、Option 和字符串键 Map 仍是普通配置数据结构。普通嵌套类型使用
自身合法的 Deserialize（例如正常 derive 或手写实现），工具链按真实字段关系
连接已经合法收集的校验，详见第 6.5 节。自动级联不为这些类型新增或替换
Deserialize，不递归接管其 Serde 用法、默认值、字段映射或未知字段策略；根模型的
strict 不自动传播到普通子对象。它也不会凭一个名为 validate 的方法发现规则。

外部类型同样按自身 Deserialize 绑定；类型内部是否可继续校验由合法声明能力及
metadata 决定，不能因实现了 Deserialize 就承诺能穿透其私有结构。仍须满足真实
trait、生命周期及第 5.2 节文本转换边界。完全自定义映射通过普通类型和手写 factory
接入，不为 configable 的直接字段重新开放通用 Serde helper。

拟通过独立于具体客户端的共同运行期支持层 re-export Serde，生成 derive 与
`serde(crate = ...)`；该支持层是否独立成 package 尚待定型。
两者必须指向同一个真实依赖身份。实现必须覆盖 Cargo rename、库内 crate 路径和跨 crate
使用，不能硬编码所有应用都存在 `::nestrs_config` / `::serde`。供下游生成代码使用的
re-export 若需 public，就明确是工具支持路径，不能把 doc-hidden 描述成 Rust 私有。
它不暴露 core 的内部执行 ABI。工具为 default/label 生成必要的绑定适配；这些内部
Serde 设置不是业务声明入口，须能区分用户原始 helper 与工具生成 helper。生成的
label 映射只作用于 Deserialize；用户额外请求 Serialize 时仍保留其 Rust 字段名，
不能意外同时改写输出协议。公共 get/get_required 的类型约束及官方扩展
bind<T: DeserializeOwned>() 都继续使用 Serde，不宣称公开读取契约已与 Serde 解耦。

### 6.5 普通嵌套对象与按类型关系自动级联

本轮移除 `#[config_object]`。根模型使用带 prefix 的 configable；内部对象保持普通
Rust 类型，由工具链按真实字段类型自动发现和连接深层校验。不增加 nested、Valid、
ValidateNested 等逐层启用标记，也不把无 prefix 的 configable 作为替代写法。
对象字段上的 rule 表达实际业务约束；即使不写 rule，子对象已声明的规则也必须级联。

#### 6.5.1 目标用法与执行关系

以下是独立的拟议示例。根 configable 生成自己的 Deserialize；普通嵌套类型在这里
用正常 Serde derive 提供绑定能力。普通类型上的校验属性还需要第 6.5.3 节的新声明
入口，示例不能视为原版 Cargo 或当前 Nestrs 已可编译的代码：

```rust
use nestrs::configable;
use serde::Deserialize;

#[configable(prefix = "app.database")]
pub struct DatabaseOptions {
    pub pool: PoolOptions,
}

#[derive(Deserialize)]
pub struct PoolOptions {
    #[min(1)]
    pub max_size: u32,

    #[compare(le = max_size)]
    pub min_idle: u32,

    pub retry: RetryOptions,
}

#[derive(Deserialize)]
pub struct RetryOptions {
    #[min(1)]
    pub attempts: u32,
}
```

config_value 先通过后端取得完整 DatabaseOptions，再执行 PoolOptions 的字段规则，
继续进入 RetryOptions。DatabaseOptions 没有本地规则也必须深入；所有可分析的
可达类型均无规则时才省略额外校验。普通类型只需要声明实际约束，无需配置类型标记
或手写 Validate。这里的 Deserialize 是绑定能力，不是开启校验的标记。

类型图按真实身份分析，重复使用同一类型可复用校验代码，但运行期每个字段/集合元素
中的实际值仍须检查，不能用“这个类型已经访问过”跳过第二个对象。规则默认作用于
完整绑定后的 Rust 值；顺序、首错和 when 的局部门控继续遵守第 5.3.2 节。

#### 6.5.2 自动遍历的范围

| 字段类型 | 拟议级联行为 |
| --- | --- |
| 可分析的普通嵌套 struct | 按真实字段继续深入，不要求 config_object；本层无规则也分析后续对象 |
| Option<T> | None 跳过内层对象校验，Some 检查实际 T；字段 required 的语义不变 |
| Vec<T> / [T; N] | 按索引检查每个实际元素，空集合不凭空构造元素 |
| 受支持的字符串键 Map | 按稳定条目次序检查值，错误用条目序号和角色定位，不泄漏动态键 |
| 以上容器的组合 | 继续展开受支持的实际类型，保留使用点、索引及条目角色 |
| 嵌套 configable 类型 | 检查父对象中已绑定的实例；该次使用不采用其独立 prefix，也不额外读取或创建实例 |
| 标量、String 及不提供可组合规则能力的外部值类型 | 作为叶子；按字段显式规则检查，不遍历其实现细节 |

类型可分析性不能仅以“能看到字段”判断。上游库须提供合法的类型关系、规则及执行
能力；URL 等外部类型的私有内部结构不因可达就进入自动遍历。普通类型的已有绑定
协议继续有效，可在父字段用 rule 补充对象整体的业务检查；它不是级联启用开关。
已声明规则却缺少合法执行能力，或已知规则需要经过不支持的容器时，须明确诊断，
不能静默当作无规则。把外部值作为叶子也不等于证明其内部全部通过 Nestrs 校验。

第一版自动展开普通具名字段、无借用且无自定义泛型参数的结构体，及上表受支持的
容器；其他类型形状仍须明确支持后才能展开。有限的深层嵌套使用工作栈和资源预算，
不只检查一层，也不把配置深度直接变成调用栈深度。分析图中出现递归类型环时，
第一版仍明确拒绝；同一类型在多个字段复用不等于类型环。任意递归 schema、Box/Arc
图的循环检测等没有因此自动纳入支持范围，不以运行期死循环或静默截断处理。

错误按实际使用点组成，例如本例字段与输入键一致时为
app.database.pool.retry.attempts。普通类型若通过 Serde 或手写转换改变输入形状，
逻辑模型字段位置不能冒充配置输入地址，见第 5.3.3 节。没有后端来源元数据时也不
推断文件行列或覆盖历史。

自动深入校验不递归创建 config_value 字符串地址。第 7 节仍只由 configable 的
prefix 和直接字段贡献地址；本例可选择整个 pool 字段，但不会据此新增所有更深层
字段的地址。普通嵌套类型不独立贡献根地址或类型字段选择器。取得整个字段时仍须
满足第 7.5 节的 Clone 约定；根对象交付、重复绑定及缓存待决项不因本轮改变。

#### 6.5.3 普通类型属性接入与绑定边界

“找到一个嵌套类型”“合法收集它的规则”“生成 Deserialize/Debug”是不同能力。
根 configable 过程宏只能直接处理自己的输入，不能天然取得另一类型、文件或 crate
的全部字段声明。普通 struct 中的裸 min/compare 等属性不会因稍后的类型分析就
变成合法 helper；写成 nestrs:: 路径也不会自动解决标准 Rust 的属性处理问题。

为了提供不加外层配置标记的目标语法，工具链需要新增声明采集/消费机制。实现前
必须确定它在标准宏展开中的位置、属性身份及生成载体，覆盖 cfg/cfg_attr、外部
模块、macro_rules 生成项、其他属性和 derive 顺序。不能等未知属性已经被拒绝后
才声称后期分析可以修复；也不扫描源码文本、不依赖可变宏全局表、不注册 nestrs
工具属性或替换原生宏管线。短名与完整路径的身份检查不能接管其他宏拥有的同名规则。

规则及可组合级联能力在声明所属 crate 内生成并类型检查，库自身编译不要求下游根
模型已经出现。跨 crate 元数据需要携带必要类型关系、规则与合法调用能力，具体
采集范围和表示待定。下游只能组合已有能力，不能回写已编译 rlib、提升字段可见性、
为外部类型补固有 impl 或绕过借用检查。缺失能力须报告，不能靠运行时反射补齐。

| 能力 | 本轮边界 |
| --- | --- |
| configable 根模型绑定 | 继续由其宏生成 Deserialize，处理 default/label/strict；要求 prefix |
| 普通嵌套类型绑定 | 使用自身 Deserialize，不自动新增或覆盖；其 Serde 规则由该类型负责 |
| 普通类型校验规则 | 目标是无需外层配置标记，由新的合法声明入口收集并按类型关系连接 |
| 普通类型的 default/label | 自动校验不赋予这两个字段 helper 绑定语义；无外层宏的绑定生成入口仍待另行设计 |
| 普通类型的 sensitive/Debug | 不因可达而接管已有 Debug；精细字段脱敏的合法生成入口仍待设计，不能悄悄忽略已声明标记 |

普通类型可继续通过自己的反序列化实现处理缺失值或映射；这不开放 configable 根类型
上的任意 Serde helper。根字段可使用 sensitive 遮罩整个嵌套对象，见第 9 节。
本轮不新增替代类型宏，也不把自动规则发现扩大成对所有可达类型的绑定或格式化改写。

公共 get<T>/get_required<T> 及官方 section.bind<T>() 继续只执行 Deserialize，
不自动调用独立生成的根/子对象校验；config_value 负责连接整条自动校验流程。

## 7. 消费配置：服务注入与配置值绑定

### 7.1 对象与字段统一使用 config_value

inject 只用于服务依赖；配置模型和字段统一使用 config_value。以下保留配置消费的
目标语法；背后的所选配置服务输入及分派方式仍待第 2.5 节评审，不写入 dyn 字段：

```rust
use nestrs::injectable;

#[injectable]
struct OrderService {
    // 普通配置对象，不是 Injection<DatabaseOptions>。
    #[config_value("app.database")]
    database: DatabaseOptions,

    #[config_value("app.database.host")]
    database_host: String,

    #[config_value("app.database.replica_host")]
    replica_host: Option<String>,

    #[config_value("app.http.allowed_origins")]
    allowed_origins: Vec<String>,

    #[value("orders")]
    service_name: String,
}
```

此例并列展示对象及字段请求，业务按需选择。工具链负责建立所需的配置服务依赖，
不要求业务再写一个仅供 config_value 使用的 inject 字段；具体输入类型尚未定型。
主动调用两个泛型方法的静态用法见第 5.1 节，不能推导出 dyn 注入已经可用。

config_value 是 injectable 消费的字段 helper，支持裸名及 nestrs:: 路径，不单独
导入或作为独立字段过程宏运行。所有配置字段保持原有 Rust 数据类型。配置模型的
injectable、inject、普通服务查询与手写 factory 注册冲突按第 6.2 节诊断。

字符串形式是静态声明地址，可以选择根配置对象或直接字段，不是任意运行期树查询。
保留 config_value(DatabaseOptions::host) 作为显式字段选择器；两种字段形式使用
相同的 schema、读取门面依赖、绑定和校验过程。旧方案中“类型形式可从任意普通
factory 服务取字段”的能力撤下；只有配置声明类型贡献这套语义。

例如 app.database.pool_size 映射到 DatabaseOptions::pool_size。部署文件省略该
字段时，声明地址仍成立；只要必需分组存在且整个对象绑定、校验成功，就能取得其
声明默认值 16。直接 get_required::<u32>("app.database.pool_size") 不会自动使用
这个字段默认值，也不查询声明目录。敏感字段交付真实值，不能用遮罩代替。

### 7.2 对象、字段地址与 label

目录由 configable 的真实声明贡献，保存分段路径、真实类型、目标种类及字段身份，
不把字符串作为 Rust 类型身份。普通嵌套类型不因自动级联而贡献根地址。

| 声明情况 | 字符串地址规则 |
| --- | --- |
| configable 的 prefix | 对应整个根配置模型；如 app.database 选中 DatabaseOptions |
| 普通直接字段 | prefix + Rust 字段名；raw identifier 去掉语法前缀 r# |
| 字段 label("pool-size") | 使用该字面输入键；不保留 Rust 字段名作为隐式地址别名 |
| prefix = "" | 空字符串地址选中整个根模型；字段地址不添加开头的点 |
| default / sensitive | 不改变地址，不在编译期执行默认表达式或用遮罩代替值 |
| 直接对象/Vec/Map 字段 | 可选中整个字段；不递归生成内部字段、索引或动态键地址 |
| 普通服务/factory 类型 | 不提供 config_value 地址，也不接受类型字段形式绕过 schema 声明 |

空字符串选择根对象是本轮新增约定，替代旧版“空地址不表示整个对象”。label 只改变
字段输入名和字符串地址，类型选择器仍使用 Rust 字段名。例如 db.listen-port 与
DbOptions::port 可以选中同一个字段。

同一类型重复 label 或重复最终键名必须诊断；不同模型的地址冲突按下一节处理。
尤其是某模型的 prefix 可能与另一模型的直接字段地址相同：不能按目标类型区分后
偷偷选中一个。对象/字段候选共同参与歧义检查，诊断须标出两者种类和声明位置。

含点、括号或空字段名不适用快捷点路径地址，使用类型字段形式，不把一个字面 key
拆成多个路径段。第一版仍不递归展开嵌套地址；直接字段本身为对象时可以取得它的
整体值，其内部规则由所属根模型的自动级联执行。

### 7.3 编译期解析与真实服务依赖

```text
config_value("app.database.port") / config_value(DatabaseOptions::port)
    → 静态选择 DatabaseOptions schema 与 port 字段
    → 消费者取得所选配置服务输入（分派方式待决）
    → 后端 get_required::<DatabaseOptions>("app.database")
    → 消费者适配器执行完整模型校验
    → 取得 port 的普通值

config_value("app.database")
    → 静态选择 DatabaseOptions 根模型
    → 同一读取/绑定/校验流程
    → 交付整个普通 DatabaseOptions 值
```

1. 真实 cfg/feature 决定声明与请求；保留的请求即使未运行也必须通过地址和类型检查。
2. 解析域是消费 crate 与实际依赖贡献的声明目录，不是进程级可变注册表。library 在
   自身编译时选定真实模型/字段，生成合法 typed adapter；不能留到运行时猜类型。
3. 地址零候选报不存在，多候选报歧义；不能按消费者字段类型、可见性、声明顺序或
   服务 primary 隐式筛选。重复导出的同一声明幂等去重，不合并不同模型。
4. 仅被引用的地址要求唯一；未引用地址重名不自动增加独立错误。显式类型字段形式
   可消除字段来源歧义。整对象类型选择器 config_value(DatabaseOptions) 暂列待评审，
   本版不将其作为已确定语法；根地址歧义仍须调整声明或等待该扩展定型。
5. 所选配置服务的候选选择与 schema 地址解析是两套检查。分派方案须明确输入类型
   及候选身份，再将真实依赖接入既有 DI 图的缺失、歧义、环、key 和生命周期检查；
   当前不能把泛型 ConfigService 直接用作 dyn 自动绑定。schema 不是服务节点，
   不能用服务 primary 解决其地址冲突。缺失部署字段属于运行期绑定或默认值处理。
6. 上游生成自己的绑定/校验能力；消费 library 解析请求；最终应用提供门面实现和
   部署数据。下游声明不重定向上游选择、不回写 rlib，不提升业务类型/字段可见性。
7. 继续使用发现/生成与跨 crate metadata，不扫描源码、不用可变宏全局表、不读取
   部署值决定编译图，也不要求普通 proc-macro 独自跨 crate 发现全部类型。

生成适配器读取激活器已经交付的门面输入，不在 worker 内调用动态 resolve。缺失门面
或依赖错误不能被 Option 配置字段隐藏；字符串形式不获得额外私有访问权限。

### 7.4 显式类型字段选择器

DatabaseOptions::host 是属性定义的字段选择语法，不是求值 Rust 关联项。最后一个
路径段为真实实例字段，其余部分按 Rust 类型位置解析；只接受具名字段，不猜来源。

| 写法 | 含义 |
| --- | --- |
| config_value(DatabaseOptions::host) | 选择该 schema 的 host 字段 |
| config_value(crate::settings::DatabaseOptions::host) | 完整类型路径与字段 |
| config_value(DbOptions::host) | 合法类型别名/use 别名，按真实类型解析 |
| config_value(DatabaseOptions::r#type) | raw identifier 字段 |

类型形式始终使用 Rust 字段名，不受 label 或消费者字段名影响。类型必须具有有效
configable 声明；不能用普通 factory 成功类型代替，也不因此开放泛型配置模型。
不支持点式字段链、方法调用、索引、元组字段、限定关联类型表达式或从裸标记猜来源。
同名关联常量/函数不作为字段缺失时的回退；保持准确 Rust 类型、可见性与宏卫生。

### 7.5 交付、所有权与构造顺序

每个属性只接受一个字符串字面量或类型字段选择器，可带末尾逗号。不接受变量、
concat!、插值、通配符、命名参数或地址内默认值；没有声明时不回退原始树读取。
同一字段不能混用 config_value 与 inject/value/字段 lazy。第一版仍限自动字段
构造模式，不扩展 factory/constructor 参数；有 constructor 的类型继续拒绝混用。
补全、跳转及重命名均需真实 IDE 验证，不能只因语法接近 Rust 路径就宣称已支持。

删除配置 provider 后，旧的 Singleton 实例/lease 复制语义不再适用。为使本稿可评审，
先提出以下最小交付基线，重复绑定与缓存取舍仍列入第 11 节确认：

- 每个 config_value 请求通过后端 get_required::<所属模型>(prefix) 独立构造完整对象，
  再执行生成的校验；不消费公共原始节点。不引入全局 typed cache，
  也不承诺同一消费者中的多个请求只绑定一次；default(expr)/rule 可随绑定重复执行。
- 整对象请求直接按值交付新构造的对象，不要求整个对象 Clone，不生成 Injection<T>。
- 字段请求在完整绑定及校验后沿用字段 Clone，仅要求被选字段 Clone；不要求根模型
  Clone。此提议保留此前字段契约，也避免通过字段移动给实现 Drop 的模型增加限制。
- 若以后共享已绑定对象，必须明确缓存键、范围、失败、重复规则执行和拥有所有权的
  交付方式，不能同时承诺任意非 Clone 整对象按值交付以及单份共享 typed 实例。
- 目标类型准确匹配，不隐式 Into，不伪造长期借用；Option 只是数据类型。门面缺失、
  section 缺失或绑定失败不因此变成 None，sensitive 不改变真实交付值。
- 所有 DI typed 输入读取并 ensure_all_consumed 后，才按消费者字段源码顺序执行
  配置读取/绑定/校验/复制、Default 或 value；cfg 排除字段不执行这些操作。失败时
  不交付消费者，但已经执行的用户默认值/规则/Clone 副作用不承诺回滚。
- 真实依赖及关闭顺序指向读取门面，遵守既有 lease/owner 协议；不会因为字段已拥有
  一份值就删掉必要依赖边，也不新增一个隐藏配置对象 provider。
- 同一 root 的 Singleton 门面提供固定原始版本，但独立绑定产生的对象不保证身份
  相同；用户默认表达式也未必纯。未来热更新不会自动改写已经交付的普通字段值。

### 7.6 与通用读取 API 的边界

| 写法 | 行为 |
| --- | --- |
| 所选配置服务 + get_required::<u16>(path) | 后端按动态路径转换成 u16，不查 schema 默认值或规则；输入分派仍待定 |
| get_required::<DatabaseOptions>(path) | 后端执行生成 Deserialize 的 default/label/strict，不自动调用独立配置校验 |
| 官方 section.bind::<DatabaseOptions>() | 官方扩展，同样只反序列化；不属于公共 trait |
| config_value("app.database") | 静态选择模型，完整绑定及校验后交付对象 |
| config_value("app.database.port") / config_value(DatabaseOptions::port) | 静态选择所属模型，完整绑定及校验后取得字段 |
| inject DatabaseOptions / 服务查询 DatabaseOptions | 配置角色误用，工具链应诊断，见第 6.2 节 |

## 8. 与现有启动流程组合

下例展示未来如何用已有 factory 形式准备官方客户端。Configuration 及加载 API
已在独立库实现；客户端实现公共泛型 trait 后如何交付给无后端类型的
消费者，仍受第 2.5 节分派决策约束。真正客户端拟作为默认 key、Singleton 服务提供，
普通 factory 默认生命周期即为 Singleton：

```rust
use nestrs::{factory, lazy};

#[factory]
#[lazy(false)]
fn configuration() -> Result<Configuration, ConfigError> {
    load_configuration("production")
}
```

Configuration 实现泛型 ConfigService 并不会使它能自动投影成 dyn ConfigService；
现有 trait 自动绑定不能完成这一步。第三方可以实现同一两方法契约并替换加载逻辑，
但怎样替换 DI 输入、是否传播后端类型以及如何连接上游消费者，必须随分派方案定型。
库内 schema 与消费声明不依赖具体客户端 API 是设计目标；当前不能把“只换一个
factory 就完成集成”写成已经成立的保证。

客户端 factory 可以是同步或异步。上例的 lazy(false) 仅保证客户端在 root 创建时
完成准备；它不触发全部 schema 的绑定。消费者被 Eager/服务标记选中或被实际查询时，
其 config_value 才执行对应绑定及校验。希望某个消费者在启动时验证配置，可以对
那个真正的服务使用 lazy(false)，但不能给 DatabaseOptions 使用服务策略。

运行期顺序是客户端准备 → 消费者适配器绑定/校验 → 消费者交付；服务图只表达消费者
对读取门面的依赖，schema 可作为配置元数据解释，不能伪装成服务节点。失败按客户端
或消费者所属阶段进入 BuildError/ResolveError，并沿用未交付 owner 的清理规则。

以下只展示现有 DI 的创建、查询与关闭结构。OrderService 中的配置字段需要未来
配置分派及生成能力完成后才能接入，不能用此示例证明新 trait 已可注入：

```rust
use nestrs_core::{ResolveError, ServiceProvider};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::build(None).await?;
    let operation = async {
        let service = provider.get_required_service::<OrderService>().await?;
        println!("{}", service.service_name);
        Ok::<(), ResolveError>(())
    }.await;
    let disposal = provider.dispose_async().await;
    match (operation, disposal) {
        (Ok(()), Ok(())) => Ok(()),
        (operation, disposal) => Err(format!(
            "应用操作结果：{operation:?}；关闭结果：{disposal:?}"
        ).into()),
    }
}
```

此处的 await 属于 DI 服务构造/查询，不证明配置读取接口异步。未来验收必须分别覆盖
客户端准备、公共读取、宏绑定及真实 cargo nestrs 编译集成；独立反序列化或 Rust
语言探针都不能代替这些功能测试。

## 9. 敏感配置与脱敏设计

第一版采用字段标注表达敏感性，业务字段保留其真实 Rust 类型。脱敏控制配置对象的
诊断表示，不改写配置值、Serde 绑定、业务校验或 DI 交付。用户无需声明 SecretString
或调用 expose_secret；本版不再把该秘密包装类型作为公开 API 交付目标。

### 9.1 业务声明：sensitive

主要写法如下：

```rust
use nestrs::configable;

#[configable(prefix = "app.database")]
#[derive(Debug)]
pub struct DatabaseOptions {
    pub host: String,

    #[sensitive]
    pub password: String,

    #[sensitive("******")]
    pub token: String,
}
```

本例是前文 DatabaseOptions 的替代示意，不是第二次声明同名配置类型。

| 写法 | 拟议含义 |
| --- | --- |
| `#[sensitive]` | 配置对象的受控 Debug 使用默认固定替代文本 `"[REDACTED]"` |
| `#[sensitive("******")]` | 同一位置使用自定义固定替代文本 `"******"` |
| 未标记字段 | 按字段自身的 Debug 显示，保留普通 Debug 约束 |

本节 sensitive 由 configable 消费，不单独导入，也不是可独立用于任意字段的过程
属性宏。支持裸名及 nestrs::sensitive 路径。普通嵌套类型的脱敏生成入口仍待设计，
不能把第 6.5 节的自动校验当作自动接管 Debug 的授权。第一版只接受裸标记或一个字符串
字面量（含原始字符串字面量），有参数时允许末尾逗号；空括号、重复标记、非字符串、
额外参数、命名参数或求值表达式均报错。空替代字符串也按固定空字符串处理。

替代文本是完整的固定内容，不是格式模板、正则或“按原值长度重复”的字符。大括号、
引号和换行按普通字符串 Debug 规则转义；不进行插值或执行函数。作者负责不把真实
秘密写进该源码常量。字段的 raw identifier、cfg 和 cfg_attr 按真实展开结果处理，
被 cfg 排除的字段不贡献敏感标记、格式化操作或 trait 要求。

标记作用于整个字段，字段类型可以是 String、数字、Option、集合或嵌套对象。被标记
字段在受控 Debug 中始终输出替代文本，不显示其 Some/None、长度、元素个数或内部
结构，也不要求该字段为此实现 Debug。未标记的嵌套对象遵循它自己的 Debug；仅在
另一个类型上写过 sensitive，不代表所有外层类型会自动获得脱敏能力。

### 9.2 与 derive(Debug) 的协作

标准 Rust derive(Debug) 本身不理解 sensitive。工具需要接管用户明确请求的标准
Debug 派生，为配置对象生成按字段替换的 Debug impl，同时保留 Clone 等其他 derive。
不先格式化秘密再替换，而是直接用替代字符串代替该字段；格式化过程不读取被遮罩
字段，也不调用其 Debug。未标记字段继续正常格式化。

第一版约定：

1. 常用顺序为本节示例中的 configable 在上、derive(Debug) 在下。带敏感字段的类型
   请求标准 Debug 时，必须只保留一份受控实现，不留下原始派生再额外生成第二份。
2. 支持常用裸 Debug，以及明确的 core::fmt::Debug / std::fmt::Debug 路径（可含
   开头的 ::）。必须确认真实标准派生宏身份，不能只凭最后一段叫 Debug 就认定
   是标准宏。同名自定义派生或无法确认身份的别名需要明确诊断，不静默替换。
3. 未请求 Debug 就不自动新增 Debug；敏感字段元数据仍保留。configable 不因该标记
   自动生成 Clone、Display 或 Serialize，也不把字段改成包装类型。
4. 若标准 Debug 已在 configable 前展开，后执行的属性宏未必能从剩余 token 判断
   这一事实。工具链必须检查敏感配置类型的实际 Debug 实现来源，拒绝这种未接管
   的实现并提示调整顺序；不能仅在文档要求顺序，却静默留下会输出原值的派生。
5. 第一版对含 sensitive 字段的 configable 类型只接受上述受控 Debug 实现，手写
   Debug 或其他派生生成的 Debug 不属于支持组合。冲突或不受控实现必须编译失败，
   不能删除用户实现、分析其函数体后猜测安全性，或任选一份继续构建。
6. 真实宏身份、cfg/derive 顺序、已展开实现来源和跨 crate 检查需要配合现有工具链，
   不是普通属性宏天然拥有的能力。必须有正反编译器契约验收；这些检查与生成目前
   都未实现，不能以 token snapshot 或文档顺序约定代替。

敏感标记不改变 label/default 或字段类型自身的输入行为，也不表示序列化时跳过
该字段。configable 不自动派生 Serialize；用户显式请求 Serialize 时，仍按字段
真实值及正常派生或用户实现序列化，label 不改写输出名，不悄悄把业务 JSON 换成
星号。其他派生输出、Display 或业务序列化不是本节受控 Debug 的脱敏接口。

普通嵌套类型维持自身 Debug；根对象自动级联校验不改变这个实现。需要遮罩整个
嵌套值时，可以在 configable 的对应字段标注 sensitive；若要在普通嵌套类型内部
逐字段生成受控 Debug，其属性接入、派生顺序和冲突策略必须另行定型。不能因移除
config_object 就静默丢弃 sensitive 或假装原生 derive(Debug) 已支持脱敏。

### 9.3 目标输出与字段读取

假设 host 为 db.example，password 和 token 保存示例秘密，格式化整个配置对象的
目标输出如下。这是尚未执行的设计示意，不是当前 library 的运行结果：

```text
DatabaseOptions { host: "db.example", password: "[REDACTED]", token: "******" }
```

普通 `{:?}` 和 pretty `{:#?}` 都只在被标记字段的位置使用替代字符串，变化的是外层布局。
空密码、短密码和长 token 不改变固定替代内容；未标记字段仍正常显示。

字段继续是普通 String，应用可以直接读取、借用、克隆和传给数据库客户端：

```rust
let password: &str = options.password.as_str();
let owned_password: String = options.password.clone();

tracing::debug!(configuration = ?options); // 受控的整对象 Debug 对敏感字段打码
tracing::debug!(password = ?options.password); // 普通 String 的 Debug，可能输出明文
```

这里的保护范围是配置对象的受控 Debug，不是所有流经程序的字符串。字符串的
Display、单独 Debug、拼接和显式 Serialize 均保持其真实类型行为。该设计不提供
查询权限、存储加密、自动清零、无副本或抵抗进程转储的保证。

### 9.4 config_value、字段元数据与 logger

两种 config_value 字段选择形式都读取和克隆原始 Rust 字段值；敏感 String 字段注入 String
是正常用法：

```rust
#[injectable]
struct DatabaseClient {
    // 也可写为 #[config_value(DatabaseOptions::password)]。
    #[config_value("app.database.password")]
    password: String,
}
```

工具的字段声明记录保存真实字段身份、sensitive 标记与替代文本，两种选择器解析后
都保留相同的来源关联。字段因 label 含点等原因没有快捷字符串地址时，标记仍属于
该真实字段，受控 Debug 和可用的类型选择器不丢失这份内部记录。

这些是工具内部的声明/来源元数据，不是 String 中的运行时标签，也不承诺沿任意
赋值传播。第一版不自动改写 DatabaseClient 的 Debug，不在 injectable 中额外接受
sensitive，也不因配置来源的标记改变普通消费者的派生 Debug。克隆到消费者后单独
打印该字段，或打印消费者自己的普通 Debug，仍可能出现明文。

| 场景 | 第一版职责与保证 |
| --- | --- |
| 格式化配置对象 | 工具根据 sensitive 生成受控 Debug；config 不需要反向依赖 logger |
| 格式化单个普通字段或消费者对象 | 按其真实类型和实现处理；来源元数据不自动拦截输出 |
| 原始配置诊断和错误 | config 自身保证安全默认表示，规则见第 9.5 节 |
| 通用日志分类、局部遮罩、过滤与输出目标 | 由未来 logger 单独设计，不是本版 config 的实现 |
| 管理端点查看原值 | 由未来端点与权限层另行定义 |

未来 logger 若要使用敏感元数据，应通过单独设计的显式结构化接口取得它；不能从一个
已经拼接好的普通 String 恢复字段来源，也不建立全局秘密字符串替换表。本版内部
元数据记录不等于已经交付 logger 集成或动态信息流追踪。

### 9.5 原始配置与完整错误链的独立脱敏

即使没有 sensitive 标记或绑定尚未成功，原始配置和错误仍遵守以下安全输出规则：

- 官方 Configuration/ConfigLayer/ConfigSection 及其内部节点的默认 Debug 省略全部
  原始值或原始文件内容，包括未标记的普通值。内置 source、builder 等持值类型若
  实现 Debug，也必须省略这些内容；也可不实现 Debug，不透过内部自定义 source 的
  格式化泄露它们。第三方自己的诊断视图由其负责，不要求导出同一组原始节点类型。
- 官方扩展 explain 只输出来源与覆盖说明，不返回节点值。本文的“配置预览”指这些诊断视图，
  不额外承诺 preview/show_values API，也不按 sensitive 放开其他原始值的展示。
- ConfigError 的 Display、Debug 和完整 Error::source() 链只暴露安全结构，不
  透传原始 parser/serde/custom error 文本，不提供公开 raw_error 绕过入口。
- 错误保留类别、可用逻辑路径及已知安全位置；未来规则错误可携带静态约束信息。配置来源为可选信息，
  成功读取后生成的校验不能通用追溯文件行列，见第 5.3.3 节。不附带原始行、源码摘录、rejected
  value、连接串解析器的任意消息。普通手写或嵌套类型中的自定义反序列化错误文本
  同样不原样输出；业务校验沿用第 5.3 节的静态规则文案。
- 现有 [factory codegen](../cargo-nestrs/src/codegen/injection/macros/factory/codegen.rs)
  使用 `format!("{:?}", error)`，所以 ConfigError 的 Debug 和 cause 链不能依靠未来
  logger 再补救；直接 println、CLI 或测试失败信息也可能绕过日志层。

sensitive 不改变原始快照的读取权限，业务仍可显式调用
`configuration.get_required::<String>("app.database.password")` 取得真实值。Origin 的
文件名、环境变量名和动态键是定位元数据，本身若含秘密不属于“所有任意输入均隐藏”
的保证；调用方应为来源提供不含凭据的展示名称。但集合校验错误中的动态键及编码
该键的来源字段必须按第 5.3.3 节省略；此处的元数据边界不放宽该规则。

库不能拦截自定义 Deserialize、手写业务校验和 source 内部的输出、第三方日志或 panic hook，
也不能证明作者写入的静态规则与替代文本不含秘密。默认诊断脱敏不替代业务对明文的
使用约束，deny_unknown_fields 和自动 Deserialize 本身也不产生这项保护。

### 9.6 脱敏验收范围

以下均是未来验收要求，当前没有实现或执行：

- 默认/自定义替代文本、空文本、引号/换行转义、裸名/nestrs 路径、非法参数及重复
  标记；字段 raw identifier、cfg/cfg_attr、label/default 等组合。
- 整对象普通/pretty Debug 对被标记字段整体替换；空/短/长值、Option 和集合使用
  相同替代内容。未标记字段照常显示；被标记字段无需 Debug，已有的 Debug 也不能
  被调用，应以会 panic 或计数的字段 Debug 实现作为控制用例。
- 标准 Debug 的真实宏身份、正确顺序、提前展开、同名遮蔽、手写/派生实现冲突、
  无 Debug 请求、与其他 derive 共存，以及真实跨 crate/rustdoc/IDE 行为。普通嵌套
  类型不被自动改写；根字段整体遮罩与嵌套类型自身 Debug 的边界分别验收。
- 两种 config_value 字段选择器均克隆准确的真实 String，并保留工具声明来源记录；整对象
  config_value 按值交付时仍保留类型本身的受控 Debug 行为；整配置 Debug
  被遮罩、单独字段和普通消费者 Debug 保持普通行为，显式 Serialize 不被改写。
- 原始快照、内置 source/builder、解析/绑定/校验错误及完整 cause 链不透出原始值。
  显式明文读取和业务主动输出作为边界用例，不冒充框架默认输出的安全保证。

## 10. 热更新与未来扩展

第一版提议中同一个客户端实例表示固定原始数据版本；这是一致性契约，不要求公共
snapshot() 方法或某种内部缓存结构。没有 watcher、reload、Monitor、运行期扩图或
自动重建消费者。第 2/8 节的 Singleton 客户端前提保障 root 内共享同一原始版本，
不保证多文件在同一物理瞬间读取，也不保证多次绑定的默认表达式只执行一次。

未来若加入更新，需要明确读取视图/版本、多模型一致性、候选值绑定校验、失败保留
旧版本与发布通知。配置对象已经不是服务，不能沿用“重建配置 Singleton”的描述；
已有对象和字段副本、数据库连接池及监听端口均不会自动变化。schema 集中启动校验、
共享 typed cache 和每次请求访问远端的异步读取，也必须各自设计，不与本轮分层混同。

占位符、import、官方远程源、YAML、泛型配置、递归 schema、时间相对性/十进制精度
规则及验证分组仍为后续范围。集合校验的结构化安全位置已纳入本版设计。
没有定义前，不暴露空实现或容易被误认为已支持的 API。

## 11. 实现前选型与评审决策

公共两方法边界已经由维护者确定；以下分派、接入与工程细节仍需在实现前定型。
不重新把 get_value/统一节点列为公共协议选项，也不沿用旧版 dyn 门面评审结论。

| 项目 | 当前约束 / 后续需确定 |
| --- | --- |
| 类型化分派与 DI 接入 | 两方法泛型 trait 不兼容 dyn；静态类型传递、工具链有限类型适配或另行设计的门面接入见第 2.5 节。选型后须核对业务注入形式、生成输入及跨 crate；不以隐藏 get_value 代替两方法边界 |
| 公共协议 package | 独立阶段在 nestrs-config 中定义固定 ConfigService/ConfigError，协议不引用客户端节点；暂不拆 package。未来校验支持与工具链接入仍待评审，不新增公开宏包 |
| 泛型读取 | 公共 get/get_required 使用 DeserializeOwned，仅绑定；config_value 自动连接校验。若统一 typed 读取的校验，须评审 Deserialize 行为或 ConfigDecode 及重叠 impl 限制，不添加业务 enable 开关 |
| 绑定与所有权 | 第 7.5 节按请求绑定、对象移动/字段 Clone 是可评审基线；缓存/去重、default/rule 次数及失败范围需最终确认，不能恢复配置对象服务身份 |
| 启动校验 | 真正服务的初始化触发其配置消费；未使用 schema 不运行。集中准备/校验全部模型仍待独立设计，配置模型不接受 lazy(false) |
| 对象地址 | prefix 与直接字段共用目录，空根对象与对象/字段碰撞须覆盖；整对象类型选择器尚未定型，不能按消费目标类型隐式消歧 |
| 来源与诊断 | 后端读取/转换错误可提供安全的可选来源；成功 get 只返回 T，生成校验不能通用追溯文件位置或覆盖历史。不新增来源查询协议；官方诊断扩展与共同错误分别设计 |
| 服务角色诊断 | configable 显式配置类型用于 inject/服务查询、injectable 或 factory 注册拟诊断冲突；普通类型不因可达或有校验规则而全局禁止其既有 DI 能力，普通注入不获得配置绑定语义 |
| 官方 source 构造器 | 独立阶段已实现 ConfigSource、ConfigLayer、Origin 及内置来源，具体入口见第 0.2 节；替换整个客户端不要求使用它们 |
| 路径 API | 独立库已提供 &str 快捷路径及 ConfigPath 显式段；get_path/get_required_path、section/explain 及对应 path 版本属于官方扩展。ConfigLocation 表达字段/索引/Map 条目，条目序号不是读取路径且不暴露动态键 |
| 官方解析器与 features | 已复用 workspace 的 Serde/JSON/TOML 版本；json/toml 默认开启，dotenv 为可选受限纯解析器，不使用有隐式插值的 dotenvy；版本由 Cargo.lock 固定，不限制第三方内部选型 |
| 配置值域与资源限制 | 官方值域见第 3.4 节；已实现每文件16MiB、树深度64及每层/最终树100,000节点限制。未来生成校验的步数及Map预算另定，第三方协议不强制共同节点模型 |
| 根配置字段 helper | configable 中 default/default(expr)/label 的规则见第 6.3 节；表达式类型与宏卫生、原生枚举 default 隔离、Serde helper 来源及仅反序列化映射需生成后端落实 |
| 自动校验生成 | 从 configable 根沿真实字段类型和受支持容器连接普通对象规则；父无本地规则仍深入，仅可分析的整个可达子结构无规则才省略。移除 config_object，不增加逐层启用标记；metadata、私有字段合法能力和类型图处理待实现选型 |
| 普通类型属性接入 | 无外层配置宏的规则须在标准流程中合法收集/消费，明确阶段、真实属性身份、cfg/宏生成项和 derive 顺序；不能靠后期类型图消除未知属性，也不注册工具属性、扫描源文本或回写 rlib |
| 嵌套绑定及 Debug | 普通类型保留自身 Deserialize、映射、默认值、未知字段和 Debug；无外层宏的 default/label/sensitive 生成入口另待设计，不因自动校验隐式接管。没有能力不能静默忽略声明；根 strict 不递归传播 |
| 逻辑位置与输入映射 | 根受控 label 可提供准确输入键；普通类型的 rename/flatten/手写转换不从字段名反推路径，缺少可证明映射时明确报告 Rust 模型字段位置；不伪造文件来源 |
| 格式规则与依赖 | 冻结 regex/URL/email/UUID/CIDR/SemVer/RFC3339 的具体库版本、方言和 features；工具与运行期模式检查一致，补齐输入边界语料，不等同完整 Java/JS 验证器兼容性 |
| 集合/组合执行 | 固定数组与嵌套容器的静态识别、工作栈预算、Map 排序/安全逻辑位置、when/compare 字段解析与 rule 函数签名检查；不提升私有字段可见性、反查后端来源或注册配置对象服务 |
| sensitive 与 Debug | 默认/自定义固定替代文本、保留原类型、受控整对象 Debug 的契约见第 9 节；派生宏真实身份、实现来源及错误顺序的强制检查需落实到工具阶段，不以单纯 token 匹配替代 |
| 安全错误构造 | 独立库已提供 new(kind)/at_path/at_location/with_origin，source() 为 None，无任意动态消息；未来生成校验及异步工厂错误适配仍待实现 |
| 对象/字段地址元数据 | 第 7 节拟定规范地址、消费 crate 解析域、唯一目标和两种字段语法的统一语义；目录编码、发现/生成的具体阶段及用户位置映射需工具实现审查，不向运行期公开注册 API |
| 编译诊断 | 明确区分 Rust 类型错误、宏语法/字段地址解析错误和 DI 图错误；编号沿用诊断治理，不在设计阶段编造已存在的新错误码 |

维护者已明确配置模型与服务分离、配置对象/字段通过 config_value 消费、工具链与
客户端分离，以及公共 trait 只保留两个类型化读取方法。本轮进一步移除 config_object，
确定沿真实字段类型自动级联的方向。根 configable 的 default/default(expr)/label
及 sensitive 语义保持；普通类型的校验属性接入和绑定/Debug 扩展分别待定。
分派/DI 接入、具体类型约束与签名、按次绑定/字段 Clone 基线、数据角色诊断及
集中启动校验仍需评审；不能把助手提出的
细节自动记为已确认，也不能把语法探针当作配置运行期实现验收。

## 12. 未来验收矩阵

下表同时包含独立运行时的验收契约与未来工具链能力的实现门槛；已交付范围见第 0 节，
宏、DI、自动校验及其 driver/IDE 项仍是未来要求。库与工具的正式测试放在各自负责的
位置，不为测试暴露 core 内部 API；core 原有测试布局见[说明](../nestrs-core/tests/README.md)。

| 层次 | 必须覆盖的正例与负例 |
| --- | --- |
| 公共 trait | 仅 get/get_required，DeserializeOwned，具体/C: ConfigService 静态调用、默认必选读取；新 trait 的 dyn 和 impl dyn 均不能成立；依赖重命名及同一真实 trait 身份 |
| 分派与接入 | 所选方案的真实输入、后端类型传递/隐藏及闭合 T；适配器调用原生检查、跨 crate 库和未知下游调用边界；不能拿现有 Injection<dyn Trait> 证明支持该泛型 trait |
| 后端替换 | 至少官方与内部表示不同的独立适配器通过同一行为测试；不要求共同节点/binder/来源树；缺失/null/精确整数/目标 Deserialize、安全错误和未知来源。schema/消费 library 的替换方式随分派方案验收 |
| 读取接口 | 两方法的路径/类型错误时机、缺失与显式 null、完整对象/集合及生成 Deserialize 默认值；普通 typed 读取不执行独立规则。成功只返回 T，不要求来源侧通道 |
| 官方扩展 | section 的立即读取/固定子树/相对错误路径、标量/null section、ConfigPath 的字面键/索引、explain 的未知来源与不可用覆盖历史；不作为第三方适配的通过条件 |
| 官方来源 | 内存/TOML/JSON/env/dotenv，自定义 ConfigSource 的合法构造入口，文件不存在与损坏的区别，UTF-8，规范化撞名，只读环境，显式 base_dir |
| 官方合并 | 顺序覆盖、对象合并、数组整体替换、空值、Null/Missing、父子冲突、非有限值和超范围数字 |
| 路径与绑定 | 公共空根/非法路径/Missing 与形态错误/缺失分组；特殊键和数组通过完整对象读取；目标类型的数值范围、未知字段、默认值、坏元素。官方文本与结构化字符串区别及 flatten/untagged 文本限制另作来源兼容测试 |
| 校验声明 | 根及普通类型规则合法收集；未被配置消费的声明也检查语法/类型，本地/可达有规则自动连接、全部无规则省略；拒绝旧开关和 config_object；参数/类型/重复、required 与 Option/default、cfg 及 derive 顺序；未知规则不能静默跳过 |
| 数值/字符串/格式 | 有符号极值、浮点 NaN/无穷和准确类型边界、multiple_of 零/极值；Unicode 标量/UTF-8 字节/空白；正则全串/方言/非法模式；email/URL/IP/hostname/CIDR/UUID/SemVer/RFC3339 边界与不修改原值 |
| 集合与组合 | unique 首次重复索引、contains_all/none、each/key/value 与嵌套容器、空集合、Option 元素 required；Map 稳定顺序及键不泄漏；compare 类型/字段/cfg/label、when 只门控内部约束、字段组合 Some/default 存在性 |
| 自动级联与自定义 | 多层普通对象无额外标记、连续多层无本地规则而叶子有规则、Option/集合/Map 组合；同类型代码复用但每个实例都校验；外部值叶子、已知规则缺能力诊断、递归类型环与重复类型区分；rule 为业务检查而非启用级联 |
| 普通类型绑定/角色 | 正常 derive 与手写 Deserialize 不被替换，缺少绑定能力仍诊断；自身默认值/未知字段/字段映射不被根覆盖；不自动改写 Debug，不静默接受未接管的 default/label/sensitive；不因可达而注册服务或禁止原有 DI 能力 |
| 声明入口与跨 crate | 普通类型裸名/nestrs 路径规则的合法消费、其他宏同名属性隔离、cfg/外部模块/macro_rules/derive 顺序；上游无下游根时也生成合法能力和 metadata，私有字段/依赖别名/多次使用；不靠源码扫描或修改已有 rlib |
| 校验执行 | 完整绑定后按字段显式规则、嵌套对象、类型规则顺序首错；when 不关闭独立级联；失败不交付当前消费者，延迟依赖沿用 core 语义；未用类型仅编译检查；公共 typed 读取与官方 bind 不执行独立生成校验；迭代遍历与资源预算 |
| 校验错误 | label 字面键/索引/Map 条目角色；普通类型 rename/flatten/手写转换后的模型位置不伪装为输入路径，已知映射与未知映射分开；声明位置保留，成功 typed 读取不补查来源；后端错误来源可选且安全；动态键不进入 Display/Debug/source/raw-key；手写 factory 显式逻辑上下文 |
| 敏感配置与脱敏 | sensitive 默认/自定义/空替代文本与转义；普通/pretty 整对象 Debug 按整字段遮罩，不读取秘密或调用其 Debug；未标记字段正常；字段选择器保留实际 String 与工具声明来源记录，整对象交付保留受控 Debug；单字段/消费者 Debug 及 Serialize 的普通行为；原始树与完整错误链安全输出 |
| 生成后端 | configable 参数、内部 Serde 依赖路径/重命名、根类型及直接字段的用户 serde helper 拒绝、derive 冲突、配置模型 lazy 拒绝；根 sensitive、标准 Debug 真实身份/提前展开/冲突/其他 derive 保留；config_value 选择器、裸名/nestrs 路径、cfg 和宏卫生 |
| 字段默认与标签 | default 裸标记/单表达式/函数调用、准确目标类型与无隐式 Into、Option 覆盖、缺失才求值、显式 null/错误不兜底、多次绑定与失败路径；原生枚举 default 和结构体 Default 不变、派生顺序；label 字面键/重复/非法参数、与敏感标记共存、Serialize 名称不变 |
| 对象/字段地址目录 | prefix/空 prefix 的整对象目标、Rust 字段名/label、无隐式别名、特殊字面键、对象/字段碰撞、同身份幂等；自动深层校验不增加嵌套 prefix/字段地址；default 不回写树 |
| 真实 driver | 同一字段两种形式同义；根对象和字段解析；不按目标类型/可见性/primary 筛选、不回退原始树；配置模型不产生服务节点、inject/服务查询/注册角色冲突、门面缺失/歧义/环/Scope 检查、准确类型/字段 Clone/对象非 Clone、可见性、raw identifier 与别名 |
| 生命周期 | 客户端及消费者的 Lazy/Eager/lazy(false)；只准备客户端不校验所有模型；重复绑定/默认值/规则次数、失败归属、多 root 隔离及 scope 共享原始版本，不虚构配置对象 Singleton |
| 构造顺序 | 全部 typed 输入消费后才执行配置绑定/校验/字段 Clone；与 Default/value 的字段顺序、失败前副作用边界、panic、cfg 排除；门面真实依赖与 lease；无隐藏 resolve 或配置 provider |
| 工具无副作用 | 缺少运行期文件/key 不影响声明地址存在性；check/build/graph 不读取配置、不执行 factory/默认函数/校验，运行构造才判断数据有效性；未被查询的配置声明仍接受规则语法/类型检查 |
| 跨 crate / IDE / docs | 校验在声明 crate 生成并类型检查、最终入口保留调用且不重写 rlib；依赖 rename/re-export、公开/私有字段、消费库编译时解析目录与真实 adapter、最终入口核验、下游声明不重定向上游请求、仅下游定义来源时拒绝、rustdoc、原版 rust-analyzer 两种选择器；不能仅靠 token snapshot |

库内部与集成测试可放在 `nestrs-config/tests/`；宏单测复用 cargo-nestrs 的生成后端测试，
真实编译器测试放在 `cargo-nestrs/tests/` harness 和 `tests/fixtures/`。不把所有测试
搬进 core，也不把当前 core 的目录规则误写成 cargo-nestrs 已禁止全部内联测试。
负例检查诊断 code/message/数量和用户位置，不能仅检查非零退出或批量覆盖 stderr。

同步 build 的阻塞行为要单列验证。Linux、Windows 和其他 target 的结果分别记录；
独立库本机验收不等于其他平台文件编码、环境行为或未来工具链/IDE 已验收。

## 13. 框架参考与取舍

研究核对了官方实现源码、文档及部分源码中的测试，没有运行这些框架的服务或测试程序。
以下版本是研究样本，不表示 Nestrs 与它们行为兼容，或不同上游版本组合经过运行验证。

| 样本 | 固定版本 / commit | 对 Nestrs 的取舍 |
| --- | --- | --- |
| Spring Boot | 4.1.1 / `6fdf67ea1552691e932604d4bf67a5e08ff0b0ea` | 借鉴分组绑定、列表整体覆盖、来源追踪；不复制全部 profile/import/relaxed binding |
| NestJS Config | 12.0.1 / `2785b5f1a1331be0d8a54053a2dca23b636045d9` | 借鉴分组配置与类型注入；泛型 get 只是 TS 断言，Nestrs 要真实转换且不写 process.env |
| NestJS 配置文档 | `e5b515a3bb78802e5696017e2f45cf7957b1d59c` | 核对 plainToInstance 后 validateSync 的分层；Nestrs 工具链自动连接声明规则，无需单独开启 |
| class-validator | `2e1a5c27dbd65b80e27fe96b49bd6e6641fa3603` | 借鉴字段旁声明规则，采用 Rust 类型化生成；不移植全局 metadata 注册、原值错误结构或同步忽略异步规则 |
| Spring Boot 配置校验文档 | `f6142c9f47591b70949391d1848c1f81503006db` | Boot 接入 Jakarta 校验并用 Valid 级联；Nestrs 不照搬 Validated 启用开关 |
| Jakarta Validation | `9fe12637086b1c802bd451004483d28a6b634ac9` | 借鉴存在性/范围/容器和对象约束；Option、浮点及字符串单位由 Nestrs 明确定义 |
| Hibernate Validator | `235a02acc8aad8cc71e3b97ace568c140351acf2` | 借鉴扩展格式、容器元素与组合约束；不把实现扩展称为 Spring Boot 自带规则 |
| .NET runtime | 10.0.12 / `4271d88e0aebf3d04f188f1334c2220d80555ef6` | 借鉴来源组合与 typed options 分层；不复制索引键导致的数组尾项保留 |
| .NET extensions | 10.10.1 / `59e1741f6fb45cf7ae08e2df088aef71537a7bc4` | 显式数据分类与日志 redaction 是独立扩展，不是 IConfiguration 默认能力 |
| Spring Cloud Commons | 5.0.3 / `9e19c8233fd4d1f23e0e2953fedfdd474acedf33` | 仅用于核对刷新边界；不能把 Cloud refresh 归给 Boot 基础配置 |

关键依据：

- [Spring 加载阶段][spring-load]、[列表绑定][spring-list]与[配置绑定/校验][spring-bind]：
  来源、绑定、验证各有职责；Nestrs 的“先构建快照再绑定”是自身设计。
- [Nest 配置模块][nest-module]与[环境读取][nest-get]：默认进程环境与 env 文件有自身
  优先级，get 的 T 不执行运行期转换；自定义 load 结果也不是自动全面 schema 校验。
- [Nest 自定义配置校验示例][nest-validation]先转换再同步校验；Nestrs 把生成规则的
  调用连接交给工具链。[class-validator 元数据存储][class-validation-metadata]采用
  全局登记，Nestrs 继续采用静态声明和类型化代码。[其错误生成][class-validation-error]
  可携带 target/value；本方案只保留安全路径和规则说明。[其同步入口][class-validation-sync]
  会忽略异步规则，Nestrs 第一版则明确不接受异步校验声明，不静默跳过已声明规则。
- [class-validator 规则目录][class-validation-catalog]包含格式、数值、数组和条件等
  能力；[Spring Boot 配置校验][spring-config-validation]负责接入，[Jakarta 标准约束][jakarta-constraints]
  与 [Hibernate 标准/扩展规则][hibernate-constraints]负责具体契约。Nestrs 采用自身的
  Option 和准确类型规则，不承诺名字相似就具有相同的 null、浮点或正则行为。
- [Hibernate 容器约束][hibernate-containers]与 [对象级联][hibernate-cascade]区分
  元素规则和对象图；Nestrs 拟按真实字段类型及合法规则能力自动连接级联。[类级约束][hibernate-class-constraints]
  与 [组合约束][hibernate-composition]为字段组合和可复用 rule 提供参考。时间与
  十进制精度依赖独立类型契约，本版不提前宣称支持。
- [.NET Root 按来源反查][dotnet-root]与[数组官方测试][dotnet-array]：低层 `[a,b,c]`
  叠加高层 `[x]` 的测试结果为 `[x,b,c]`；Nestrs 明确选择整数组替换。
- [.NET OptionsMonitor][dotnet-monitor] 先清缓存再重新创建/校验，不提供失败自动回滚
  旧缓存的保证；[Cloud 刷新文档][cloud-refresh]也区分对象原位重绑与其他刷新机制。
- [Spring Actuator 展示策略][spring-sanitize]默认隐藏端点配置值，但
  [绑定失败诊断][spring-bind-error]仍可能格式化原值；端点脱敏不是普通日志全局脱敏。
- [.NET GetDebugView][dotnet-debug]默认可输出原值，调用方可提供处理函数；
  [Options 校验异常][dotnet-validation]直接组合 validator 消息。
- [.NET EnableRedaction][dotnet-redaction]属于日志扩展，须结合策略与已分类字段；
  配置来源是 User Secrets/Key Vault 不会让普通 String 自动具有秘密属性。
- 第 2 节的 impl dyn/lifetime/可见性说明另以 Rust Reference 和独立语言探针核验；
  不以其他框架的接口作为 Rust 对象安全或借用行为的证据。

## 14. 评审与本轮验证记录

### 14.1 历史设计评审

以下记录保留当时检查范围。旧版配置 provider、Singleton、配置模型 lazy(false)、
整对象 inject、任意 factory 对象字段选择以及配置服务失败缓存等结论，已由当前
第 2/5/6/7/8 节替代。config_object 的历史引入也已由第 6.5 节撤回。历史“通过”
不代表当前所有新契约已经选定或实现。

2026-10-03 完成三个独立 agent 的只读初审、作者修订和聚焦复审。评审只针对设计
一致性、架构边界、来源依据与未来可验收性，不把评审代替实现测试。三个评审均确认
其检查范围内没有未解决的阻塞项；这不等于维护者已批准全部新契约。

| 评审范围 | 初审发现与修订 | 复审结果 |
| --- | --- | --- |
| 静态 DI、生成代码、输入与初始化 | 修正 lazy(false) 不是 Eager 主动初始化的必要条件；补齐统一快照的 Singleton 前提；明确字段取值消费已交付 typed 输入及依赖边方向 | 原 2 项 P2 和 1 项 P3 已闭合 |
| 上游依据、来源、绑定和骨架范围 | 用独立 Serde 实验确认 flatten/untagged 的文本缓存限制；补齐文本/结构化字符串语义标签和统一路径规则 | 原 1 项 P2 和 2 项 P3 已闭合 |
| 秘密类型、诊断和错误接口 | 增加校验路径的静态分段入口，消除字面 key 与嵌套路径歧义；补齐 section/default 来源边界；将安全 ConfigError 构造列入实现前选型 | 原 1 项 P2 和 1 项 P3 已闭合 |

同日按维护者要求将字段 helper 命名改为 config_value；命名修订及类型选择器经过
两位独立 agent 复审。本次进一步采用 `#[config_value("配置字段地址")]` 为主写法，
保留 `#[config_value(TypePath::field)]` 为补充：两者统一到真实类型依赖和字段 Clone，
撤回“字符串形式只能读取原始配置树、不能复用默认值和校验”的前提。第 7 节补齐地址
命名、Serde 规则、解析域、歧义、跨 crate 和可见性边界，验收矩阵同步增加相应案例。
这次字段地址修订已由两个独立 agent 完成实际文档复审：分别核对静态 DI/跨 crate
边界和 Serde/地址命名规则，均未发现未解决阻塞项。本地链接、引用定义、代码围栏、
旧语法残留检查及 `git diff --check` 通过；AGENTS 与文档导航同步更新。本次只有文档
修改，没有实现解析器或运行期能力，也没有将验收矩阵当作已通过的功能测试。

脱敏章节的入口、原始配置/错误链安全表示及 logger 分工此前已经过独立复审。本轮
按维护者偏好将公开声明改为 `#[sensitive]` / `#[sensitive("******")]`，保留 String
等原字段类型，由 configable 接管明确请求的标准 Debug。保护范围改为受控整对象
格式化；旧方案中“克隆后值自身仍会打码、需 expose_secret 才能读取”的前提不再适用。
第 6/7/9 节、实现前选型及验收矩阵同步修订。三个独立 agent 对宏/Debug 生成边界、
安全语义和 Serde/派生协作完成实际文档复审，均未发现未解决阻塞项。链接、章节
锚点、引用定义、代码围栏、旧契约残留与空白检查，以及
`cargo fmt -p nestrs-config -- --check` 和 `git diff --check` 通过。AGENTS、文档
导航与 lib 注释同步更新；本轮仍未实现宏或执行脱敏功能测试。

同日按维护者意见进一步收窄配置声明，采用 `#[default]`、`#[default(表达式)]` 和
`#[label("配置键")]`，不采用字段上的 configable(default/name)，也不再承诺透传
用户 Serde helper。第 6.3/6.4 节记录缺失值、准确类型、原生 Default 隔离和高级
手写绑定边界；第 7 节字符串地址改为 prefix 与 label/Rust 字段名组合。此前评审
记录中的 Serde 字段命名方案已经被本次声明规则替代，其他独立绑定实验仍保留其
适用范围。两个独立 agent 对实际修订文档完成只读复审，均未发现未解决阻塞项；
已按非阻塞建议澄清示例章节引用、类型适配入口和自定义错误文案的适用位置。
本地链接、引用、章节锚点、代码围栏、代码块语言标记、旧属性残留及空白检查通过，
`cargo fmt -p nestrs-config -- --check` 和 `git diff --check` 通过。本轮仅修改
设计文档、规范/导航和 crate 说明；没有实现或执行 default/label 的功能测试。

同日按维护者纠正，将校验收敛为“工具链统一收集声明，有规则就生成并执行，没有
规则就省略额外步骤”。撤下公共 Validate、bind_validated、validate 参数以及
validate_with 启动入口，新增第 5.3 节的收集/生成/执行分层与首批规则；常规配置
结构体只声明规则，不实现或启用校验协议。普通 bind 仍只绑定，自动校验连接在
生成 provider 中；第 6/7/9/11/12 节、AGENTS、导航和 crate 说明同步更新。
一名独立 agent 对实际文档完成只读复审：修正“失败阻止所有消费者构造”为立即
输入的消费者，并补齐延迟句柄边界；明确手写 factory 必须显式提供校验错误的
section 上下文。聚焦复核确认两处闭合，没有剩余阻塞项。链接/引用/锚点、围栏、
代码块语言标记、旧公开校验 API 残留及空白检查通过，
`cargo fmt -p nestrs-config -- --check` 与 `git diff --check` 通过。未实现校验宏，
未执行校验功能测试，也没有把工具收集规则描述成编译期执行部署配置校验。

随后按维护者要求对照 class-validator、Spring Boot/Jakarta Validation 与 Hibernate
Validator 扩展规则目录，新增存在性、数值/格式、集合元素、条件/跨字段和自定义 rule。
当时 config_object 与自动级联同时进入设计，补齐父无本地规则时的可达子规则判断、Map
稳定遍历与不泄漏动态键的错误位置；嵌套校验不扩展 config_value 字符串地址或 DI 图。
固定源码样本和差异说明补入第 13 节。独立 agent 对实际新增章节及同步内容完成
只读复审和收尾核对，没有设计阻塞项；按建议明确不同规则作用域、each 的内层规则
限制，以及集合校验错误对来源元数据的更严格处理。旧版将递归/集合约束全部留待
后续的表述已由本轮契约取代；递归 schema 定义、时间相对性/十进制、分组和异步
仍为后续范围。链接/引用/锚点、围栏、代码块语言标记、旧语法和空白检查通过，
`cargo fmt -p nestrs-config -- --check` 与 `git diff --check` 通过。未新增运行期
依赖、实现宏或执行配置功能测试；所有规则仍是待实现设计。

### 14.2 历史门面与数据模型修订（节点契约已被取代）

2026-10-03 较早一轮记录：公共协议与客户端分离；配置模型不注册 DI；config_value
覆盖对象和字段；普通泛型读取与自动 schema 校验分开；取消强制 snapshot() 接口。
当时另提公共 get_value/ConfigValue 与门面 A/B，讲解 impl dyn 的可见性取舍，并更新
启动时机、所有权、地址冲突及错误边界。**节点契约、A/B 选型和通用来源查询已由
本轮两方法决定取代**；以下保留实验和评审历史，不代表当前公共接口仍有这些要求。

独立语言探针使用本机 rustc 1.98.0（88d9e12ae178fab0fb5cc050a94da85685d449ea），
以 FromStr 缩小转换依赖，验证 Rust 机制而非配置实现。7 项断言通过：

| 实际探针 | 结果与范围 |
| --- | --- |
| 非 static 实现 + 显式 `+ '_`，及同形 Deref 包装调用 | 编译/运行通过；不改变 Nestrs 服务注册的 static 要求 |
| 跨 crate 调用 doc(hidden) get_value | 编译/运行通过，证明方法仍公开 |
| 普通 rustdoc 展示 doc(hidden) 方法 | 正常输出省略方法，与上述调用权限无关 |
| 将泛型方法放进主 trait 并使用 dyn | 预期 E0038 |
| 固有 impl 省略对象 lifetime 后接受非 static 后端 | 预期 E0521 |
| 显式附加 Send + Sync 的 trait 对象调用该组固有方法 | 预期 E0599 |
| 具体后端直接调用该组固有泛型方法 | 预期 E0599 |

源码、命令、stdout/stderr、退出码和汇总位于忽略目录
`target/nestrs-config-design-review/facade-probe/`，run.py 可复现。该目录不随仓库分发；
本文保留结论。未执行尚不存在的 configable/config_value/后端适配集成测试，也未以
独立语言机制证明 Windows、跨 target 或 IDE 新能力。

当时两位独立 agent 完成只读复审，撤回残留的 config_object factory 服务路径，
区分敏感字段复制与整对象交付，补明级联发生于 config_value。旧稿还要求公共显式
ConfigPath 入口并保留 A/B 待选；这两项已撤回或改为官方扩展。旧评审结论不能
用于证明当前泛型 trait 可形成 dyn，也不解除新的分派选型问题。

本地文件链接、引用定义、章节锚点、围栏配对及空白检查通过；21 处 Rust 代码块均使用
普通 rust 标记，没有 rust,ignore。cargo fmt -p nestrs-config -- --check 与
git diff --check 通过。检查结果及本轮文档差异另存于忽略目录
target/nestrs-config-design-review/。本轮仅修改文档与说明注释，没有实现配置能力、
增加运行期依赖、提交或推送；工作区已有的其他修改保留。

### 14.3 此前两方法契约修订

2026-10-03 按维护者决定将公共 ConfigService 收敛为 get<T>/get_required<T>，
后端自行读取并按 DeserializeOwned 转换。撤回 get_value、统一节点/快照/绑定器和
A/B 作为公共协议选项；section/explain/ConfigPath 只保留为官方客户端扩展。
同步改正 DI 自动绑定、完整模型读取后校验、成功 T 不携带配置来源、评审项和验收矩阵。

第 2.4 节保留 impl dyn 的重点讲解，用另一个对象安全的 ObjectReader 示例解释
原生语法；它不能用于新 ConfigService。维护者确定了公共方法边界，具体分派与
DI 注入仍待评审，不在本轮选择特殊编译器改写或实施功能。

新增独立探针实际使用缓存 Serde 1.0.229 / serde_json 1.0.151，Cargo 全程 offline，
编译器为本机 rustc 1.98.0（88d9e12ae178fab0fb5cc050a94da85685d449ea），
host 为 x86_64-unknown-linux-gnu：

| 实际验证 | 结果与范围 |
| --- | --- |
| 具体类型、C: ConfigService、&impl ConfigService 及默认 get_required | 静态调用通过 |
| 标量/对象/集合、真实 Serde 字段默认值、缺失/null/类型错误/非法路径 | 与上一行合计 14 项运行断言通过 |
| 新 trait 用作 &dyn ConfigService | 预期 E0038 |
| 新的本地 trait 用于 impl dyn ConfigService + '_ | 预期 E0038；排除外部 trait 固有 impl 限制造成的混淆 |

源码、命令、stdout/stderr、退出码与 summary.json 位于忽略目录
`target/nestrs-config-design-review/typed-facade-probe/`，run.py 可复现。实验只验证
Rust/Serde 机制，不是 Nestrs 配置宏、DI 接入、后端替换或新接口的功能验收；探针
在内部使用 serde_json 也不构成要求所有后端使用该值模型的证据。

两位独立 agent 完成只读复审：覆盖第 1–2 节的 Rust/协议边界、第 3–12 节的客户端
扩展与消费链路，以及本节实验记录；伴随 AGENTS、入口文档及 crate 注释已同步。
检查范围内无未解决的文档一致性问题；分派方案本身仍是实现前待决事项。按复审建议，
进一步明确官方 bind 扩展及工具声明来源记录，避免混同公共方法或运行期配置来源。

本地文件链接、引用定义、章节锚点、围栏配对与空白检查通过；20 处 Rust 代码块均为
普通 rust 标记。cargo fmt -p nestrs-config -- --check、git diff --check 通过。
本轮文档差异与检查记录保存在忽略目录 target/nestrs-config-design-review/。
仅更新文档与说明注释，没有实现配置能力、增加正式依赖、提交或推送。

### 14.4 本轮移除 config_object 与自动深层校验修订

按维护者决定，撤回 config_object，不以无 prefix 的 configable、nested 或
ValidateNested 代替。从根配置沿真实字段类型自动连接普通嵌套对象及受支持容器，
本层无规则仍继续深入；只有整个可分析子结构无规则才省略校验。

第 5/6 节更新规则收集、三层对象示例、普通类型绑定与校验的分工；第 9 节明确普通
类型 Debug 不随级联自动改写，根字段仍可整体遮罩。第 11/12 节补齐属性合法消费、
跨 crate 能力、逻辑模型位置与输入映射、递归边界及普通类型 DI 角色的评审/验收要求。
AGENTS、导航和 crate 注释同步撤下旧声明。

本轮仅记录目标和边界。普通类型无外层配置标记的规则属性接入尚待设计，深层校验
示例尚不可作为当前 Nestrs 能力；普通类型 default/label/sensitive 的生成入口也
未因移除旧宏自动确定。两方法读取契约、DI 分派待决项和按次绑定/缓存待决项保持。

两位独立 agent 完成只读复审，覆盖主文档与伴随规范的一致性，以及属性合法接入、
跨 crate 能力、绑定/Debug、模型逻辑位置、普通类型 DI 角色和递归限制。按复审建议，
将服务 lazy 限制明确限定于显式 configable 模型，并区分外部不透明叶子与“已知规则
缺少合法执行能力”的诊断，避免静默漏检。检查范围内没有未解决的文档一致性问题。

本地文件链接、引用定义、章节锚点、代码围栏及空白检查通过；20 处 Rust 代码块
均使用普通 rust 标记。cargo fmt -p nestrs-config -- --check、git diff --check
通过。检查及本轮差异保存在忽略目录 target/nestrs-config-design-review/。
本轮仅更新设计文档、仓库规范及说明注释，没有新增功能、正式依赖或配置宏测试；
此前 Rust/Serde 探针不能证明新的声明采集或自动级联已经可用。未提交或推送。

### 14.5 骨架与此前独立绑定实验

此前骨架建立阶段的实际检查使用本机 `x86_64-unknown-linux-gnu`、rustc 1.98.0
（`88d9e12ae178fab0fb5cc050a94da85685d449ea`）：

| 实际执行 | 结果与证明范围 |
| --- | --- |
| `cargo check -p nestrs-config --offline`，随后 `cargo check -p nestrs-config --locked --offline` | 通过；仅证明空 library 骨架和锁文件可构建 |
| `cargo metadata --no-deps --format-version 1 --offline` | 确认为 lib target、workspace 成员、无依赖、禁止发布，不在 default-members 中 |
| `cargo doc -p nestrs-config --no-deps --locked --offline` | 通过；生成当前 crate 说明，不表示拟议 README 示例已编译 |
| `cargo fmt --all -- --check` | 通过；没有修改既有 Rust 实现 |
| `git diff --check`、设计文档本地链接/引用定义/代码围栏检查 | 通过；Cargo.lock 仅新增 nestrs-config 本地包条目 |

另以 Serde 1.0.229 运行独立反序列化小程序：同一文本 `"42"` 直接绑定 u16 成功，
flatten 的 u16 和 untagged 纯数值分支失败，untagged 的数值/字符串分支选择字符串。
该实验验证第 5.2 节的支持边界，不是 nestrs-config 功能测试。源码、锁文件、命令、
退出码 0 和输出保存在忽略目录 `target/nestrs-config-design-review/serde-buffering/`；
临时证据不随仓库分发，核心结论保留在本文。

未运行尚不存在的配置功能测试，也未宣称 Windows、跨 target 或新宏的 IDE 集成已验收。
第 12 节仍是未来验收要求；第 11 节仍列出实现前应完成的接口细化与依赖选型。

维护者已确认的边界与仍待定的细节按第 11 节区分。独立文档复审不代替分派方案定型；
此前该阶段只包含骨架与文档；当前独立运行时范围见第 0 节。工作仍保留未提交。

[spring-load]: https://github.com/spring-projects/spring-boot/blob/6fdf67ea1552691e932604d4bf67a5e08ff0b0ea/core/spring-boot/src/main/java/org/springframework/boot/context/config/ConfigDataEnvironment.java#L236-L247
[spring-list]: https://github.com/spring-projects/spring-boot/blob/6fdf67ea1552691e932604d4bf67a5e08ff0b0ea/core/spring-boot/src/main/java/org/springframework/boot/context/properties/bind/IndexedElementsBinder.java#L72-L80
[spring-bind]: https://github.com/spring-projects/spring-boot/blob/6fdf67ea1552691e932604d4bf67a5e08ff0b0ea/core/spring-boot/src/main/java/org/springframework/boot/context/properties/ConfigurationPropertiesBinder.java#L92-L130
[nest-module]: https://github.com/nestjs/config/blob/2785b5f1a1331be0d8a54053a2dca23b636045d9/lib/config.module.ts#L57-L163
[nest-get]: https://github.com/nestjs/config/blob/2785b5f1a1331be0d8a54053a2dca23b636045d9/lib/config.service.ts#L302-L317
[dotnet-root]: https://github.com/dotnet/runtime/blob/4271d88e0aebf3d04f188f1334c2220d80555ef6/src/libraries/Microsoft.Extensions.Configuration/src/ConfigurationRoot.cs#L114-L127
[dotnet-array]: https://github.com/dotnet/runtime/blob/4271d88e0aebf3d04f188f1334c2220d80555ef6/src/libraries/Microsoft.Extensions.Configuration.Json/tests/ArrayTest.cs#L82-L110
[dotnet-monitor]: https://github.com/dotnet/runtime/blob/4271d88e0aebf3d04f188f1334c2220d80555ef6/src/libraries/Microsoft.Extensions.Options/src/OptionsMonitor.cs#L64-L70
[cloud-refresh]: https://github.com/spring-cloud/spring-cloud-commons/blob/9e19c8233fd4d1f23e0e2953fedfdd474acedf33/docs/modules/ROOT/pages/spring-cloud-commons/application-context-services.adoc#L185-L214
[spring-sanitize]: https://github.com/spring-projects/spring-boot/blob/6fdf67ea1552691e932604d4bf67a5e08ff0b0ea/documentation/spring-boot-docs/src/docs/antora/modules/reference/pages/actuator/endpoints.adoc#L307-L326
[spring-bind-error]: https://github.com/spring-projects/spring-boot/blob/6fdf67ea1552691e932604d4bf67a5e08ff0b0ea/core/spring-boot/src/main/java/org/springframework/boot/diagnostics/analyzer/BindFailureAnalyzer.java#L57-L90
[dotnet-debug]: https://github.com/dotnet/runtime/blob/4271d88e0aebf3d04f188f1334c2220d80555ef6/src/libraries/Microsoft.Extensions.Configuration.Abstractions/src/ConfigurationRootExtensions.cs#L20-L60
[dotnet-validation]: https://github.com/dotnet/runtime/blob/4271d88e0aebf3d04f188f1334c2220d80555ef6/src/libraries/Microsoft.Extensions.Options/src/OptionsValidationException.cs#L20-L48
[dotnet-redaction]: https://github.com/dotnet/extensions/blob/59e1741f6fb45cf7ae08e2df088aef71537a7bc4/src/Libraries/Microsoft.Extensions.Telemetry/Logging/LoggingRedactionExtensions.cs#L22-L40
[nest-validation]: https://github.com/nestjs/docs.nestjs.com/blob/e5b515a3bb78802e5696017e2f45cf7957b1d59c/content/application/configuration.md#L465-L523
[class-validation-metadata]: https://github.com/typestack/class-validator/blob/2e1a5c27dbd65b80e27fe96b49bd6e6641fa3603/src/metadata/MetadataStorage.ts#L159-L171
[class-validation-error]: https://github.com/typestack/class-validator/blob/2e1a5c27dbd65b80e27fe96b49bd6e6641fa3603/src/validation/ValidationExecutor.ts#L219-L242
[class-validation-sync]: https://github.com/typestack/class-validator/blob/2e1a5c27dbd65b80e27fe96b49bd6e6641fa3603/src/validation/Validator.ts#L59-L85
[class-validation-catalog]: https://github.com/typestack/class-validator/blob/2e1a5c27dbd65b80e27fe96b49bd6e6641fa3603/README.md#validation-decorators
[spring-config-validation]: https://github.com/spring-projects/spring-boot/blob/f6142c9f47591b70949391d1848c1f81503006db/documentation/spring-boot-docs/src/docs/antora/modules/reference/pages/features/external-config.adoc#L1398-L1417
[jakarta-constraints]: https://github.com/jakartaee/validation/tree/9fe12637086b1c802bd451004483d28a6b634ac9/src/main/java/jakarta/validation/constraints
[hibernate-constraints]: https://github.com/hibernate/hibernate-validator/blob/235a02acc8aad8cc71e3b97ace568c140351acf2/documentation/src/main/asciidoc/reference/_ch02.adoc#L554-L766
[hibernate-containers]: https://github.com/hibernate/hibernate-validator/blob/235a02acc8aad8cc71e3b97ace568c140351acf2/documentation/src/main/asciidoc/reference/_ch02.adoc#L93-L268
[hibernate-cascade]: https://github.com/hibernate/hibernate-validator/blob/235a02acc8aad8cc71e3b97ace568c140351acf2/documentation/src/main/asciidoc/reference/_ch02.adoc#L355-L408
[hibernate-class-constraints]: https://github.com/hibernate/hibernate-validator/blob/235a02acc8aad8cc71e3b97ace568c140351acf2/documentation/src/main/asciidoc/reference/_ch06.adoc#L390-L425
[hibernate-composition]: https://github.com/hibernate/hibernate-validator/blob/235a02acc8aad8cc71e3b97ace568c140351acf2/documentation/src/main/asciidoc/reference/_ch06.adoc#L545-L586


### 14.6 独立配置运行时实现与验收（2026-10-04）

本轮交付第 0 节的普通 Cargo 配置库，保持单一 crate，无 core、工具链或 Tokio 依赖。
ConfigService 的静态泛型读取可直接使用；宏、自动校验、字段 sensitive 和 DI 分派
均未进入本轮实现。原配置设计与历史实验保留为后续集成依据。

在本机 x86_64-unknown-linux-gnu、rustc 1.98.0
（88d9e12ae178fab0fb5cc050a94da85685d449ea）执行以下验收：

| Feature 组合 | 集成测试 | 文档测试 |
| --- | --- | --- |
| 默认 json + toml | 60 通过 | 1 通过 |
| 无默认 features | 46 通过 | 1 通过 |
| 仅 json | 55 通过 | 1 通过 |
| 仅 toml | 50 通过 | 1 通过 |
| 仅 dotenv | 48 通过 | 1 通过 |
| json + dotenv | 57 通过 | 1 通过 |
| toml + dotenv | 52 通过 | 1 通过 |
| 全部 features | 62 通过 | 1 通过 |

全部 features 的集成测试由 binding 17 项、runtime 19 项、sources 26 项组成，包含
独立 Cargo 验收及隔离进程的入口测试。Clippy（all-targets、-D warnings）、格式检查、
rustdoc 生成与 Markdown 本地链接/代码围栏检查通过。examples/basic 通过普通 Cargo
运行并输出 HTTP port: 8080。独立 fixture 实际使用重命名依赖，并核查依赖树不包含
nestrs-core、cargo-nestrs、nestrs-tool-bridge 或 Tokio。

独立复审发现并修复了 section 泛型必选读取的父路径丢失、覆盖后来源残留、空对象
来源丢失、动态 Map 完整读取后自定义错误重新附带敏感来源、上游 TOML 递归限制误分类，
以及静态 FIFO 在打开前未被拒绝的问题；对应回归已经纳入上述测试。
文件打开前后均检查普通文件属性，未承诺抵御并发恶意替换路径的竞态。

以上为本次 Linux 实现与测试证据，未执行 Windows 验收，不等于未来 Nestrs 宏、
DI、IDE 或其他目标平台已受支持。本轮未提交、推送或发布。
