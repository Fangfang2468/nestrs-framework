use nestrs_macro::factory;

struct Database;

impl Database {
    fn ping(&self) {}
}

struct Service;
struct FutureService;

#[factory]
async fn create(database: Database) -> Service {
    database.ping();
    async {}.await;
    database.ping();
    Service
}

#[factory]
fn create_future(database: Database) -> impl ::core::future::Future<Output = FutureService> {
    async move {
        database.ping();
        FutureService
    }
}

fn main() {}
