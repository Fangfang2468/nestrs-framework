use nestrs::{factory, injectable, lazy};

#[lazy]
#[injectable(lifetime = Scoped)]
struct DeferredSession;

struct DeferredApplication;

#[lazy(true)]
#[factory]
fn application(_session: DeferredSession) -> DeferredApplication {
    panic!("must not execute");
}

fn main() {
    panic!("must not execute");
}
