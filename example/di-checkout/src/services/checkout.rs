use nestrs::injectable;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::domain::{CheckoutError, CheckoutRequest, Order, PaymentMethod};

use super::{Inventory, OrderStore, PaymentGateway};

/// 请求级上下文在同一个 scope 内复用；不同用户请求应创建不同 scope。
#[injectable(lifetime = Scoped, cleanup = "cleanup_request_context")]
pub struct RequestContext {
    #[value(crate::observe::created("RequestContext"))]
    id: usize,
}

impl RequestContext {
    pub fn id(&self) -> usize {
        self.id
    }
}

async fn cleanup_request_context() {
    crate::observe::event("cleanup hook RequestContext（零参数，仅记录钩子调用）");
}

impl Drop for RequestContext {
    fn drop(&mut self) {
        crate::observe::event(format!("drop RequestContext #{}", self.id));
    }
}

/// 无共享业务状态，每个消费位置都会得到独立实例；实例编号只用于展示 DI 行为。
#[injectable(lifetime = Transient)]
pub struct ReceiptFormatter {
    #[value(crate::observe::created("ReceiptFormatter"))]
    id: usize,
}

impl ReceiptFormatter {
    pub fn id(&self) -> usize {
        self.id
    }

    pub fn format(&self, order: &Order) -> String {
        format!(
            "订单 {} | 客户 {} | {} × {} | 合计 ¥{}.{:02} | 支付 {} | request #{} | formatter #{}",
            order.id,
            order.customer,
            order.sku,
            order.quantity,
            order.total_cents / 100,
            order.total_cents % 100,
            order.payment_reference,
            order.request_id,
            self.id,
        )
    }
}

/// 本例不注册实现，用于展示可选 trait 依赖的合法缺席。
pub trait FraudCheck: Send + Sync {
    fn check(&self, request: &CheckoutRequest) -> Result<(), CheckoutError>;
}

static NEXT_ORDER: AtomicUsize = AtomicUsize::new(1);

/// Scoped 应用服务串联校验、库存预留、支付与保存；构造本身不执行任何下单业务。
#[injectable(lifetime = Scoped, cleanup = "cleanup_checkout")]
pub struct CheckoutService {
    #[inject]
    context: RequestContext,
    #[inject]
    orders: dyn OrderStore,
    #[inject]
    inventory: Inventory,
    #[inject(key = "card")]
    card: dyn PaymentGateway,
    #[inject(key = "wallet")]
    wallet: dyn PaymentGateway,
    #[inject]
    formatter: ReceiptFormatter,
    #[inject]
    fraud: Option<dyn FraudCheck>,
    #[value(crate::observe::created("CheckoutService"))]
    id: usize,
}

impl CheckoutService {
    pub async fn place_order(&self, request: CheckoutRequest) -> Result<Order, CheckoutError> {
        if request.quantity == 0 {
            return Err(CheckoutError::InvalidQuantity);
        }
        if request.customer.trim().is_empty() {
            return Err(CheckoutError::InvalidCustomer);
        }
        if let Some(fraud) = &self.fraud {
            fraud.check(&request)?;
        }

        let reservation = self.inventory.reserve(&request.sku, request.quantity)?;
        let total_cents = reservation.total_cents();
        let gateway: &dyn PaymentGateway = match request.payment {
            PaymentMethod::Card => &*self.card,
            PaymentMethod::Wallet => &*self.wallet,
        };
        let payment_reference = gateway.charge(total_cents, &request.payment_token).await?;
        let number = NEXT_ORDER.fetch_add(1, Ordering::Relaxed);
        let order = Order {
            id: format!("ORD-{number:04}"),
            request_id: self.context.id(),
            customer: request.customer,
            sku: request.sku,
            quantity: request.quantity,
            total_cents,
            payment_reference,
        };
        self.orders.save(order.clone());
        reservation.commit();
        crate::observe::event(format!(
            "业务订单保存 {}（request #{}）",
            order.id, order.request_id
        ));
        Ok(order)
    }

    pub fn context_id(&self) -> usize {
        self.context.id()
    }
    pub fn store_id(&self) -> usize {
        self.orders.instance_id()
    }
    pub fn database_id(&self) -> usize {
        self.orders.database_id()
    }
    pub fn formatter_id(&self) -> usize {
        self.formatter.id()
    }
    pub fn format_receipt(&self, order: &Order) -> String {
        self.formatter.format(order)
    }
    pub fn has_fraud_check(&self) -> bool {
        self.fraud.is_some()
    }
}

async fn cleanup_checkout() {
    crate::observe::event("cleanup hook CheckoutService（零参数，仅记录钩子调用）");
}

impl Drop for CheckoutService {
    fn drop(&mut self) {
        crate::observe::event(format!("drop CheckoutService #{}", self.id));
    }
}
