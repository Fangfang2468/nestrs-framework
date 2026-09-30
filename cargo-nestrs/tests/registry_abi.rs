//! The executable registry is compiler-owned, including its private upstream
//! callbacks. Exercise the production driver rather than a MIR-only mock.
#![cfg(feature = "compiler-driver")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn quoted(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).unwrap()
}

fn workspace() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()
}

fn artifacts(test: &str) -> PathBuf {
    workspace()
        .join("target/registry-abi")
        .join(std::process::id().to_string())
        .join(test)
}

fn run(directory: &Path, arguments: &[&str], log_name: &str) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"))
        .args(arguments)
        .current_dir(directory)
        .env("CARGO_TARGET_DIR", directory.join("build"))
        .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .env_remove("RUSTC_BOOTSTRAP")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .output()
        .unwrap();
    fs::write(
        directory.join(format!("{log_name}.log")),
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ),
    )
    .unwrap();
    output
}

#[test]
fn source_cannot_call_registry_entry_or_forge_declaration_callbacks() {
    let directory = artifacts("source-audit");
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            "[package]\nname = \"registry-source-audit\"\nversion = \"0.0.0\"\n\
             edition = \"2024\"\n[workspace]\n[dependencies]\n\
             nestrs-core = {{ path = {} }}\n",
            quoted(&workspace().join("nestrs-core")),
        ),
    )
    .unwrap();
    let cases = [
        (
            "direct",
            "fn main() { __nestrs_registry_v1(std::ptr::null_mut()); }",
        ),
        (
            "address",
            "fn main() { let _entry: fn(*mut ()) = __nestrs_registry_v1; }",
        ),
        (
            "constant",
            "const ENTRY: fn(*mut ()) = __nestrs_registry_v1; fn main() {}",
        ),
        (
            "alias",
            "use crate::__nestrs_registry_v1 as hidden; fn main() {}",
        ),
        (
            "glob-dormant",
            "mod hidden { use super::*; fn dormant() { __nestrs_registry_v1(std::ptr::null_mut()); } } fn main() {}",
        ),
    ];
    for (case, source) in cases {
        fs::write(directory.join("src/main.rs"), source).unwrap();
        let output = run(&directory, &["check", "--offline"], case);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{case}: source access compiled");
        assert!(
            stderr.contains("compiler-owned Nestrs registry entry cannot be referenced"),
            "{case}: unexpected diagnostic: {stderr}",
        );
    }
    for callback in [
        "__nestrs_reflect_provider",
        "__nestrs_reflected_factory",
        "__nestrs_reflect_trait_binding",
        "__nestrs_query_root",
        "__nestrs_reflect_automatic_binding",
        "__nestrs_reflect_blueprint",
        "__nestrs_reflect_blueprint_path",
    ] {
        fs::write(
            directory.join("src/main.rs"),
            format!("fn {callback}() {{}} fn main() {{}}"),
        )
        .unwrap();
        let output = run(&directory, &["check", "--offline"], callback);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "{callback}: forged callback compiled"
        );
        assert!(
            stderr.contains("callback names are reserved for authenticated declarations"),
            "{callback}: unexpected diagnostic: {stderr}",
        );
    }
}

#[test]
fn compiler_access_keeps_public_runtime_types_private_fields_inaccessible() {
    let directory = artifacts("private-runtime-fields");
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            "[package]\nname = \"registry-private-fields\"\nversion = \"0.0.0\"\n\
             edition = \"2024\"\n[workspace]\n[dependencies]\n\
             nestrs-core = {{ path = {} }}\n",
            quoted(&workspace().join("nestrs-core")),
        ),
    )
    .unwrap();
    // Field privacy comes from rustc's FieldDef metadata, independently of the
    // temporary resolver visibility used by generated adapters. In particular,
    // changing an Injection's raw pointer through safe Rust must remain illegal.
    let cases = [
        (
            "injection-read",
            "fn f(x: nestrs_core::Injection<u64>) { let _ = x.ptr; }",
        ),
        (
            "injection-write",
            "fn f(mut x: nestrs_core::Injection<u64>) { x.ptr = std::ptr::NonNull::dangling(); }",
        ),
        (
            "injection-borrow",
            "fn f(mut x: nestrs_core::Injection<u64>) { let _ = &mut x.ptr; }",
        ),
        (
            "injection-pattern",
            "fn f(x: nestrs_core::Injection<u64>) { let nestrs_core::Injection { ptr, .. } = x; let _ = ptr; }",
        ),
        (
            "injection-update",
            "fn f(x: nestrs_core::Injection<u64>) { let _ = nestrs_core::Injection { ptr: std::ptr::NonNull::dangling(), ..x }; }",
        ),
        (
            "injection-construction",
            "fn f() { let _ = nestrs_core::Injection::<u64> { ptr: std::ptr::NonNull::dangling(), _lease: panic!() }; }",
        ),
        (
            "provider-read",
            "fn f(x: nestrs_core::ServiceProvider) { let _ = x.owner; }",
        ),
        (
            "provider-pattern",
            "fn f(x: nestrs_core::ServiceProvider) { let nestrs_core::ServiceProvider { ref owner, .. } = x; let _ = owner; }",
        ),
        (
            "dispose-error-read",
            "fn f(x: nestrs_core::DisposeError) { let _ = x.failures; }",
        ),
    ];
    for (case, source) in cases {
        fs::write(
            directory.join("src/main.rs"),
            format!("#![forbid(unsafe_code)]\n{source}\nfn main() {{}}\n"),
        )
        .unwrap();
        let output = run(&directory, &["check", "--offline"], case);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{case}: private field compiled");
        assert!(
            stderr.contains("E0616") || stderr.contains("E0451"),
            "{case}: expected rustc field privacy diagnostic: {stderr}",
        );
    }

    fs::write(
        directory.join("src/main.rs"),
        r#"#![forbid(unsafe_code)]
fn main() {
    let mut options = nestrs_core::ServiceProviderOptions::default();
    options.initialization = nestrs_core::InitializationMode::Eager;
    options.max_concurrent_activations = std::num::NonZeroUsize::new(3).unwrap();
    let nestrs_core::ServiceProviderOptions { initialization, max_concurrent_activations } = options;
    assert_eq!(initialization, nestrs_core::InitializationMode::Eager);
    assert_eq!(max_concurrent_activations.get(), 3);
}
"#,
    )
    .unwrap();
    let output = run(&directory, &["run", "--offline"], "public-fields");
    assert!(
        output.status.success(),
        "public options must retain ordinary field access: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn optimized_registry_keeps_private_upstream_declarations_without_source_unsafe() {
    let directory = artifacts("private-upstream-lto");
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::create_dir_all(directory.join("upstream/src")).unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            r#"[package]
name = "registry-private-upstream"
version = "0.0.0"
edition = "2024"
[workspace]
members = ["upstream"]
[dependencies]
nestrs-core = {{ path = {} }}
upstream = {{ path = "upstream" }}
tokio = {{ version = "1.53.1", features = ["rt", "macros"] }}
[profile.release]
lto = "fat"
codegen-units = 1
"#,
            quoted(&workspace().join("nestrs-core")),
        ),
    )
    .unwrap();
    fs::write(
        directory.join("upstream/Cargo.toml"),
        format!(
            "[package]\nname = \"upstream\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
             [dependencies]\nnestrs-core = {{ path = {} }}\n",
            quoted(&workspace().join("nestrs-core")),
        ),
    )
    .unwrap();
    fs::write(
        directory.join("upstream/src/lib.rs"),
        r#"#![forbid(unsafe_code)]
pub trait Answer: Send + Sync { fn value(&self) -> u32; }
#[nestrs::injectable(lifetime = Singleton)]
struct PrivateAnswer;
impl Answer for PrivateAnswer { fn value(&self) -> u32 { 73 } }
"#,
    )
    .unwrap();
    fs::write(
        directory.join("src/main.rs"),
        r#"#![forbid(unsafe_code)]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = nestrs_core::ServiceProvider::build().await.unwrap();
    let answer = nestrs_core::get_required_service!(provider, dyn upstream::Answer).await.unwrap();
    assert_eq!(answer.value(), 73);
    provider.dispose_async().await.unwrap();
    println!("private upstream registry survived LTO");
}
"#,
    )
    .unwrap();
    let output = run(&directory, &["run", "--offline", "--release"], "release");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("private upstream registry survived LTO"),
    );
}

#[test]
fn same_named_dependency_is_not_the_cli_selected_declaration_bridge() {
    let directory = artifacts("bridge-artifact-identity");
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::create_dir_all(directory.join("other-bridge/src")).unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            r#"[package]
name = "registry-bridge-identity"
version = "0.0.0"
edition = "2024"
[workspace]
members = ["other-bridge"]
[dependencies]
nestrs-core = {{ path = {} }}
other = {{ package = "nestrs-tool-bridge", path = "other-bridge" }}
tokio = {{ version = "1.53.1", features = ["rt", "macros"] }}
"#,
            quoted(&workspace().join("nestrs-core")),
        ),
    )
    .unwrap();
    fs::write(
        directory.join("other-bridge/Cargo.toml"),
        "[package]\nname = \"nestrs-tool-bridge\"\nversion = \"0.0.0\"\n\
         edition = \"2024\"\n[lib]\nproc-macro = true\n",
    )
    .unwrap();
    fs::write(
        directory.join("other-bridge/src/lib.rs"),
        r#"extern crate proc_macro;
use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn keep(_: TokenStream, item: TokenStream) -> TokenStream { item }

#[proc_macro]
pub fn collect(_: TokenStream) -> TokenStream {
    "let _ = ::nestrs_core::registration::catalog::collect();".parse().unwrap()
}

#[proc_macro_attribute]
pub fn injectable(_: TokenStream, item: TokenStream) -> TokenStream {
    let mut output = item;
    output.extend("fn private_access() { let _ = ::nestrs_core::registration::catalog::collect(); }".parse::<TokenStream>().unwrap());
    output
}
"#,
    )
    .unwrap();
    fs::write(
        directory.join("src/main.rs"),
        r#"#![forbid(unsafe_code)]
#[other::keep]
#[nestrs::injectable]
struct Service { #[value(37)] value: usize }

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = nestrs_core::ServiceProvider::build().await.unwrap();
    assert_eq!(nestrs_core::get_required_service!(provider, Service).await.unwrap().value, 37);
    provider.dispose_async().await.unwrap();
    println!("CLI bridge retains its identity beside a same-named dependency");
}
"#,
    )
    .unwrap();
    let output = run(&directory, &["run", "--offline"], "official-bridge");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("CLI bridge retains its identity beside a same-named dependency")
    );

    for (name, source) in [
        (
            "function-like",
            "#![forbid(unsafe_code)] fn main() { other::collect!(); }",
        ),
        (
            "attribute",
            "#![forbid(unsafe_code)] #[other::injectable] struct Service; fn main() {}",
        ),
    ] {
        fs::write(directory.join("src/main.rs"), source).unwrap();
        let output = run(&directory, &["check", "--offline"], name);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "{name}: a different bridge artifact received compiler-internal access"
        );
        assert!(stderr.contains("Nestrs 内部实现"), "{name}: {stderr}");
    }
}
