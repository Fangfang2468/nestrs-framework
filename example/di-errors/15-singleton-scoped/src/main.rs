#![allow(dead_code)]

use nestrs::injectable;

#[injectable(lifetime = Scoped)]
struct RequestSession;

#[injectable(lifetime = Singleton)]
struct ApplicationCache {
    #[inject]
    session: RequestSession,
}

fn main() {}
