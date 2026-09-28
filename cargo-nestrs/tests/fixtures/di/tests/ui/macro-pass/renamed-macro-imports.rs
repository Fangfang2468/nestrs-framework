use nestrs as declarations;
use declarations::{factory as build_service, injectable as component};

#[component]
struct Database;

#[component]
struct Consumer {
    #[inject]
    database: Database,
    #[value(7)]
    number: usize,
}

struct Configuration;

#[build_service]
fn configuration() -> Configuration {
    Configuration
}

fn main() {
    assert_eq!(nestrs_core::__private::REFLECTED_PROVIDERS.len(), 3);
}
