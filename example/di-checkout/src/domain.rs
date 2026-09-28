//! 下单用例的数据与业务错误，不依赖 DI 容器或实例生命周期。

use thiserror::Error;

/// 用户在结算时选择支付渠道；渠道实例由应用的静态 key 绑定。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaymentMethod {
    Card,
    Wallet,
}

/// 一次用户下单请求。金额由服务端库存目录计算，不能由调用者指定。
#[derive(Clone, Debug)]
pub struct CheckoutRequest {
    pub customer: String,
    pub sku: String,
    pub quantity: u32,
    pub payment: PaymentMethod,
    pub payment_token: String,
}

/// 已支付、已保存的订单。金额使用整数分，避免浮点金额误差。
#[derive(Clone, Debug)]
pub struct Order {
    pub id: String,
    pub request_id: usize,
    pub customer: String,
    pub sku: String,
    pub quantity: u32,
    pub total_cents: u64,
    pub payment_reference: String,
}

/// 用于演示同一个泛型仓储模板的另一种闭合类型。
#[derive(Clone, Debug)]
pub struct AuditEvent {
    pub message: String,
}

/// 业务失败与容器构建失败分开：库存不足、支付拒绝不会使 DI 容器失效。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CheckoutError {
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
