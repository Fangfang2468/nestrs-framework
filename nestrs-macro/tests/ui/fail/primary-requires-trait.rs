use nestrs_macro::primary;

struct NotATrait;

#[primary(NotATrait)]
struct InvalidPrimary;

fn main() {}
