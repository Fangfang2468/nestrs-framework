use nestrs::injectable;
#[injectable(lifetime = Scoped)]
struct Session;
#[injectable(lifetime = Transient)]
struct Intermediate {
    #[inject]
    #[lazy]
    session: Session,
}
#[injectable]
struct Application {
    #[inject]
    intermediate: Intermediate,
}
fn main() {}
