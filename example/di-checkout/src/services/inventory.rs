use nestrs::injectable;
use std::{collections::BTreeMap, sync::Mutex};

use crate::domain::CheckoutError;

struct Stock {
    available: u32,
    unit_price_cents: u64,
}

fn initial_products() -> Mutex<BTreeMap<String, Stock>> {
    Mutex::new(BTreeMap::from([(
        "KEYBOARD".to_owned(),
        Stock {
            available: 5,
            unit_price_cents: 19_900,
        },
    )]))
}

/// 跨请求共享的库存。预留在短临界区内完成，支付等待期间不持有 Mutex。
#[injectable]
pub struct Inventory {
    #[value(initial_products())]
    products: Mutex<BTreeMap<String, Stock>>,
    #[value(crate::observe::created("Inventory"))]
    id: usize,
}

impl Inventory {
    pub fn id(&self) -> usize {
        self.id
    }

    pub fn remaining(&self, sku: &str) -> Option<u32> {
        self.products
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(sku)
            .map(|stock| stock.available)
    }

    pub(crate) fn reserve(
        &self,
        sku: &str,
        quantity: u32,
    ) -> Result<Reservation<'_>, CheckoutError> {
        if quantity == 0 {
            return Err(CheckoutError::InvalidQuantity);
        }
        // Allocate the guard's owned identity before changing shared stock.
        let reserved_sku = sku.to_owned();
        let total_cents;
        {
            let mut products = self
                .products
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let stock = products
                .get_mut(sku)
                .ok_or_else(|| CheckoutError::UnknownProduct(sku.to_owned()))?;
            if quantity > stock.available {
                return Err(CheckoutError::OutOfStock {
                    sku: sku.to_owned(),
                    requested: quantity,
                    available: stock.available,
                });
            }
            total_cents = stock
                .unit_price_cents
                .checked_mul(u64::from(quantity))
                .ok_or(CheckoutError::TotalOverflow)?;
            stock.available -= quantity;
        }
        let reservation = Reservation {
            inventory: self,
            sku: reserved_sku,
            quantity,
            total_cents,
            committed: false,
        };
        // From here on even a panic while recording the event has a rollback guard.
        crate::observe::event(format!("业务库存预留 {sku} × {quantity}"));
        Ok(reservation)
    }
}

/// 未提交的预留在业务错误、panic 或 future 取消时归还库存。
///
/// 这是普通 Rust RAII，与容器的无参数 cleanup hook 无关。
pub(crate) struct Reservation<'inventory> {
    inventory: &'inventory Inventory,
    sku: String,
    quantity: u32,
    total_cents: u64,
    committed: bool,
}

impl Reservation<'_> {
    pub(crate) fn total_cents(&self) -> u64 {
        self.total_cents
    }

    pub(crate) fn commit(mut self) {
        self.committed = true;
        crate::observe::event(format!("业务库存提交 {} × {}", self.sku, self.quantity));
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let restored = {
            let mut products = self
                .inventory
                .products
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(stock) = products.get_mut(&self.sku) {
                // Only reservations reduce stock; each guard restores its own quantity once.
                stock.available += self.quantity;
                true
            } else {
                false
            }
        };
        if restored {
            crate::observe::event(format!("业务库存回滚 {} × {}", self.sku, self.quantity));
        }
    }
}
