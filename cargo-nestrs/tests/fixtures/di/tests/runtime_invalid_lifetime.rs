//! 错误注册只存在于子进程的 binary，不污染本测试或其他测试的静态依赖图。

use std::process::Command;

#[test]
fn singleton_to_scoped_registration_panics_before_construction() {
    let output = Command::new(env!("CARGO_BIN_EXE_invalid_lifetime"))
        .env_remove("RUST_BACKTRACE")
        .output()
        .expect("invalid_lifetime binary should start");
    let diagnostic = format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    assert_eq!(output.status.code(), Some(1), "{diagnostic}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("图验证已在服务实例化前拒绝依赖，构造次数 = 0"),
        "{diagnostic}",
    );
    assert!(stderr.contains("DI 依赖图验证失败"), "{diagnostic}");
    assert!(stderr.contains("ApplicationCache"), "{diagnostic}");
    assert!(stderr.contains("RequestSession"), "{diagnostic}");
    assert!(stderr.contains("invalid_lifetime.rs"), "{diagnostic}");
}
