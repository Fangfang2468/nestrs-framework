use nestrs::injectable;

struct Database;

#[injectable]
struct Consumer {
    #[inject(key = "database")]
    database: Database,
}

fn main() {}
