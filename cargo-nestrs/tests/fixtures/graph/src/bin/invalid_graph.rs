use nestrs::injectable;
fn forbidden(phase: &str) -> ! {
    let path = std::env::var_os("NESTRS_GRAPH_SENTINEL").expect("sentinel path");
    std::fs::write(path, phase).unwrap();
    panic!("graph must not execute {phase}");
}

struct Missing;

fn forbidden_value() -> usize {
    forbidden("invalid graph value")
}

#[injectable]
struct InvalidService {
    #[inject]
    missing: Missing,
    #[value(forbidden_value())]
    value: usize,
}

fn main() {
    forbidden("invalid graph business main");
}
