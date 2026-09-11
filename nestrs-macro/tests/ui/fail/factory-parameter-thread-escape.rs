use nestrs_macro::factory;

struct Database;

impl Database {
    fn ping(&self) {}
}

struct Service;

#[factory]
fn create(database: Database) -> Service {
    std::thread::spawn(move || database.ping())
        .join()
        .expect("worker should finish");
    Service
}

fn main() {}
