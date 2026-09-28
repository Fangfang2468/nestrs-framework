use nestrs::{factory};

struct Service;
struct OpaqueError;

#[factory]
fn create() -> Result<Service, OpaqueError> {
    Err(OpaqueError)
}

fn main() {}
