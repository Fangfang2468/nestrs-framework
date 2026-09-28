use nestrs::injectable;

#[injectable]
struct OtherPackageOnly;

fn main() {
    let path = std::env::var_os("NESTRS_GRAPH_SENTINEL").expect("sentinel path");
    std::fs::write(path, "other package business main").unwrap();
    panic!("graph must not execute business main");
}
