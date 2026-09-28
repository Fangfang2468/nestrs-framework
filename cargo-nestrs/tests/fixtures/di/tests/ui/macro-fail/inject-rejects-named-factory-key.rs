use nestrs::factory;

struct Database;
struct Service;

#[factory]
fn service(#[inject(key = 7)] database: Database) -> Service {
    let _ = database;
    Service
}

fn main() {}
