fn main() {
    println!("cargo:rustc-check-cfg=cfg(nestrs_fixture_generated)");
    println!("cargo:rustc-cfg=nestrs_fixture_generated");
    println!("cargo:rustc-env=NESTRS_FIXTURE_LABEL=generated-by-build-script");
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(
        output.join("generated.rs"),
        "pub const GENERATED_COUNT: usize = 17;\n",
    )
    .unwrap();
}
