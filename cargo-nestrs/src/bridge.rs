//! Discovery and rustc argument injection for the tool-owned declaration bridge.
//!
//! Applications do not declare this crate in Cargo.toml. The CLI supplies the
//! same artifact to rustc, rustdoc and rust-analyzer's editor-only crate graph.

use std::{
    env,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

/// 覆盖私有声明桥接工件路径的环境变量；应用无需声明宏依赖。
pub const BRIDGE_ENV: &str = "NESTRS_MACRO_BRIDGE";

/// 已经定位并计算内容身份的工具私有过程宏工件。
#[derive(Clone, Debug)]
pub struct Bridge {
    /// 桥接动态库的规范路径，供 rustc、rustdoc 和编辑器共同使用。
    pub path: PathBuf,

    /// Content identity, rather than mtime, for Cargo's external wrapper cache.
    pub fingerprint: String,
}

impl Bridge {
    /// 按照当前宿主动态库命名规则生成桥接文件名。
    pub fn file_name() -> String {
        format!(
            "{}nestrs_tool_bridge{}",
            env::consts::DLL_PREFIX,
            env::consts::DLL_SUFFIX
        )
    }

    /// 从显式环境配置或 driver 同目录发现桥接，并计算缓存指纹。
    pub fn discover(driver: &Path) -> Result<Self, String> {
        Self::at(&Self::locate(driver)?)
    }

    /// Compiler children only need the validated path. The CLI fingerprints
    /// bytes once per invocation rather than once per dependency compilation.
    pub fn locate(driver: &Path) -> Result<PathBuf, String> {
        let path = env::var_os(BRIDGE_ENV)
            .map(PathBuf::from)
            .unwrap_or_else(|| driver.with_file_name(Self::file_name()));
        canonical_bridge(&path)
    }

    /// 确认指定路径为文件并流式计算内容指纹，不以 mtime 判断内容身份。
    pub fn at(path: &Path) -> Result<Self, String> {
        let path = canonical_bridge(path)?;
        let mut file = File::open(&path)
            .map_err(|error| format!("cannot read Nestrs declaration bridge: {error}"))?;
        // This is an artifact cache identity, not a cryptographic trust check.
        let mut hash = 0xcbf29ce484222325u64;
        let mut buffer = [0; 64 * 1024];
        loop {
            let length = file.read(&mut buffer).map_err(|error| {
                format!("cannot fingerprint Nestrs declaration bridge: {error}")
            })?;
            if length == 0 {
                break;
            }
            for byte in &buffer[..length] {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
        Ok(Self {
            path,
            fingerprint: format!("{hash:016x}"),
        })
    }
}

/// 将桥接路径解析为真实文件；缺失或目录路径均明确拒绝。
fn canonical_bridge(path: &Path) -> Result<PathBuf, String> {
    let path = path.canonicalize().map_err(|error| format!(
        "cannot find Nestrs declaration bridge {}: {error}; build/install the complete cargo nestrs toolchain or set {BRIDGE_ENV}",
        path.display(),
    ))?;
    if !path.is_file() {
        return Err(format!(
            "Nestrs declaration bridge is not a file: {}",
            path.display()
        ));
    }
    Ok(path)
}

/// 按 rustc extern 名称及其修饰符检查依赖，避免子串误匹配。
pub fn has_extern(args: &[String], name: &str) -> bool {
    extern_values(args).any(|value| {
        // rustc supports --extern modifiers such as priv:crate=path.
        let head = value.split('=').next().unwrap_or(value);
        head.rsplit(':').next() == Some(name)
    })
}

/// 兼容分隔和等号两种 --extern 参数形式，返回原始参数值。
fn extern_values(args: &[String]) -> impl Iterator<Item = &str> {
    args.iter().enumerate().filter_map(|(index, argument)| {
        if argument == "--extern" {
            args.get(index + 1).map(String::as_str)
        } else {
            argument.strip_prefix("--extern=")
        }
    })
}

/// Reserve `nestrs` for the tool bridge and avoid silently shadowing a real crate.
pub fn inject_extern(args: &mut Vec<String>, path: &Path) -> Result<(), String> {
    if has_extern(args, "nestrs") {
        return Err("the extern crate name `nestrs` is reserved for the cargo nestrs declaration bridge; rename the conflicting Cargo dependency".into());
    }
    let path = path
        .to_str()
        .ok_or("Nestrs declaration bridge path is not UTF-8")?;
    args.extend(["--extern".into(), format!("nestrs={path}")]);
    Ok(())
}

/// Upstream metadata records the bridge's real crate identity. Even a consumer
/// without a direct core dependency must be able to load it recursively; an
/// `--extern nestrs=...` alias on the producer alone is insufficient.
pub fn inject_dependency_search(args: &mut Vec<String>, bridge: &Path) -> Result<(), String> {
    let directory = bridge
        .parent()
        .and_then(Path::to_str)
        .ok_or("Nestrs declaration bridge needs a UTF-8 parent directory")?;
    args.extend(["-L".into(), format!("dependency={directory}")]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injection_preserves_all_user_arguments_and_is_lexically_exact() {
        let mut args = vec![
            "rustc".into(),
            "--extern=nestrs_core=/core.rlib".into(),
            "--cfg".into(),
            "feature=\"api\"".into(),
        ];
        let original = args.clone();
        assert!(has_extern(&args, "nestrs_core"));
        assert!(!has_extern(&args, "nestrs"));
        inject_extern(&mut args, Path::new("/tools with spaces/bridge.so")).unwrap();
        assert_eq!(&args[..original.len()], original.as_slice());
        assert_eq!(
            &args[original.len()..],
            ["--extern", "nestrs=/tools with spaces/bridge.so"]
        );
    }

    #[test]
    fn a_user_namespace_conflict_fails_before_mutating_arguments() {
        for argument in ["nestrs=/user.rlib", "priv:nestrs=/user.rlib", "nestrs"] {
            for mut args in [
                vec!["--extern".into(), argument.into()],
                vec![format!("--extern={argument}")],
            ] {
                let before = args.clone();
                assert!(
                    inject_extern(&mut args, Path::new("/bridge.so"))
                        .unwrap_err()
                        .contains("reserved")
                );
                assert_eq!(args, before);
            }
        }
    }

    #[test]
    fn metadata_consumers_get_a_search_directory_without_a_new_extern_dependency() {
        let mut args = vec![
            "rustc".into(),
            "--extern".into(),
            "producer=/output/libproducer.rmeta".into(),
        ];
        inject_dependency_search(&mut args, Path::new("/tools with spaces/bridge.so")).unwrap();
        assert!(!has_extern(&args, "nestrs_core"));
        assert!(!has_extern(&args, "nestrs"));
        assert_eq!(&args[3..], ["-L", "dependency=/tools with spaces"]);
    }

    #[test]
    fn content_changes_invalidate_the_bridge_identity() {
        let directory =
            std::env::temp_dir().join(format!("nestrs-bridge-identity-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(Bridge::file_name());
        std::fs::write(&path, b"before").unwrap();
        let first = Bridge::at(&path).unwrap();
        std::fs::write(&path, b"after!").unwrap();
        let second = Bridge::at(&path).unwrap();
        assert_eq!(first.path, second.path);
        assert_ne!(first.fingerprint, second.fingerprint);
        assert_eq!(second.fingerprint, Bridge::at(&path).unwrap().fingerprint);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
