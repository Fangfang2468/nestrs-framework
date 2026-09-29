//! Business contracts have no dependency on the DI runtime or tooling.

mod ports {
    pub trait InventoryPort: Send + Sync {
        fn available(&self) -> usize;
        fn identity(&self) -> usize;
    }

    pub trait ConnectionPort: Send + Sync {
        fn connection_id(&self) -> usize;
        fn identity(&self) -> usize;
    }

    pub trait ConnectionView: Send + Sync {
        fn view_identity(&self) -> usize;
        fn view_connection_id(&self) -> usize;
    }

    // The provider crate has no local impl of ConnectionView. Its capability
    // follows from an external blanket impl with a real trait obligation.
    impl<T: ConnectionPort> ConnectionView for T {
        fn view_identity(&self) -> usize {
            self.identity()
        }

        fn view_connection_id(&self) -> usize {
            self.connection_id()
        }
    }

    pub trait DeliveryPort: Send + Sync {
        fn source(&self) -> &'static str;
        fn identity(&self) -> usize;
    }

    pub trait TrackingPort: Send + Sync {
        fn carrier(&self) -> &'static str;
        fn identity(&self) -> usize;
    }

    /// Multiple implementations exist, but no DI request mentions this trait.
    pub trait DormantPort: Send + Sync {}

    pub struct UserEntity;

    pub trait IdentityPort: Send + Sync {
        fn identity(&self) -> usize;
    }

    pub trait EntityReader: IdentityPort {
        type Entity: Send + Sync + 'static;

        fn entity_name(&self) -> &'static str;
    }

    pub trait RepositoryPort<T>: Send + Sync {
        fn count(&self) -> usize;
        fn identity(&self) -> usize;
    }

    pub trait AmbiguousPort: Send + Sync {}

    /// No crate implements this optional integration.
    pub trait FraudPlugin: Send + Sync {}
}

// The compiler's definition path contains a private module. Downstream code
// must use this real public path, and must preserve the renamed trait's identity.
pub use ports::{
    AmbiguousPort, ConnectionPort, ConnectionView, DeliveryPort, DormantPort, EntityReader,
    FraudPlugin, IdentityPort, InventoryPort as CatalogPort, RepositoryPort, TrackingPort,
    UserEntity,
};
