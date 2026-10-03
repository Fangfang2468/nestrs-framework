//! 只读工具链诊断。结构化输出复用正式工件发现及缓存规则，不推导项目配置。

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::cli::DoctorOptions;
use crate::toolchain::Toolchain;

/// JSON v1 的完整编译器身份；字段名与固定工具链配置保持一致。
#[derive(Serialize)]
struct CompilerReport<'a> {
    /// 已核对的 rustc release。
    release: &'a str,

    /// 已核对的完整 rustc commit。
    commit_hash: &'a str,

    /// CLI、driver 与编译器共同使用的本机宿主。
    host: &'a str,
}

/// 已验证工件的只读视图。未显式查询 target 时，三个目录字段均为 null。
#[derive(Serialize)]
struct DoctorReport<'a> {
    /// 诊断 JSON 格式版本，独立于编译器计划 ABI。
    version: u32,

    /// 当前选择的完整编译器身份。
    rustc: CompilerReport<'a>,

    /// 实际 rustc 可执行文件路径。
    compiler: &'a Path,

    /// 标准库和 rustc-dev 所在目录。
    sysroot: &'a Path,

    /// 已完成身份核对的 driver 路径。
    driver: &'a Path,

    /// 与 driver 配套并参与联合指纹的私有宏桥接路径。
    macro_bridge: &'a Path,

    /// 正式工具计算的 driver 与桥接工件联合内容指纹。
    fingerprint: &'a str,

    /// 仅在显式传入 --target-dir 时提供的绝对路径。
    target_directory: Option<PathBuf>,

    /// 由 Toolchain 的平台规则生成的隔离 Cargo 目录。
    cache_directory: Option<PathBuf>,

    /// 当前缓存内保存编译器分析产物的目录。
    compiler_output_directory: Option<PathBuf>,
}

/// 核对真实工具链后一次输出结果；失败只由命令入口写 stderr。
pub(super) fn run(options: DoctorOptions) -> Result<u8, String> {
    let toolchain = Toolchain::discover()?;
    if options.json {
        println!("{}", json_report(&toolchain, options.target_directory)?);
    } else {
        println!("Nestrs toolchain is ready");
        println!(
            "rustc: {} ({})",
            toolchain.identity.release, toolchain.identity.commit
        );
        println!("host: {}", toolchain.identity.host);
        println!("compiler: {}", toolchain.rustc.display());
        println!("sysroot: {}", toolchain.sysroot.display());
        println!("driver: {}", toolchain.driver.display());
        println!("macro bridge: {}", toolchain.bridge.display());
        println!("driver fingerprint: {}", toolchain.fingerprint);
    }
    Ok(0)
}

/// 将显式 target 解析为绝对路径；不调用 Cargo metadata 或要求该目录存在。
fn json_report(toolchain: &Toolchain, target: Option<PathBuf>) -> Result<String, String> {
    let target_directory = target
        .map(|path| {
            std::path::absolute(path)
                .map_err(|error| format!("cannot resolve target directory: {error}"))
        })
        .transpose()?;
    let cache_directory = target_directory
        .as_deref()
        .map(|target| toolchain.cache_directory(target));
    let compiler_output_directory = cache_directory
        .as_ref()
        .map(|cache| cache.join("nestrs").join("compiler"));
    let report = DoctorReport {
        version: 1,
        rustc: CompilerReport {
            release: &toolchain.identity.release,
            commit_hash: &toolchain.identity.commit,
            host: &toolchain.identity.host,
        },
        compiler: &toolchain.rustc,
        sysroot: &toolchain.sysroot,
        driver: &toolchain.driver,
        macro_bridge: &toolchain.bridge,
        fingerprint: &toolchain.fingerprint,
        target_directory,
        cache_directory,
        compiler_output_directory,
    };
    // Path serialization rejects paths that JSON cannot represent losslessly;
    // report the error instead of silently replacing OS path bytes.
    serde_json::to_string_pretty(&report)
        .map_err(|error| format!("cannot serialize toolchain report: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolchain::CompilerIdentity;

    fn toolchain(host: &str) -> Toolchain {
        let root = std::env::current_dir().unwrap().join("工具 with spaces");
        Toolchain {
            identity: CompilerIdentity {
                release: "test-release".into(),
                commit: "test-commit".into(),
                host: host.into(),
            },
            rustc: root.join("rustc"),
            sysroot: root.join("sysroot"),
            driver: root.join("driver"),
            bridge: root.join("bridge"),
            fingerprint: "driver-bridge-content".into(),
        }
    }

    #[test]
    fn report_preserves_identity_and_leaves_unspecified_project_directories_null() {
        let toolchain = toolchain("x86_64-unknown-linux-gnu");
        let report: serde_json::Value =
            serde_json::from_str(&json_report(&toolchain, None).unwrap()).unwrap();
        assert_eq!(report["version"], 1);
        assert_eq!(report["rustc"]["release"], toolchain.identity.release);
        assert_eq!(report["rustc"]["commit_hash"], toolchain.identity.commit);
        assert_eq!(report["rustc"]["host"], toolchain.identity.host);
        assert_eq!(report["compiler"], toolchain.rustc.to_str().unwrap());
        assert_eq!(report["sysroot"], toolchain.sysroot.to_str().unwrap());
        assert_eq!(report["driver"], toolchain.driver.to_str().unwrap());
        assert_eq!(report["macro_bridge"], toolchain.bridge.to_str().unwrap());
        assert_eq!(report["fingerprint"], toolchain.fingerprint);
        for field in [
            "target_directory",
            "cache_directory",
            "compiler_output_directory",
        ] {
            assert!(report.get(field).unwrap().is_null());
        }
    }

    #[test]
    fn explicit_target_uses_the_toolchains_native_cache_rule_without_creating_directories() {
        let target = PathBuf::from(format!("未创建 target {}", std::process::id()));
        let absolute = std::env::current_dir().unwrap().join(&target);
        assert!(!absolute.exists());
        for host in ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"] {
            let toolchain = toolchain(host);
            for path in [&target, &absolute] {
                let report: serde_json::Value =
                    serde_json::from_str(&json_report(&toolchain, Some(path.clone())).unwrap())
                        .unwrap();
                let cache = toolchain.cache_directory(&absolute);
                assert_eq!(report["target_directory"], absolute.to_str().unwrap());
                assert_eq!(report["cache_directory"], cache.to_str().unwrap());
                assert_eq!(
                    report["compiler_output_directory"],
                    cache.join("nestrs/compiler").to_str().unwrap()
                );
            }
        }
        assert!(!absolute.exists());
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_json_paths_fail_without_lossy_replacement() {
        use std::{ffi::OsString, os::unix::ffi::OsStringExt};

        let toolchain = toolchain("x86_64-unknown-linux-gnu");
        let target = PathBuf::from(OsString::from_vec(vec![b't', 0xff]));
        let error = json_report(&toolchain, Some(target)).unwrap_err();
        assert!(error.contains("cannot serialize toolchain report"));
    }
}
