//! 目标端反射入口与私有 adapter 由编译器认证。直接调用生产 driver，验证源码无法
//! 伪造声明、取得私有执行能力，且上游真实 adapter 在优化后仍然可达。
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
fn check_rejects_an_older_core_protocol_even_without_service_declarations() {
    fn copy_sources(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let destination = target.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_sources(&entry.path(), &destination);
            } else {
                fs::copy(entry.path(), destination).unwrap();
            }
        }
    }

    let directory = artifacts("old-core-protocol");
    fs::create_dir_all(directory.join("src")).unwrap();
    let core = directory.join("core");
    copy_sources(&workspace().join("nestrs-core/src"), &core.join("src"));
    fs::copy(
        workspace().join("nestrs-core/Cargo.toml"),
        core.join("Cargo.toml"),
    )
    .unwrap();
    // Model a mismatched installation using real core source and an older sink
    // name. No declarations are needed to load the core dependency in rustc.
    let plan_path = core.join("src/graph/plan.rs");
    let source = fs::read_to_string(&plan_path).unwrap();
    assert!(source.contains("plan_set_options_v2"));
    fs::write(
        &plan_path,
        source.replace("plan_set_options_v2", "plan_set_options"),
    )
    .unwrap();
    let original: toml::Value =
        toml::from_str(&fs::read_to_string(workspace().join("Cargo.toml")).unwrap()).unwrap();
    let mut manifest: toml::Value = toml::from_str(
        r#"
[package]
name = "old-core-protocol"
version = "0.0.0"
edition = "2024"
[dependencies]
nestrs-core = { path = "core" }
[workspace]
members = ["core"]
resolver = "3"
"#,
    )
    .unwrap();
    let workspace = manifest["workspace"].as_table_mut().unwrap();
    workspace.insert("package".into(), original["workspace"]["package"].clone());
    workspace.insert(
        "dependencies".into(),
        original["workspace"]["dependencies"].clone(),
    );
    fs::write(
        directory.join("Cargo.toml"),
        toml::to_string(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        directory.join("src/main.rs"),
        "fn main() { let _ = std::mem::size_of::<nestrs_core::ServiceProvider>(); }",
    )
    .unwrap();

    let output = run(&directory, &["check", "--offline"], "old-protocol");
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "check accepted an incompatible core"
    );
    assert!(diagnostic.contains("内部协议版本不匹配"), "{diagnostic}");
    assert!(diagnostic.contains("plan_set_options_v2"), "{diagnostic}");
    assert!(
        !diagnostic.contains("internal compiler error"),
        "{diagnostic}"
    );
}

#[test]
fn explicit_binding_satisfies_automatic_demand_without_a_duplicate_pair() {
    let directory = artifacts("explicit-binding-demand");
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            "[package]\nname = \"explicit-binding-demand\"\nversion = \"0.0.0\"\n\
             edition = \"2024\"\n[workspace]\n[dependencies]\n\
             nestrs-core = {{ path = {} }}\n\
             tokio = {{ version = \"1.53.1\", features = [\"rt\", \"macros\"] }}\n",
            quoted(&workspace().join("nestrs-core")),
        ),
    )
    .unwrap();
    fs::write(
        directory.join("src/main.rs"),
        r#"use nestrs::{bind, injectable};
use nestrs_core::ServiceProvider;
trait Port: Send + Sync { fn value(&self) -> u32; }
#[injectable]
struct Service { value: u32 }
#[bind]
impl Port for Service { fn value(&self) -> u32 { self.value } }
#[injectable]
struct Consumer { #[inject] port: dyn Port }
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = ServiceProvider::build().await.unwrap();
    let concrete = provider.get_required_service::<Service>().await.unwrap();
    let projected = provider.get_required_service::<dyn Port>().await.unwrap();
    let consumer = provider.get_required_service::<Consumer>().await.unwrap();
    assert!(std::ptr::addr_eq(concrete, projected));
    assert!(std::ptr::addr_eq(projected, &*consumer.port));
    assert_eq!(projected.value(), concrete.value);
    provider.dispose_async().await.unwrap();
    println!("explicit pair satisfies field and root automatic demand");
}
"#,
    )
    .unwrap();
    let output = run(&directory, &["run", "--offline"], "explicit-demand");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("explicit pair satisfies field and root automatic demand")
    );
}

#[test]
fn duplicate_explicit_bindings_are_rejected_before_a_final_program_can_run() {
    let directory = artifacts("duplicate-explicit-bindings");
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            "[package]\nname = \"duplicate-explicit-bindings\"\nversion = \"0.0.0\"\n\
             edition = \"2024\"\n[workspace]\n[dependencies]\n\
             nestrs-core = {{ path = {} }}\n",
            quoted(&workspace().join("nestrs-core")),
        ),
    )
    .unwrap();
    fs::write(
        directory.join("src/main.rs"),
        r#"use nestrs::{bind, injectable};
trait Port: Send + Sync {}
#[injectable]
struct Service;
#[bind]
#[bind]
impl Port for Service {}
fn main() { panic!("duplicate binding validation must not execute main"); }
"#,
    )
    .unwrap();
    for operation in ["check", "build"] {
        let output = run(&directory, &[operation, "--offline"], operation);
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "{operation} accepted duplicate bindings"
        );
        for expected in ["DuplicateBinding", "Service", "Port"] {
            assert!(diagnostic.contains(expected), "{operation}: {diagnostic}");
        }
        assert!(
            !diagnostic.contains("internal compiler error"),
            "{diagnostic}"
        );
    }
}

#[test]
fn closed_generic_blueprints_keep_demand_and_exact_factory_priority() {
    let directory = artifacts("closed-blueprint-priority");
    fs::create_dir_all(&directory).unwrap();
    let manifest = workspace().join("tools/compiler-probe/fixtures/auto-binding/Cargo.toml");
    // 逐一执行有效正例，不调用仍把非法图当作运行期错误的历史 --bins probe。
    // 这些 fixture 同时检查实际服务行为和编译器生成的需求/显式投影清单。
    for (binary, expected) in [
        (
            "factory_override",
            "default/named/indexed blueprints remain unused",
        ),
        (
            "factory_other_key",
            "generic default blueprint remains active",
        ),
        (
            "unreferenced_generic",
            "unused generic: no unmaterialized trait request",
        ),
        (
            "explicit_generic_root",
            "unqueried blueprint dependency bound",
        ),
    ] {
        let output = run(
            &directory,
            &[
                "run",
                "--offline",
                "--locked",
                "--manifest-path",
                manifest.to_str().unwrap(),
                "--bin",
                binary,
            ],
            binary,
        );
        assert!(
            output.status.success(),
            "{binary}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains(expected),
            "{binary}: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn source_cannot_call_reflection_entry_or_forge_declaration_callbacks() {
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
            "fn main() { __nestrs_reflect_v2(std::ptr::null_mut()); }",
        ),
        (
            "address",
            "fn main() { let _entry: fn(*mut ()) = __nestrs_reflect_v2; }",
        ),
        (
            "constant",
            "const ENTRY: fn(*mut ()) = __nestrs_reflect_v2; fn main() {}",
        ),
        (
            "alias",
            "use crate::__nestrs_reflect_v2 as hidden; fn main() {}",
        ),
        (
            "glob-dormant",
            "mod hidden { use super::*; fn dormant() { __nestrs_reflect_v2(std::ptr::null_mut()); } } fn main() {}",
        ),
    ];
    for (case, source) in cases {
        fs::write(directory.join("src/main.rs"), source).unwrap();
        let output = run(&directory, &["check", "--offline"], case);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{case}: source access compiled");
        assert!(
            stderr.contains("compiler-owned Nestrs reflection entry cannot be referenced"),
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
fn source_spelling_does_not_create_a_reflection_declaration() {
    let directory = artifacts("spoofed-local-reflection");
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            "[package]\nname = \"reflection-marker-source-audit\"\nversion = \"0.0.0\"\n\
             edition = \"2024\"\n[workspace]\n[dependencies]\n\
             nestrs-core = {{ path = {} }}\n\
             tokio = {{ version = \"1.53.1\", features = [\"rt\", \"macros\"] }}\n",
            quoted(&workspace().join("nestrs-core")),
        ),
    )
    .unwrap();
    // 精确模仿工具声明的模块、marker 名称及签名，仍然不能获得 bridge 宏来源。
    // 若收集器只匹配拼写，它会错误地把 Pretend 注册，或尝试从伪造蓝图读取 adapter。
    fs::write(
        directory.join("src/main.rs"),
        r#"#![forbid(unsafe_code)]
#![allow(dead_code)]
struct Pretend;
mod __nestrs_reflect {
    pub enum CompilerKey { Default, Named(&'static str), Indexed(usize) }
    pub const fn compiler_provider<T: ?Sized>(_: CompilerKey) {}
    pub const fn compiler_plan_provider<T: ?Sized, const L: u8, const P: bool, const I: u8>(_: CompilerKey) {}
    pub const fn compiler_query_root<T: ?Sized>() {}
    pub trait ProviderDefinition { fn provider(); }
    pub fn provider_definition<T: ProviderDefinition>() { T::provider() }
}
impl __nestrs_reflect::ProviderDefinition for Pretend { fn provider() {} }
fn ordinary_user_function() {
    __nestrs_reflect::compiler_provider::<Pretend>(__nestrs_reflect::CompilerKey::Default);
    __nestrs_reflect::compiler_plan_provider::<Pretend, 0, false, 0>(__nestrs_reflect::CompilerKey::Default);
    __nestrs_reflect::provider_definition::<Pretend>();
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    ordinary_user_function();
    let provider = nestrs_core::ServiceProvider::build().await.unwrap();
    assert!(provider.get_service::<Pretend>().await.unwrap().is_none());
    provider.dispose_async().await.unwrap();
    println!("user marker spelling did not register a service");
}
"#,
    )
    .unwrap();
    let output = run(&directory, &["run", "--offline"], "source-markers");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        String::from_utf8_lossy(&output.stdout)
            .contains("user marker spelling did not register a service"),
    );
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
    let answer = provider.get_required_service::<dyn upstream::Answer>().await.unwrap();
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
    "let _ = ::nestrs_core::graph::plan::CompiledApplication::load();".parse().unwrap()
}

#[proc_macro_attribute]
pub fn injectable(_: TokenStream, item: TokenStream) -> TokenStream {
    let mut output = item;
    output.extend("fn private_access() { let _ = ::nestrs_core::graph::plan::CompiledApplication::load(); }".parse::<TokenStream>().unwrap());
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
    assert_eq!(provider.get_required_service::<Service>().await.unwrap().value, 37);
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
        // The target must resolve: a missing symbol would only test spelling,
        // not whether the compiler rejects a different macro artifact's access.
        assert!(
            !stderr.contains("E0425"),
            "{name}: stale attack target: {stderr}"
        );
        assert!(stderr.contains("Nestrs 内部实现"), "{name}: {stderr}");
    }
}
