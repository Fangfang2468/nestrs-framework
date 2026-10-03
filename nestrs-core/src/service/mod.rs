//! 查询与实例执行所需的基础类型。
//!
//! 这一层不依赖构造器或声明分析：仅保存服务真实类型、查询限定符和错误来源。
//! 宏语法、候选选择及泛型发现属于工具链，不进入这些运行期身份对象。

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
