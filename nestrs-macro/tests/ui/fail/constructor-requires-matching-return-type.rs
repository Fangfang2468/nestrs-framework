use nestrs_macro::constructor;

struct Service;
struct Other;

impl Service {
    #[constructor]
    fn new() -> Other {
        Other
    }
}

fn main() {}
