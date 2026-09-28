#![cfg_attr(all(), no_main)]

#[unsafe(no_mangle)]
pub extern "C" fn main() -> i32 {
    let path = std::env::var_os("NESTRS_GRAPH_SENTINEL").expect("sentinel path");
    std::fs::write(path, "custom cfg no_main entry").unwrap();
    0
}
