//! Upstream registrations contain no DI queries and no handwritten bindings.

use std::sync::atomic::{AtomicUsize, Ordering};

static CONSTRUCTIONS: AtomicUsize = AtomicUsize::new(0);
static CONNECTION_CLEANUPS: AtomicUsize = AtomicUsize::new(0);
static CONNECTION_DROPS: AtomicUsize = AtomicUsize::new(0);

fn next_id() -> usize {
    CONSTRUCTIONS.fetch_add(1, Ordering::SeqCst) + 1
}

mod implementation {
    use super::{CONNECTION_CLEANUPS, CONNECTION_DROPS, Ordering, next_id};
    use contracts::{
        AmbiguousPort, CatalogPort, ConnectionPort, DeliveryPort, DormantPort, EntityReader,
        IdentityPort, RepositoryPort, UserEntity,
    };
    use nestrs::{factory, injectable, primary};

    #[injectable]
    pub struct Inventory {
        #[value(next_id())]
        id: usize,
    }

    impl Inventory {
        pub fn id(&self) -> usize {
            self.id
        }
    }

    impl CatalogPort for Inventory {
        fn available(&self) -> usize {
            17
        }

        fn identity(&self) -> usize {
            self as *const Self as usize
        }
    }

    // A factory-only private type: downstream application source cannot name it.
    // The implementation must generate a legal bridge here without making it pub.
    struct PrivateConnection {
        id: usize,
    }

    impl ConnectionPort for PrivateConnection {
        fn connection_id(&self) -> usize {
            self.id
        }

        fn identity(&self) -> usize {
            self as *const Self as usize
        }
    }

    impl Drop for PrivateConnection {
        fn drop(&mut self) {
            CONNECTION_DROPS.fetch_add(1, Ordering::SeqCst);
        }
    }

    async fn cleanup_connection() {
        CONNECTION_CLEANUPS.fetch_add(1, Ordering::SeqCst);
    }

    #[factory(cleanup = "cleanup_connection")]
    async fn connection() -> PrivateConnection {
        tokio::task::yield_now().await;
        PrivateConnection { id: next_id() }
    }

    #[injectable]
    #[primary]
    pub struct Service {
        #[value(next_id())]
        id: usize,
        #[value("primary")]
        source: &'static str,
    }

    impl Service {
        pub fn id(&self) -> usize {
            self.id
        }
    }

    impl DeliveryPort for Service {
        fn source(&self) -> &'static str {
            self.source
        }

        fn identity(&self) -> usize {
            self as *const Self as usize
        }
    }

    // The binding belongs to the concrete/trait pair and must serve both keys
    // without becoming two duplicate binding registrations.
    #[factory(key = "audit")]
    fn audit_delivery() -> Service {
        Service {
            id: next_id(),
            source: "audit",
        }
    }

    #[injectable]
    struct Unrequested;

    impl DormantPort for Unrequested {}

    // This closed impl is visible to rustc, but neither its trait nor concrete
    // type is requested. Activating its automatic descriptor would incorrectly
    // materialize a generic provider with a missing required dependency.
    #[allow(dead_code)]
    struct UnusedEntity;

    #[allow(dead_code)]
    trait MissingResource: Send + Sync {}

    #[allow(dead_code)]
    #[injectable]
    struct DormantRepository<T> {
        marker: std::marker::PhantomData<T>,
        #[inject]
        resource: dyn MissingResource,
    }

    impl DormantPort for DormantRepository<UnusedEntity> {}

    trait PrivateRepositoryDependency: Send + Sync {
        fn count(&self) -> usize;
    }

    #[injectable]
    struct RepositoryConnection {
        #[value(23)]
        count: usize,
    }

    impl PrivateRepositoryDependency for RepositoryConnection {
        fn count(&self) -> usize {
            self.count
        }
    }

    // Neither this generic type nor its dependency interface is exported.
    // Only downstream dyn queries should activate the known closed impls.
    #[allow(dead_code)]
    #[injectable]
    struct Repository<T> {
        marker: std::marker::PhantomData<T>,
        #[inject]
        connection: dyn PrivateRepositoryDependency,
    }

    impl IdentityPort for Repository<UserEntity> {
        fn identity(&self) -> usize {
            self as *const Self as usize
        }
    }

    impl EntityReader for Repository<UserEntity> {
        type Entity = UserEntity;

        fn entity_name(&self) -> &'static str {
            "UserEntity"
        }
    }

    impl RepositoryPort<UserEntity> for Repository<UserEntity> {
        fn count(&self) -> usize {
            self.connection.count()
        }

        fn identity(&self) -> usize {
            self as *const Self as usize
        }
    }

    struct Conflict;

    impl AmbiguousPort for Conflict {}

    #[factory(key = "conflict")]
    fn conflict() -> Conflict {
        next_id();
        Conflict
    }
}

pub use implementation::{Inventory as Catalog, Service};

pub fn total_constructions() -> usize {
    CONSTRUCTIONS.load(Ordering::SeqCst)
}

pub fn connection_cleanup_count() -> usize {
    CONNECTION_CLEANUPS.load(Ordering::SeqCst)
}

pub fn connection_drop_count() -> usize {
    CONNECTION_DROPS.load(Ordering::SeqCst)
}
