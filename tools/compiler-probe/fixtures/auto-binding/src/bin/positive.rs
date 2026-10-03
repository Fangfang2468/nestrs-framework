//! Ordinary service declarations and impls; this binary has no explicit binding.

use nestrs::{factory, injectable, primary};
use nestrs_core::{ServiceKey, ServiceProvider};
use std::{
    marker::PhantomData,
    sync::atomic::{AtomicUsize, Ordering},
};

static CONNECTIONS: AtomicUsize = AtomicUsize::new(0);
static CONNECTION_CLEANUPS: AtomicUsize = AtomicUsize::new(0);
static CONNECTION_DROPS: AtomicUsize = AtomicUsize::new(0);
static SESSIONS: AtomicUsize = AtomicUsize::new(0);

mod warehouse {
    use nestrs::injectable;
    use nestrs_core::ServiceProvider;

    pub(crate) trait StockPort: Send + Sync {
        fn available(&self) -> usize;
        fn identity(&self) -> usize;
    }

    // The generator has to preserve module privacy; exporting this type solely
    // to make an external generated module compile would weaken this fixture.
    #[injectable]
    struct Inventory {
        #[value(17)]
        available: usize,
    }

    macro_rules! implement_stock {
        ($service:ty) => {
            impl StockPort for $service {
                fn available(&self) -> usize {
                    self.available
                }

                fn identity(&self) -> usize {
                    self as *const Self as usize
                }
            }
        };
    }

    implement_stock!(Inventory);

    pub(crate) async fn assert_identity(provider: &ServiceProvider) {
        let concrete = provider.get_required_service::<Inventory>().await.unwrap();
        let interface = provider
            .get_required_service::<dyn StockPort>()
            .await
            .unwrap();
        assert_eq!(interface.available(), 17);
        assert_eq!(interface.identity(), concrete as *const Inventory as usize);
    }
}

#[derive(Default)]
struct User;

trait RepositoryPort<T>: Send + Sync {
    fn identity(&self) -> usize;
    fn entity_name(&self) -> &'static str;
}

#[injectable]
struct Repository<T> {
    marker: PhantomData<T>,
    #[value(29)]
    _allocation: usize,
}

impl<T: Send + Sync> RepositoryPort<T> for Repository<T> {
    fn identity(&self) -> usize {
        self as *const Self as usize
    }

    fn entity_name(&self) -> &'static str {
        std::any::type_name::<T>()
    }
}

// This closed query root bounds generic discovery. The tool must not attempt
// to enumerate all possible T for the implementation above.
type UserRepository = Repository<User>;

trait ConnectionPort: Send + Sync {
    fn id(&self) -> usize;
    fn identity(&self) -> usize;
}

struct Connection {
    id: usize,
}

impl Drop for Connection {
    fn drop(&mut self) {
        CONNECTION_DROPS.fetch_add(1, Ordering::SeqCst);
    }
}

impl ConnectionPort for Connection {
    fn id(&self) -> usize {
        self.id
    }

    fn identity(&self) -> usize {
        self as *const Self as usize
    }
}

async fn cleanup_connection() {
    CONNECTION_CLEANUPS.fetch_add(1, Ordering::SeqCst);
}

// A factory-only concrete type has no ProviderDefinition implementation.
#[factory(cleanup = "cleanup_connection")]
async fn connection() -> Connection {
    tokio::task::yield_now().await;
    Connection {
        id: CONNECTIONS.fetch_add(1, Ordering::SeqCst) + 1,
    }
}

trait SessionPort: Send + Sync {
    fn id(&self) -> usize;
    fn connection_id(&self) -> usize;
    fn identity(&self) -> usize;
}

struct Session {
    id: usize,
    connection_id: usize,
}

impl SessionPort for Session {
    fn id(&self) -> usize {
        self.id
    }

    fn connection_id(&self) -> usize {
        self.connection_id
    }

    fn identity(&self) -> usize {
        self as *const Self as usize
    }
}

#[factory(lifetime = Scoped)]
async fn session(connection: dyn ConnectionPort) -> Session {
    tokio::task::yield_now().await;
    Session {
        id: SESSIONS.fetch_add(1, Ordering::SeqCst) + 1,
        connection_id: connection.id(),
    }
}

trait GreetingPort: Send + Sync {
    fn text(&self) -> &'static str;
}

#[injectable]
#[primary]
struct English;

impl GreetingPort for English {
    fn text(&self) -> &'static str {
        "hello"
    }
}

#[injectable]
struct French;

impl GreetingPort for French {
    fn text(&self) -> &'static str {
        "bonjour"
    }
}

#[injectable(key = "zh")]
struct Chinese;

impl GreetingPort for Chinese {
    fn text(&self) -> &'static str {
        "你好"
    }
}

trait MissingPlugin: Send + Sync {}

#[injectable(lifetime = Scoped)]
struct Checkout {
    #[inject]
    inventory: dyn warehouse::StockPort,
    #[inject]
    session: dyn SessionPort,
    #[inject]
    repository: dyn RepositoryPort<User>,
    #[inject]
    optional_connection: Option<dyn ConnectionPort>,
    #[inject]
    absent_plugin: Option<dyn MissingPlugin>,
    #[inject("zh")]
    greeting: dyn GreetingPort,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    assert_eq!(CONNECTIONS.load(Ordering::SeqCst), 0);
    warehouse::assert_identity(&provider).await;

    let concrete = provider.get_required_service::<Connection>().await.unwrap();
    let interface = provider
        .get_required_service::<dyn ConnectionPort>()
        .await
        .unwrap();
    assert_eq!(interface.identity(), concrete as *const Connection as usize);
    assert_eq!(CONNECTIONS.load(Ordering::SeqCst), 1);

    let repository = provider
        .get_required_service::<UserRepository>()
        .await
        .unwrap();
    let repository_port = provider
        .get_required_service::<dyn RepositoryPort<User>>()
        .await
        .unwrap();
    assert_eq!(
        repository_port.identity(),
        repository as *const UserRepository as usize
    );
    assert!(repository_port.entity_name().ends_with("::User"));

    let greeting = provider
        .get_required_service::<dyn GreetingPort>()
        .await
        .unwrap();
    assert_eq!(greeting.text(), "hello");
    let chinese = provider
        .get_required_keyed_service::<dyn GreetingPort>(ServiceKey::Named("zh".into()))
        .await
        .unwrap();
    assert_eq!(chinese.text(), "你好");
    assert!(
        provider
            .get_service::<dyn MissingPlugin>()
            .await
            .unwrap()
            .is_none()
    );
    assert!(provider.get_service::<dyn SessionPort>().await.is_err());

    let left = provider.create_scope();
    let right = provider.create_scope();
    let left_session = left
        .service_provider()
        .get_required_service::<dyn SessionPort>()
        .await
        .unwrap();
    let left_concrete = left
        .service_provider()
        .get_required_service::<Session>()
        .await
        .unwrap();
    let right_session = right
        .service_provider()
        .get_required_service::<dyn SessionPort>()
        .await
        .unwrap();
    assert_eq!(
        left_session.identity(),
        left_concrete as *const Session as usize
    );
    assert_ne!(left_session.id(), right_session.id());
    assert_eq!(left_session.connection_id(), right_session.connection_id());
    let checkout = left
        .service_provider()
        .get_required_service::<Checkout>()
        .await
        .unwrap();
    assert_eq!(checkout.inventory.available(), 17);
    assert_eq!(checkout.session.id(), left_session.id());
    assert_eq!(checkout.repository.identity(), repository_port.identity());
    assert_eq!(
        checkout.optional_connection.as_ref().unwrap().id(),
        concrete.id
    );
    assert!(checkout.absent_plugin.is_none());
    assert_eq!(checkout.greeting.text(), "你好");

    left.dispose_async().await.unwrap();
    right.dispose_async().await.unwrap();
    provider.dispose_async().await.unwrap();
    assert_eq!(CONNECTION_CLEANUPS.load(Ordering::SeqCst), 1);
    assert_eq!(CONNECTION_DROPS.load(Ordering::SeqCst), 1);
    println!(
        "auto-binding positive: private/macro/generic/factory/scoped/optional/key/primary passed"
    );
}
