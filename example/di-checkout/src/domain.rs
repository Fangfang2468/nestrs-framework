//! 订单数据、业务错误和协作者接口，不依赖容器或具体基础设施。

use std::{future::Future, pin::Pin};

use thiserror::Error;

/// 用户在结算时选择支付渠道；渠道实例由应用的静态 key 绑定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PaymentMethod {
    Card,
    Wallet,
}

/// 一次用户下单请求。金额由服务端库存目录计算，不能由调用者指定。
#[derive(Clone, Debug)]
pub(crate) struct CheckoutRequest {
    pub customer: String,
    pub sku: String,
    pub quantity: u32,
    pub payment: PaymentMethod,
    pub payment_token: String,
}

/// 已支付、已保存的订单。金额使用整数分，避免浮点金额误差。
#[derive(Clone, Debug)]
pub(crate) struct Order {
    pub id: String,
    pub request_id: usize,
    pub customer: String,
    pub sku: String,
    pub quantity: u32,
    pub total_cents: u64,
    pub payment_reference: String,
}

/// 每次实际处理请求后记录的审计事件，和订单分别保存在各自的表中。
#[derive(Clone, Debug)]
pub(crate) struct AuditEvent {
    pub message: String,
}

/// 业务失败与容器构建失败分开：库存不足、支付拒绝不会使 DI 容器失效。
#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum CheckoutError {
    #[error("商品数量必须大于零")]
    InvalidQuantity,
    #[error("客户名称不能为空")]
    InvalidCustomer,
    #[error("商品不存在：{0}")]
    UnknownProduct(String),
    #[error("商品 {sku} 库存不足：请求 {requested} 件，可用 {available} 件")]
    OutOfStock {
        sku: String,
        requested: u32,
        available: u32,
    },
    #[error("支付渠道拒绝了本次支付")]
    PaymentDeclined,
    #[error("订单金额超出支持范围")]
    TotalOverflow,
    #[error("风控检查未通过：{0}")]
    FraudRejected(String),
}

/// 结算依赖订单存储能力，基础设施负责选择存储方式。
pub(crate) trait OrderStore: Send + Sync {
    fn save(&self, order: Order);
    fn all(&self) -> Vec<Order>;
    #[cfg(test)]
    fn instance_id(&self) -> usize;
    #[cfg(test)]
    fn database_id(&self) -> usize;
}

pub(crate) type ChargeFuture<'request> =
    Pin<Box<dyn Future<Output = Result<String, CheckoutError>> + Send + 'request>>;

/// 支付能力按业务渠道选择；客户端初始化和 key 注册属于基础设施。
pub(crate) trait PaymentGateway: Send + Sync {
    fn channel(&self) -> &'static str;
    fn charge<'request>(
        &'request self,
        total_cents: u64,
        token: &'request str,
    ) -> ChargeFuture<'request>;
    #[cfg(test)]
    fn id(&self) -> usize;
}

/// 可选的风控能力。本地示例不安装实现，启用时返回拒绝原因。
pub(crate) trait FraudCheck: Send + Sync {
    fn check(&self, request: &CheckoutRequest) -> Result<(), String>;
}
