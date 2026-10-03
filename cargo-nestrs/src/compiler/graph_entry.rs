//! 图导出入口的编译身份校验。
//!
//! graph 命令通过 Cargo check 取得静态计划。这里只匹配实际源码、package 和 binary，
//! 不修改 AST、不替换 main、不生成可执行诊断程序。

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

/// graph 命令选定入口的完整 Cargo 身份，防止同名依赖覆盖产物。
struct GraphTarget {
    /// 传给 rustc 的规范 crate 名称。
    crate_name: String,

    /// Cargo 原始 binary 名称，保留连字符等字符。
    binary: OsString,

    /// 入口 package 的规范 manifest 目录。
    manifest: PathBuf,

    /// 所选 binary 主源码的规范路径。
    source: PathBuf,
}

impl GraphTarget {
    /// 读取并规范化 CLI 指定的图入口身份；未请求 graph 时返回 None。
    fn from_environment() -> Result<Option<Self>, String> {
        let Some(crate_name) = std::env::var_os("NESTRS_GRAPH_TARGET") else {
            return Ok(None);
        };
        let required = |name| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    format!("cargo nestrs graph requires {name}; refusing an unverified entry")
                })
        };
        let canonical = |name| -> Result<PathBuf, String> {
            PathBuf::from(required(name)?)
                .canonicalize()
                .map_err(|error| format!("cannot identify graph {name}: {error}"))
        };
        Ok(Some(Self {
            crate_name: crate_name
                .into_string()
                .map_err(|_| "graph crate name must be UTF-8")?,
            binary: required("NESTRS_GRAPH_BINARY")?,
            manifest: canonical("NESTRS_GRAPH_MANIFEST")?,
            source: canonical("NESTRS_GRAPH_SOURCE")?,
        }))
    }

    /// 先匹配源码，再核对原始 binary 和 package 身份；矛盾身份返回错误。
    fn matches(
        &self,
        crate_name: &str,
        source: &Path,
        cargo_binary: Option<&OsStr>,
        cargo_manifest: Option<&OsStr>,
    ) -> Result<bool, String> {
        if source.canonicalize().ok().as_ref() != Some(&self.source) {
            return Ok(false);
        }
        let manifest = cargo_manifest.and_then(|path| Path::new(path).canonicalize().ok());
        let binary_crate = self.binary.to_str().map(|name| name.replace('-', "_"));
        if crate_name != self.crate_name
            || binary_crate.as_deref() != Some(crate_name)
            || cargo_binary != Some(self.binary.as_os_str())
            || manifest.as_ref() != Some(&self.manifest)
        {
            return Err("cargo nestrs graph 无法确认所选 binary 的名称和 package 身份".into());
        }
        Ok(true)
    }

    /// 以当前 Cargo 单元的环境身份执行完整入口匹配。
    fn matches_environment(&self, crate_name: &str, source: &Path) -> Result<bool, String> {
        self.matches(
            crate_name,
            source,
            std::env::var_os("CARGO_BIN_NAME").as_deref(),
            std::env::var_os("CARGO_MANIFEST_DIR").as_deref(),
        )
    }
}

/// sidecar 写入再次复用同一入口校验，不让同名依赖覆盖所选 binary 的计划。
pub fn matches_target(crate_name: &str, source: &Path) -> Result<bool, String> {
    GraphTarget::from_environment()?
        .map(|target| target.matches_environment(crate_name, source))
        .unwrap_or(Ok(false))
}

/// 同时匹配真实源码与 Cargo 的 binary/package 身份，同名依赖或 build script 不会
/// 被当成目标入口。输入文件不必以 .rs 结尾，因此比较规范路径而非猜测文件后缀。
pub fn matches_arguments(crate_name: &str, args: &[String]) -> Result<bool, String> {
    let Some(target) = GraphTarget::from_environment()? else {
        return Ok(false);
    };
    for arg in args.iter().skip(1) {
        let source = Path::new(arg);
        if source.canonicalize().ok().as_ref() == Some(&target.source) {
            return target.matches_environment(crate_name, source);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let directory = std::env::temp_dir().join(format!(
                "nestrs graph identity {} {}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir_all(directory.join("first")).unwrap();
            fs::create_dir_all(directory.join("second")).unwrap();
            for source in ["first/entry", "first/build.rs", "second/entry"] {
                fs::write(directory.join(source), "fn main() {}\n").unwrap();
            }
            Self(directory)
        }

        fn target(&self, binary: &str) -> GraphTarget {
            GraphTarget {
                crate_name: binary.replace('-', "_"),
                binary: binary.into(),
                manifest: self.0.join("first").canonicalize().unwrap(),
                source: self.0.join("first/entry").canonicalize().unwrap(),
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn graph_identity_requires_the_raw_binary_and_canonical_package() {
        let fixture = Fixture::new();
        let target = fixture.target("my-app");
        let source = fixture.0.join("first/../first/entry");
        let manifest = fixture.0.join("first/.");
        assert!(
            target
                .matches(
                    "my_app",
                    &source,
                    Some(OsStr::new("my-app")),
                    Some(manifest.as_os_str()),
                )
                .unwrap()
        );
        for binary in [None, Some(OsStr::new("my_app"))] {
            assert!(
                target
                    .matches("my_app", &source, binary, Some(manifest.as_os_str()))
                    .is_err()
            );
        }
        assert!(
            target
                .matches(
                    "different",
                    &source,
                    Some(OsStr::new("my-app")),
                    Some(manifest.as_os_str())
                )
                .is_err()
        );
    }

    #[test]
    fn inherited_binary_name_cannot_select_a_same_named_build_script() {
        let fixture = Fixture::new();
        let target = fixture.target("build-script-build");
        assert!(
            !target
                .matches(
                    "build_script_build",
                    &fixture.0.join("first/build.rs"),
                    Some(OsStr::new("build-script-build")),
                    Some(target.manifest.as_os_str()),
                )
                .unwrap()
        );
    }

    #[test]
    fn same_binary_name_in_another_package_never_selects_its_source() {
        let fixture = Fixture::new();
        let target = fixture.target("app");
        let other_manifest = fixture.0.join("second");
        assert!(
            !target
                .matches(
                    "app",
                    &fixture.0.join("second/entry"),
                    Some(OsStr::new("app")),
                    Some(other_manifest.as_os_str()),
                )
                .unwrap()
        );
        // 同一输入被错误声明为另一个 package 时，不得误读先前缓存的计划。
        assert!(
            target
                .matches(
                    "app",
                    &target.source,
                    Some(OsStr::new("app")),
                    Some(other_manifest.as_os_str()),
                )
                .is_err()
        );
    }
}
