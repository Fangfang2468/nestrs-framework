fn main() {
    println!("cargo:rustc-check-cfg=cfg(nestrs_graph_build_script)");
    println!("cargo:rustc-cfg=nestrs_graph_build_script");
}
