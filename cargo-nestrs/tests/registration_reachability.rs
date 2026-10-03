//! A downstream registry must retain upstream private state, including open
//! generic blueprints whose first concrete use occurs in the executable.
#![cfg(feature = "compiler-driver")]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn quoted(path: &Path) -> String {
    serde_json::to_string(&path.to_string_lossy()).unwrap()
}

fn run(directory: &Path, arguments: &[&str], name: &str) -> Output {
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
        directory.join(format!("{name}.log")),
        format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        ),
    )
    .unwrap();
    output
}

fn project() -> PathBuf {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let directory = workspace
        .join("target/registration-reachability")
        .join(std::process::id().to_string());
    fs::create_dir_all(directory.join("src")).unwrap();
    fs::create_dir_all(directory.join("upstream/src")).unwrap();
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            r#"[package]
name = "registration-reachability"
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
            quoted(&workspace.join("nestrs-core")),
        ),
    )
    .unwrap();
    fs::write(
        directory.join("upstream/Cargo.toml"),
        format!(
            "[package]\nname = \"upstream\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
             [dependencies]\nnestrs-core = {{ path = {} }}\n\
             tokio = {{ version = \"1.53.1\", features = [\"rt\"] }}\n",
            quoted(&workspace.join("nestrs-core")),
        ),
    )
    .unwrap();
    fs::write(
        directory.join("upstream/src/lib.rs"),
        r#"#![forbid(unsafe_code)]
use std::{cell::Cell, marker::PhantomData, sync::atomic::{AtomicUsize, Ordering}};

// None of this state has a public accessor that could accidentally keep it
// reachable independently of compiler registration.
static FACTORY_NEXT: AtomicUsize = AtomicUsize::new(10);
static GENERIC_NEXT: AtomicUsize = AtomicUsize::new(20);
thread_local! {
    static FACTORY_TLS: Cell<usize> = const { Cell::new(40) };
    static GENERIC_TLS: Cell<usize> = const { Cell::new(30) };
}

#[inline(never)]
fn private_identity(value: usize) -> usize { value }

#[inline(always)]
fn factory_value() -> (usize, usize) {
    (private_identity(FACTORY_NEXT.fetch_add(1, Ordering::SeqCst)), FACTORY_TLS.with(|state| {
        let previous = state.get();
        state.set(previous + 1);
        previous
    }))
}

const fn factory_callback() -> fn() -> (usize, usize) { factory_value }
const FACTORY_CALLBACK: fn() -> (usize, usize) = factory_callback();

pub trait Record: Send + Sync { fn serial(&self) -> (usize, usize); }
struct PrivateRecord((usize, usize));
impl Record for PrivateRecord { fn serial(&self) -> (usize, usize) { self.0 } }

#[nestrs::factory(lifetime = Transient)]
async fn make_record() -> PrivateRecord {
    tokio::task::yield_now().await;
    PrivateRecord(FACTORY_CALLBACK())
}

#[inline(always)]
fn generic_value<T>() -> (usize, usize, usize) {
    (GENERIC_NEXT.fetch_add(1, Ordering::SeqCst), GENERIC_TLS.with(|state| {
        let previous = state.get();
        state.set(previous + 1);
        previous
    }), std::mem::size_of::<T>())
}

// No closed Repository<T> occurs in this library. The downstream queries are
// the first closed roots and must still retain this blueprint's private state.
#[nestrs::injectable(lifetime = Transient)]
pub struct Repository<T: Send + Sync + 'static> {
    marker: PhantomData<T>,
    #[value(generic_value::<T>())]
    serial: (usize, usize, usize),
}
impl<T: Send + Sync + 'static> Repository<T> {
    pub fn serial(&self) -> (usize, usize, usize) { self.serial }
}
"#,
    )
    .unwrap();
    directory
}

#[test]
fn private_state_and_first_downstream_generic_roots_survive_codegen_without_becoming_public() {
    let directory = project();
    fs::write(
        directory.join("src/main.rs"),
        r#"#![forbid(unsafe_code)]
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let provider = nestrs_core::ServiceProvider::build().await.unwrap();
    let first = provider.get_required_service::<dyn upstream::Record>().await.unwrap();
    let second = provider.get_required_service::<dyn upstream::Record>().await.unwrap();
    assert_eq!(first.serial(), (10, 40));
    assert_eq!(second.serial(), (11, 41));
    let byte = provider.get_required_service::<upstream::Repository<u8>>().await.unwrap();
    let word = provider.get_required_service::<upstream::Repository<u16>>().await.unwrap();
    assert_eq!(byte.serial(), (20, 30, 1));
    assert_eq!(word.serial(), (21, 31, 2));
    provider.dispose_async().await.unwrap();
    println!("private state retained through callbacks and first downstream generic roots");
}
"#,
    )
    .unwrap();
    for (name, arguments) in [
        ("debug", vec!["run", "--offline"]),
        ("fat-lto", vec!["run", "--offline", "--release"]),
    ] {
        let output = run(&directory, &arguments, name);
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains(
            "private state retained through callbacks and first downstream generic roots"
        ));
    }
    for (name, path) in [
        ("static", "upstream::FACTORY_NEXT"),
        ("tls", "upstream::GENERIC_TLS"),
        ("helper", "upstream::private_identity"),
        ("factory", "upstream::make_record"),
    ] {
        fs::write(
            directory.join("src/main.rs"),
            format!("fn main() {{ let _ = {path}; }}"),
        )
        .unwrap();
        let output = run(&directory, &["check", "--offline"], name);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "{name}: private item exposed");
        assert!(stderr.contains("is private"), "{name}: {stderr}");
    }
}
