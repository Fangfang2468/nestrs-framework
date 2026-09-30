use nestrs::injectable;

use crate::domain::Order;

/// 无共享业务状态，每个消费位置都会得到独立实例；实例编号只用于展示 DI 行为。
#[injectable(lifetime = Transient)]
pub(crate) struct ReceiptFormatter {
    #[value(crate::observe::created("ReceiptFormatter"))]
    id: usize,
}

impl ReceiptFormatter {
    #[cfg(test)]
    pub(crate) fn id(&self) -> usize {
        self.id
    }

    pub(crate) fn format(&self, order: &Order) -> String {
        format!(
            "订单 {} | 客户 {} | {} × {} | 合计 ¥{}.{:02} | 支付 {}",
            order.id,
            order.customer,
            order.sku,
            order.quantity,
            order.total_cents / 100,
            order.total_cents % 100,
            order.payment_reference,
        )
    }
}

impl Drop for ReceiptFormatter {
    fn drop(&mut self) {
        crate::observe::event(format!("drop ReceiptFormatter #{}", self.id));
    }
}
