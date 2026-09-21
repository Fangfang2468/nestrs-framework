//! 服务声明与标识的基础类型。
//!
//! 这一层不依赖 activation 或 registration：它只定义服务在注册、构造和未来解析过程
//! 中共同使用的类型约束与身份信息。

pub(crate) mod injectable;
pub(crate) mod service_identifier;
pub(crate) mod service_key;
pub(crate) mod service_source;
pub(crate) mod service_type;

pub use injectable::Injectable;
pub use service_identifier::ServiceIdentifier;
pub use service_key::ServiceKey;
pub use service_source::ServiceSource;
pub use service_type::ServiceType;
