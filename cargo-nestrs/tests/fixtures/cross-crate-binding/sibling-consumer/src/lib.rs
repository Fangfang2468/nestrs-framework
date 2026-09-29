//! This library deliberately does not depend on either concrete provider crate.

use contracts::{CatalogPort, DeliveryPort, FraudPlugin};
use nestrs::injectable;

#[injectable(lifetime = Scoped)]
pub struct Dispatch {
    #[inject]
    catalog: dyn CatalogPort,
    #[inject]
    preferred: dyn DeliveryPort,
    #[inject("audit")]
    audit: dyn DeliveryPort,
    #[inject]
    optional_preferred: Option<dyn DeliveryPort>,
    #[inject]
    fraud: Option<dyn FraudPlugin>,
}

impl Dispatch {
    pub fn catalog_identity(&self) -> usize {
        self.catalog.identity()
    }

    pub fn preferred_source(&self) -> &'static str {
        self.preferred.source()
    }

    pub fn preferred_identity(&self) -> usize {
        self.preferred.identity()
    }

    pub fn audit_source(&self) -> &'static str {
        self.audit.source()
    }

    pub fn audit_identity(&self) -> usize {
        self.audit.identity()
    }

    pub fn optional_identity(&self) -> Option<usize> {
        self.optional_preferred.as_ref().map(|port| port.identity())
    }

    pub fn has_fraud_plugin(&self) -> bool {
        self.fraud.is_some()
    }
}
