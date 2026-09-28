use nestrs::{injectable};

#[injectable]
struct Service {
    #[value("name")]
    name: String,
}

fn main() {
    let _ = Service::__nestrs_construct();
}
