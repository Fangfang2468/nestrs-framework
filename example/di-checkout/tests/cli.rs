//! 在独立进程中验证正常业务入口与初始化故障，避免环境变量影响其他业务测试。

use std::{
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nestrs-example-run-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn command(binary: &str) -> Command {
    let mut command = Command::new(binary);
    command.env_remove("NESTRS_EXAMPLE_FAIL_PAYMENT");
    command.env_remove("RUST_BACKTRACE");
    command.env_remove("CARGO_TARGET_DIR");
    command
}

fn transcript(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    )
}

#[test]
fn default_run_does_not_export_a_graph() {
    let directory = TestDirectory::new();
    let output = command(env!("CARGO_BIN_EXE_checkout"))
        .current_dir(&directory.0)
        .env("CARGO_TARGET_DIR", directory.0.join("target"))
        .output()
        .expect("checkout binary should start");
    let diagnostic = transcript(&output);
    assert!(output.status.success(), "{diagnostic}");
    assert!(!directory.0.join("target").exists(), "{diagnostic}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("[依赖图]"),
        "{diagnostic}",
    );
}

#[test]
fn initialization_failure_closes_root_and_exits_unsuccessfully() {
    let output = command(env!("CARGO_BIN_EXE_checkout"))
        .env("NESTRS_EXAMPLE_FAIL_PAYMENT", "1")
        .output()
        .expect("checkout binary should start");
    let diagnostic = transcript(&output);

    assert_eq!(output.status.code(), Some(1), "{diagnostic}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("示例执行失败"), "{diagnostic}");
    assert!(diagnostic.contains("初始化失败"), "{diagnostic}");
    assert!(stdout.contains("[关闭] root 已完成"), "{diagnostic}");
}
