use nestrs::injectable;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{
    domain::{
        CheckoutError, CheckoutRequest, FraudCheck, Order, OrderStore, PaymentGateway,
        PaymentMethod,
    },
    infrastructure::Inventory,
};

use super::{context::RequestContext, receipt::ReceiptFormatter};

static NEXT_ORDER: AtomicUsize = AtomicUsize::new(1);

/// Scoped 应用服务串联校验、库存预留、支付与保存；构造本身不执行任何下单业务。
#[injectable(lifetime = Scoped, cleanup = "cleanup_checkout")]
pub(crate) struct CheckoutService {
    #[inject]
    context: RequestContext,
    #[inject]
    orders: dyn OrderStore,
    #[inject]
    inventory: Inventory,
    #[inject("card")]
    card: dyn PaymentGateway,
    #[inject("wallet")]
    wallet: dyn PaymentGateway,
    #[inject]
    formatter: ReceiptFormatter,
    #[inject]
    fraud: Option<dyn FraudCheck>,
    #[value(crate::observe::created("CheckoutService"))]
    id: usize,
}

impl CheckoutService {
    pub(crate) async fn place_order(
        &self,
        request: CheckoutRequest,
    ) -> Result<Order, CheckoutError> {
        if request.quantity == 0 {
            return Err(CheckoutError::InvalidQuantity);
        }
        if request.customer.trim().is_empty() {
            return Err(CheckoutError::InvalidCustomer);
        }
        if let Some(fraud) = &self.fraud {
            fraud
                .check(&request)
                .map_err(CheckoutError::FraudRejected)?;
        }

        let reservation = self.inventory.reserve(&request.sku, request.quantity)?;
        let total_cents = reservation.total_cents();
        let gateway: &dyn PaymentGateway = match request.payment {
            PaymentMethod::Card => &*self.card,
            PaymentMethod::Wallet => &*self.wallet,
        };
        crate::observe::event(format!("业务发起支付 [{}]", gateway.channel()));
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

    pub(crate) fn context_id(&self) -> usize {
        self.context.id()
    }
    #[cfg(test)]
    pub(crate) fn store_id(&self) -> usize {
        self.orders.instance_id()
    }
    #[cfg(test)]
    pub(crate) fn database_id(&self) -> usize {
        self.orders.database_id()
    }
    #[cfg(test)]
    pub(crate) fn formatter_id(&self) -> usize {
        self.formatter.id()
    }
    pub(crate) fn format_receipt(&self, order: &Order) -> String {
        self.formatter.format(order)
    }
    #[cfg(test)]
    pub(crate) fn has_fraud_check(&self) -> bool {
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
