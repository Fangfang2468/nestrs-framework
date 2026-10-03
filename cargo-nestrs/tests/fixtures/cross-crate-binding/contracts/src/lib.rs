//! Business contracts have no dependency on the DI runtime or tooling.

// 业务只通过 contracts 使用这些接口；不能要求每个消费者再直接依赖其定义 crate。
// HiddenArgument 没有重导出，WithHiddenArgument 的被动能力仍不能生成非法类型实参。
pub use transitive_contracts::{ExposedMarker, PublicCapability, WithHiddenArgument};

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

    pub trait TextView<'a> {
        fn text(&'a self) -> &'a str;
    }

    pub trait TextPort: for<'a> TextView<'a> + Send + Sync {
        fn identity(&self) -> usize;
    }

    pub trait CountView<'a> {
        type Item;
        fn count(&'a self) -> Self::Item;
    }

    pub trait CountPort: for<'a> CountView<'a> + Send + Sync {
        fn identity(&self) -> usize;
    }

    pub trait BorrowedView<'a> {
        type Item;
        fn borrowed(&'a self) -> Self::Item;
    }

    pub trait BorrowedPort: for<'a> BorrowedView<'a, Item = &'a str> + Send + Sync {
        fn identity(&self) -> usize;
    }

    // Its concrete Item depends on the hidden parent lifetime. Rust has no
    // principal-object spelling for that equality unless it is stated here.
    // This unrequested capability must not poison the producer's compilation.
    pub trait UnspecifiedBorrowedPort: for<'a> BorrowedView<'a> + Send + Sync {}

    pub trait AmbiguousPort: Send + Sync {}

    /// No crate implements this optional integration.
    pub trait FraudPlugin: Send + Sync {}
}

// The compiler's definition path contains a private module. Downstream code
// must use this real public path, and must preserve the renamed trait's identity.
pub use ports::{
    AmbiguousPort, BorrowedPort, BorrowedView, ConnectionPort, ConnectionView, CountPort,
    CountView, DeliveryPort, DormantPort, EntityReader, FraudPlugin, IdentityPort,
    InventoryPort as CatalogPort, RepositoryPort, TextPort, TextView, TrackingPort,
    UnspecifiedBorrowedPort, UserEntity,
};
