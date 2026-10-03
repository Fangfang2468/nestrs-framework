#![allow(dead_code)]

use nestrs::injectable;

#[injectable(lifetime = Scoped)]
struct RequestSession;

#[injectable(lifetime = Transient)]
struct Formatter {
    #[inject]
    session: RequestSession,
}

#[injectable(lifetime = Singleton)]
struct Application {
    #[inject]
    formatter: Formatter,
}

fn main() {}
