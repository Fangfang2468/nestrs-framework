use nestrs_core::__private::{FactoryParameter, Inject};
use nestrs_macro::factory;

struct Database;

struct EscapedService {
    database: Inject<Database, FactoryParameter<'static>>,
}

#[factory]
async fn create(database: Database) -> EscapedService {
    async {}.await;
    EscapedService { database }
}

fn main() {}
