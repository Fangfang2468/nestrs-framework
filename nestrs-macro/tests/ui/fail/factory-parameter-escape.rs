use nestrs_macro::factory;

struct Database;

struct EscapedService {
    database: &'static Database,
}

#[factory]
fn create(database: Database) -> EscapedService {
    EscapedService { database }
}

fn main() {}
