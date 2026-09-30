//! This library first requests interfaces implemented by its upstream provider.

use contracts::{CatalogPort, ConnectionPort, DeliveryPort, FraudPlugin, TrackingPort};
use nestrs::injectable;

#[injectable(lifetime = Scoped)]
pub struct Checkout {
    #[inject]
    catalog: dyn CatalogPort,
    #[inject]
    connection: dyn ConnectionPort,
    #[inject]
    delivery: dyn DeliveryPort,
    #[inject]
    tracking: dyn TrackingPort,
    #[inject]
    fraud: Option<dyn FraudPlugin>,
}

impl Checkout {
    pub fn available(&self) -> usize {
        self.catalog.available()
    }

    pub fn catalog_identity(&self) -> usize {
        self.catalog.identity()
    }

    pub fn connection_identity(&self) -> usize {
        self.connection.identity()
    }

    pub fn connection_id(&self) -> usize {
        self.connection.connection_id()
    }

    pub fn delivery_source(&self) -> &'static str {
        self.delivery.source()
    }

    pub fn delivery_identity(&self) -> usize {
        self.delivery.identity()
    }

    pub fn has_fraud_plugin(&self) -> bool {
        self.fraud.is_some()
    }

    pub fn carrier(&self) -> &'static str {
        self.tracking.carrier()
    }

    pub fn tracking_identity(&self) -> usize {
        self.tracking.identity()
    }
}

// A normal Rust reference keeps the upstream crate in this library's linked
// dependency closure; it adds no provider, binding, root, or service query.
pub fn linked_provider_constructions() -> usize {
    primary_provider::total_constructions()
}

/// 延迟字段定义在业务库中，目标则是上游库不公开的 async factory 成功类型。
/// 下游只能通过接口调用，整个链条不得依赖公开 concrete 或手工 binding。
#[injectable]
pub struct LazyConnectionConsumer {
    #[nestrs::lazy]
    #[nestrs::inject]
    connection: dyn ConnectionPort,
}

impl LazyConnectionConsumer {
    pub async fn connection_identity(&self) -> Result<usize, nestrs_core::ResolveError> {
        Ok(self.connection.get().await?.identity())
    }
}
