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
fn sample_processes_orders_without_exporting_a_graph_or_running_di_probes() {
    let directory = TestDirectory::new();
    let output = command(env!("CARGO_BIN_EXE_checkout"))
        .arg("sample")
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
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("成功订单 2 笔；剩余库存 2 件；成交金额 597.00 元；审计 4 条"),
        "{diagnostic}"
    );
    assert!(!stdout.contains("[验证]"), "{diagnostic}");
    assert_eq!(stdout.matches("[审计]").count(), 4, "{diagnostic}");
}

#[test]
fn initialization_failure_closes_root_and_exits_unsuccessfully() {
    let output = command(env!("CARGO_BIN_EXE_checkout"))
        .arg("sample")
        .env("NESTRS_EXAMPLE_FAIL_PAYMENT", "1")
        .output()
        .expect("checkout binary should start");
    let diagnostic = transcript(&output);

    assert_eq!(output.status.code(), Some(1), "{diagnostic}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("结账应用执行失败"), "{diagnostic}");
    assert!(diagnostic.contains("初始化失败"), "{diagnostic}");
    assert!(stdout.contains("[关闭] root 已完成"), "{diagnostic}");
}

#[test]
fn help_does_not_initialize_services() {
    for args in [vec![], vec!["--help"], vec!["place-order", "--help"]] {
        let output = command(env!("CARGO_BIN_EXE_checkout"))
            .args(args)
            .output()
            .unwrap();
        let diagnostic = transcript(&output);
        assert!(output.status.success(), "{diagnostic}");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("place-order"), "{diagnostic}");
        assert!(!stdout.contains("[启动]"), "{diagnostic}");
        assert!(!stdout.contains("[构造]"), "{diagnostic}");
    }
}

#[test]
fn a_customer_can_submit_an_order_and_receives_a_real_receipt() {
    let output = command(env!("CARGO_BIN_EXE_checkout"))
        .args([
            "place-order",
            "--customer",
            "周顾客",
            "--sku",
            "KEYBOARD",
            "--quantity",
            "2",
            "--payment",
            "wallet",
            "--payment-token",
            "opaque-local-token",
        ])
        .output()
        .unwrap();
    let diagnostic = transcript(&output);
    assert!(output.status.success(), "{diagnostic}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[订单] 订单 ORD-"), "{diagnostic}");
    assert!(stdout.contains("客户 周顾客"), "{diagnostic}");
    assert!(stdout.contains("支付 wallet-"), "{diagnostic}");
    assert!(
        stdout.contains("成功订单 1 笔；剩余库存 3 件；成交金额 398.00 元；审计 1 条"),
        "{diagnostic}"
    );
    assert!(!stdout.contains("opaque-local-token"), "{diagnostic}");
    assert!(!stdout.contains("Alice"), "{diagnostic}");
    assert!(stdout.contains("[关闭] root 已完成"), "{diagnostic}");
}

#[test]
fn business_rejection_is_audited_rolls_back_stock_and_exits_unsuccessfully() {
    let output = command(env!("CARGO_BIN_EXE_checkout"))
        .args([
            "place-order",
            "--customer",
            "DeclinedCustomer",
            "--payment-token",
            "declined",
        ])
        .output()
        .unwrap();
    let diagnostic = transcript(&output);
    assert_eq!(output.status.code(), Some(1), "{diagnostic}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("[拒绝] 客户 DeclinedCustomer"),
        "{diagnostic}"
    );
    assert!(stdout.contains("业务库存回滚"), "{diagnostic}");
    assert!(
        stdout.contains("成功订单 0 笔；剩余库存 5 件；成交金额 0.00 元；审计 1 条"),
        "{diagnostic}"
    );
    assert!(stdout.contains("[关闭] root 已完成"), "{diagnostic}");
}

#[test]
fn eager_scope_warmup_and_serial_activation_preserve_sample_results() {
    let output = command(env!("CARGO_BIN_EXE_checkout"))
        .args([
            "--eager",
            "sample",
            "--warm-up-scopes",
            "--max-concurrency",
            "1",
        ])
        .output()
        .unwrap();
    let diagnostic = transcript(&output);
    assert!(output.status.success(), "{diagnostic}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Eager；构造并发上限 1；scope 预热 true"),
        "{diagnostic}"
    );
    assert!(
        stdout.contains("成功订单 2 笔；剩余库存 2 件；成交金额 597.00 元；审计 4 条"),
        "{diagnostic}"
    );
}

#[test]
fn invalid_arguments_fail_before_building_the_container() {
    for args in [
        vec!["place-order"],
        vec!["sample", "--max-concurrency", "0"],
        vec!["place-order", "--customer", "Alice", "--payment", "cash"],
    ] {
        let output = command(env!("CARGO_BIN_EXE_checkout"))
            .args(args)
            .output()
            .unwrap();
        let diagnostic = transcript(&output);
        assert_eq!(output.status.code(), Some(2), "{diagnostic}");
        assert!(
            !String::from_utf8_lossy(&output.stdout).contains("[构造]"),
            "{diagnostic}"
        );
    }
}
