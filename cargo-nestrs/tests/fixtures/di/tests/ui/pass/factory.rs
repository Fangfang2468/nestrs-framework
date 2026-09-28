use nestrs::{factory};

#[factory]
fn create() -> u8 {
    1
}

fn main() {
    let _ = create();
}
