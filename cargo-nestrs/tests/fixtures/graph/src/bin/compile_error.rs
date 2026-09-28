compile_error!("intentional project graph compile failure");

fn main() {
    let path = std::env::var_os("NESTRS_GRAPH_SENTINEL").expect("sentinel path");
    std::fs::write(path, "compile-error business main").unwrap();
}
