//! cargo nestrs run --manifest-path cargo-nestrs/tests/fixtures/di/Cargo.toml --example di
use nestrs::{factory, injectable};
use nestrs_core::{InitializationMode, ServiceProvider, ServiceProviderOptions};

use std::{marker::PhantomData, num::NonZeroUsize};

struct Database {
    connection: String,
}
async fn close_database() {
    println!("database cleanup");
}
#[factory(cleanup = "close_database")]
async fn connect_database() -> Result<Database, &'static str> {
    tokio::task::yield_now().await;
    Ok(Database {
        connection: "example database".into(),
    })
}
trait RepositoryPort: Send + Sync {
    fn name(&self) -> &str;
}
#[injectable]
struct Repository<T> {
    #[inject]
    database: Database,
    marker: PhantomData<T>,
}
struct User;

impl RepositoryPort for Repository<User> {
    fn name(&self) -> &str {
        &self.database.connection
    }
}
#[injectable(lifetime = Scoped)]
struct RequestHandler {
    #[inject]
    repository: dyn RepositoryPort,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ServiceProvider::build_with_options(ServiceProviderOptions {
        initialization: InitializationMode::Eager,
        max_concurrent_activations: NonZeroUsize::new(8).unwrap(),
    })
    .await?;
    let scope = provider.create_scope();
    scope.warm_up().await?;
    let handler = scope
        .service_provider()
        .get_required_service::<RequestHandler>()
        .await?;
    println!("request uses {}", handler.repository.name());
    scope.dispose_async().await?;
    provider.dispose_async().await?;
    Ok(())
}
