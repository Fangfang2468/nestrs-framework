fn main() {
    let path = std::env::var_os("NESTRS_GRAPH_SENTINEL").expect("sentinel path");
    std::fs::write(path, "business main without core").unwrap();
    panic!("a graph command must never run an unmodified application");
}
