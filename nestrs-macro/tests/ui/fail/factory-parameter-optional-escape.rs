use nestrs_macro::factory;

struct Database;

struct EscapedService {
    database: Option<&'static Database>,
}

#[factory]
fn create(database: Option<Database>) -> EscapedService {
    EscapedService { database }
}

fn main() {}
