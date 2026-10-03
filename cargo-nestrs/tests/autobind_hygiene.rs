//! Automatic binding overlays must not capture business constants or statics.
#![cfg(feature = "compiler-driver")]

use std::{
    fs,
    path::Path,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[test]
fn automatic_bindings_isolate_locals_without_losing_private_type_paths() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = workspace.join(format!(
        "target/autobind-hygiene/{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(directory.join("src")).unwrap();
    // Reuse the fixture's exact locked dependency set, including its package
    // identity, so this independent regression does not resolve new versions.
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            r#"[package]
name = "nestrs-di-regressions"
version = "0.0.0"
edition = "2024"
publish = false
[workspace]
[dependencies]
nestrs-core = {{ path = {} }}
tokio = {{ version = "1.53.1", default-features = false, features = ["rt-multi-thread", "sync", "macros", "time"] }}
serde_json = "1"
"#,
            serde_json::to_string(&workspace.join("nestrs-core")).unwrap(),
        ),
    )
    .unwrap();
    let lock = fs::read(workspace.join("cargo-nestrs/tests/fixtures/di/Cargo.lock")).unwrap();
    fs::write(directory.join("Cargo.lock"), &lock).unwrap();
    fs::write(directory.join("src/main.rs"), SOURCE).unwrap();

    for release in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_cargo-nestrs"));
        command
            .args(["run", "--locked", "--offline"])
            .current_dir(&directory)
            .env("CARGO_TARGET_DIR", directory.join("build"))
            .env("NESTRS_DRIVER", env!("CARGO_BIN_EXE_nestrs-driver"))
            .env_remove("RUSTC_BOOTSTRAP")
            .env_remove("RUSTC_WRAPPER")
            .env_remove("RUSTC_WORKSPACE_WRAPPER")
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS");
        if release {
            command.arg("--release");
        }
        let output = command.output().unwrap();
        let log = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        fs::write(
            directory.join(if release { "release.log" } else { "debug.log" }),
            &log,
        )
        .unwrap();
        assert!(output.status.success(), "release={release}\n{log}");
        assert!(log.contains("automatic binding identifier isolation passed"));
    }
    assert_eq!(fs::read(directory.join("Cargo.lock")).unwrap(), lock);
}

const SOURCE: &str = r#"#![allow(non_upper_case_globals, non_camel_case_types)]
use nestrs_core::ServiceProvider;

mod constants {
    use nestrs_core::ServiceProvider;
    const service: usize = 2;
    const projected: usize = 3;
    const slot: usize = 5;
    const input: usize = 7;
    const target: usize = 11;
    // The generated module is inside an anonymous const and must not collide
    // with this equally named business module or import its values.
    mod __nestrs_binding { pub const value: usize = 13; }

    trait r#type: Send + Sync { fn value(&self) -> usize; }
    use self::r#type as Port;
    macro_rules! declare {
        ($name:ident) => {
            #[nestrs::injectable]
            struct $name<T: Default + Send + Sync + 'static> { value: T }
        };
    }
    declare!(r#struct);
    impl<T: Default + Send + Sync + 'static> Port for r#struct<T> {
        fn value(&self) -> usize {
            let _ = &self.value;
            service + projected + slot + input + target + __nestrs_binding::value
        }
    }
    #[nestrs::injectable]
    struct Consumer { #[inject] port: dyn Port }

    pub async fn verify(root: &ServiceProvider) {
        let concrete = root.get_required_service::<self::r#struct<u8>>().await.unwrap();
        let bound = root.get_required_service::<dyn Port>().await.unwrap();
        let consumer = root.get_required_service::<Consumer>().await.unwrap();
        assert_eq!(bound.value(), 41);
        assert_eq!(consumer.port.value(), 41);
        assert!(std::ptr::addr_eq(concrete, bound));
        assert!(std::ptr::addr_eq(concrete, &*consumer.port));
    }
}

mod statics {
    trait Port: Send + Sync { fn value(&self) -> usize; }
    mod hidden {
        use nestrs_core::ServiceProvider;
        use super::Port as Interface;
        static service: usize = 17;
        static projected: usize = 19;
        static slot: usize = 23;
        static input: usize = 29;
        static target: usize = 31;
        #[nestrs::injectable]
        struct Service;
        impl super::Port for Service {
            fn value(&self) -> usize { service + projected + slot + input + target }
        }
        #[nestrs::injectable]
        struct Consumer { #[inject] port: dyn Interface }

        pub(super) async fn verify(root: &ServiceProvider) {
            let concrete = root.get_required_service::<Service>().await.unwrap();
            let bound = root.get_required_service::<dyn Interface>().await.unwrap();
            let consumer = root.get_required_service::<Consumer>().await.unwrap();
            assert_eq!(bound.value(), 119);
            assert_eq!(consumer.port.value(), 119);
            assert!(std::ptr::addr_eq(concrete, bound));
            assert!(std::ptr::addr_eq(concrete, &*consumer.port));
        }
    }
    pub async fn verify(root: &nestrs_core::ServiceProvider) { hidden::verify(root).await; }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let root = ServiceProvider::build().await.unwrap();
    constants::verify(&root).await;
    statics::verify(&root).await;
    root.dispose_async().await.unwrap();
    println!("automatic binding identifier isolation passed");
}
"#;
