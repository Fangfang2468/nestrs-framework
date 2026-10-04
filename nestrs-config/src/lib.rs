//! 可脱离 Nestrs 使用的同步、类型化配置库。
//!
//! [`ConfigurationBuilder::build`] 按顺序加载来源并合并成一份不可变快照；之后的
//! 类型化读取由本库访问内存，不隐式执行来源 I/O。普通 Cargo 项目即可使用，无需 DI 容器、
//! Nestrs 编译器或异步运行时。默认开启 `json` 和 `toml`，`dotenv` 需要显式启用。
//!
//! # 最小使用示例
//!
//! ```
//! use nestrs_config::{ConfigLayer, ConfigPath, Configuration, LayerValue, Origin};
//! use nestrs_config::sources::Memory;
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct Http { port: u16 }
//!
//! let mut layer = ConfigLayer::builder(Origin::named("defaults"));
//! layer.insert(ConfigPath::parse("http.port")?, LayerValue::unsigned(8080))?;
//! let config = Configuration::builder().add_source(Memory::new(layer.build()?)).build()?;
//! let http: Http = config.get_required("http")?;
//! assert_eq!(http.port, 8080);
//! # Ok::<(), nestrs_config::ConfigError>(())
//! ```
//!
//! # 模块与安全边界
//!
//! 公开读取协议、来源扩展协议和官方客户端分别导出；实现第三方 [`ConfigService`]
//! 不要求采用本库的来源或节点模型。每次读取都通过 Serde 重新绑定拥有所有权的目标值，
//! 不缓存或复用绑定结果；Clone 门面只共享原始快照。自定义反序列化器是否执行 I/O
//! 或使用业务缓存，由调用方自己的实现决定。
//!
//! 默认诊断省略配置原值；显式读取仍返回明文，普通 Rust 对象也保留自己的 Debug。
//! 来源名称和显式查询路径属于定位元数据，调用方不能将凭据放入这些字段。本库不会
//! 拦截用户代码主动输出，也不修改用户自定义反序列化逻辑的行为。
//!
//! Nestrs 配置宏、自动校验、字段脱敏宏与 DI 集成仍属于后续工作；当前只执行普通
//! Serde 绑定。输入预算由下方常量给出，所有来源统一遵守最终配置树的结构限制。

#![forbid(unsafe_code)]

mod configuration;
mod de;
mod error;
mod model;
mod path;
pub mod sources;

pub use configuration::{
    ConfigExplanation, ConfigSection, ConfigService, ConfigSource, Configuration,
    ConfigurationBuilder, HistoryStatus, LoadContext,
};
pub use error::{ConfigError, ConfigErrorKind, Origin, OriginKind};
pub use model::{ConfigLayer, ConfigLayerBuilder, ConfigOrigins, LayerValue};
pub use path::{ConfigLocation, ConfigPath, LocationSegment, MapRole, PathSegment};

/// 单个文件最多读取 16 MiB；在 UTF-8 解码和格式解析之前检查。
pub const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
/// 配置树允许的最大节点深度；根为 0，超过 64 层会返回 Limit 错误。
pub const MAX_DEPTH: usize = 64;
/// 单个输入层和最终合并快照各自允许的最大节点数，根也计为一个节点。
pub const MAX_NODES: usize = 100_000;
