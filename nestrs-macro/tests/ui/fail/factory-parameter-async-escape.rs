use nestrs_macro::factory;

struct Database;

struct EscapedService {
    database: &'static Database,
}

#[factory]
async fn create(database: Database) -> EscapedService {
    async {}.await;
    EscapedService { database }
}

fn main() {}
