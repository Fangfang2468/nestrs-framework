use nestrs::injectable;

fn forbidden(phase: &str) -> ! {
    let path = std::env::var_os("NESTRS_GRAPH_SENTINEL").expect("sentinel path");
    std::fs::write(path, phase).unwrap();
    panic!("graph must not execute {phase}");
}

#[injectable]
struct FeatureOnly {
    #[value(forbidden("feature value"))]
    value: usize,
}

fn main() {
    forbidden("feature business main");
}
