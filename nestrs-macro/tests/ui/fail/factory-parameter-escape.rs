use nestrs_core::__private::{FactoryParameter, Inject};
use nestrs_macro::factory;

struct Database;

struct EscapedService {
    database: Inject<Database, FactoryParameter<'static>>,
}

#[factory]
fn create(database: Database) -> EscapedService {
    EscapedService { database }
}

fn main() {}
