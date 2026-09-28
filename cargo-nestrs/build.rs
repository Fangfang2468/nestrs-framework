use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=toolchain.json");
    // A multi-host manifest does not make a driver portable across hosts. The
    // executable must advertise and validate the target it was compiled for.
    let target = std::env::var("TARGET").expect("Cargo build target");
    println!("cargo:rustc-env=NESTRS_BUILD_HOST={target}");
    if std::env::var_os("CARGO_FEATURE_COMPILER_DRIVER").is_none() {
        return;
    }
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(rustc)
        .args(["--print", "sysroot"])
        .output()
        .expect("query compiler sysroot");
    assert!(output.status.success(), "cannot query compiler sysroot");
    let sysroot = String::from_utf8(output.stdout).expect("UTF-8 sysroot");
    let libraries = std::path::Path::new(sysroot.trim()).join("lib");
    println!("cargo:rustc-link-search=native={}", libraries.display());
    println!(
        "cargo:rustc-link-search=native={}",
        libraries
            .join("rustlib")
            .join(&target)
            .join("lib")
            .display()
    );
    // MSVC resolves import libraries here, and Windows loads the matching DLLs
    // through PATH at invocation time. Unix linker arguments are invalid there.
    if std::env::var("CARGO_CFG_TARGET_FAMILY").as_deref() == Ok("unix") {
        println!(
            "cargo:rustc-link-arg-bin=nestrs-driver=-Wl,-rpath,{}",
            libraries.display()
        );
    }
}
