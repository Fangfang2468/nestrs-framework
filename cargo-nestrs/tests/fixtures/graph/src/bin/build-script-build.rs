use nestrs::injectable;

#[injectable]
struct BuildScriptNamedService;

fn main() {
    let path = std::env::var_os("NESTRS_GRAPH_SENTINEL").expect("sentinel path");
    std::fs::write(path, "build-script-build binary business main").unwrap();
    panic!("graph must not execute business main");
}
