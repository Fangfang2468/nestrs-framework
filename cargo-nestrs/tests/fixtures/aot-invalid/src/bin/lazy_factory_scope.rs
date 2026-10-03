use nestrs::{factory, injectable};
use nestrs_core::LazyInjection;
#[injectable(lifetime = Scoped)]
struct Session;
struct Intermediate {
    session: LazyInjection<Session>,
}
#[factory(lifetime = Transient)]
fn intermediate(#[lazy] session: Session) -> Intermediate {
    Intermediate { session }
}
struct Application {
    intermediate: LazyInjection<Intermediate>,
}
#[factory]
fn application(#[lazy] intermediate: Intermediate) -> Application {
    Application { intermediate }
}
fn main() {
    panic!("must not execute");
}
