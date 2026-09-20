use nestrs_macro::factory;

struct Database;

impl Database {
    fn ping(&self) {}
}

struct Service;
struct FutureService;

fn accepts_copy<T: Copy>(_: T) {}

#[factory]
async fn create(database: Database) -> Service {
    accepts_copy(database);
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
