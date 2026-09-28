//! 电商业务服务只声明依赖，不查询或保存容器。
//!
//! 单例保存跨请求状态，Scoped 服务保存本次请求上下文，Transient 格式化器展示
//! 按消费位置独立构造。具体的 build、query、scope 和 dispose 留在应用驱动层。

mod checkout;
mod config;
mod inventory;
mod payment;
mod storage;

pub use checkout::{CheckoutService, FraudCheck, ReceiptFormatter, RequestContext};
pub use config::AppConfig;
pub use inventory::Inventory;
pub use payment::PaymentGateway;
pub use storage::{Database, OrderStore, Repository};
