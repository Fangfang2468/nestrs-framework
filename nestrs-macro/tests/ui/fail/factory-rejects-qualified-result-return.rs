use nestrs_macro::factory;

mod external {
    pub type Result<T> = ::std::result::Result<T, &'static str>;
}

struct Service;

#[factory]
fn make() -> external::Result<Service> {
    Ok(Service)
}

fn main() {}
