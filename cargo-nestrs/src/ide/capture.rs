//! 记录真实 rustc 编译单元，供成功的 Cargo 工件选择当前有效 IDE 输入。

use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    env,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};

/// 一次真实编译的 crate、配置、extern 和环境快照，保留 feature/test 变体身份。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct Unit {
    /// rustc 本次实际使用的 crate 名称。
    pub crate_name: String,

    /// 编译单元的根源码路径。
    pub root_module: PathBuf,

    /// 本次 rustc 选择的 Rust edition。
    pub edition: String,

    /// 本次生成的 crate 类型，供库和过程宏匹配使用。
    pub crate_types: Vec<String>,

    /// 同一编译器按真实参数输出的 cfg 集合，包含编译器派生条件。
    pub cfg: Vec<String>,

    /// 本次真实 extern 名称到工件路径的映射，保留 Cargo 重命名。
    pub externs: BTreeMap<String, PathBuf>,

    /// rustc --out-dir 指定的产物目录，不等同于 build.rs 的 OUT_DIR 环境变量。
    pub out_dir: PathBuf,

    /// Cargo 为当前编译配置选择的产物文件名后缀。
    pub extra_filename: String,

    /// Cargo 包信息和 include!/env! 所需的白名单环境值。
    pub env: BTreeMap<String, String>,

    /// 该编译单元所属 package 的目录。
    pub manifest_dir: PathBuf,

    /// 是否由 --test 构建测试入口。
    pub test: bool,

    /// 命令行显式指定的 target；未设置时由工具宿主决定。
    pub target: Option<String>,
}

/// Called before the driver dispatches either ordinary rustc or DI analysis.
/// Cargo's successful artifact stream later selects which records are current.
pub fn capture_rustc(args: &[String], source: &Path) -> Result<(), String> {
    let Some(directory) = env::var_os("NESTRS_IDE_CAPTURE") else {
        return Ok(());
    };
    let cwd = env::current_dir().map_err(|error| error.to_string())?;
    let environment = env::vars()
        .filter(|(name, _)| {
            name.starts_with("CARGO_PKG_")
                || matches!(
                    name.as_str(),
                    "CARGO_MANIFEST_DIR"
                        | "CARGO_MANIFEST_PATH"
                        | "CARGO_CRATE_NAME"
                        | "CARGO_BIN_NAME"
                        | "OUT_DIR"
                )
        })
        .collect();
    let Some(mut unit) = Unit::parse(args, source, &cwd, environment) else {
        // Version, target and sysroot queries do not represent compilation units.
        return Ok(());
    };
    // --cfg alone omits compiler-derived conditions such as debug_assertions,
    // panic strategy and target features. Ask the same compiler with the exact
    // unit arguments; --print cfg exits before compiling or expanding sources.
    let output = Command::new(&args[0])
        .args(&args[1..])
        .args(["--print", "cfg"])
        .output()
        .map_err(|error| {
            format!(
                "cannot inspect compiler cfg for {}: {error}",
                unit.crate_name
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "cannot inspect compiler cfg for {}: {}",
            unit.crate_name,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    unit.cfg = String::from_utf8(output.stdout)
        .map_err(|error| format!("compiler cfg is not UTF-8: {error}"))?
        .lines()
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    unit.cfg.sort();
    unit.cfg.dedup();
    let path = PathBuf::from(directory).join(format!("{:016x}.json", unit.identity()));
    super::write_atomic(
        &path,
        &serde_json::to_vec(&unit).map_err(|error| error.to_string())?,
    )
}

impl Unit {
    /// 解析具有 crate 名及输出目录的编译调用；工具查询不形成编译单元。
    pub(super) fn parse(
        args: &[String],
        source: &Path,
        cwd: &Path,
        environment: BTreeMap<String, String>,
    ) -> Option<Self> {
        let crate_name = values(args, "--crate-name").pop()?;
        let out_dir = absolute(cwd, values(args, "--out-dir").pop()?);
        let root_module = absolute(cwd, source);
        let manifest_dir = environment
            .get("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| cwd.to_path_buf());
        let externs = values(args, "--extern")
            .into_iter()
            .filter_map(|value| {
                let (name, file) = value.split_once('=')?;
                // rustc also accepts modifiers such as `priv:name=...`.
                let name = name.rsplit(':').next().unwrap_or(name).to_owned();
                Some((name, absolute(cwd, file)))
            })
            .collect();
        let extra_filename = values(args, "-C")
            .into_iter()
            .find_map(|value| value.strip_prefix("extra-filename=").map(str::to_owned))
            .unwrap_or_default();
        let test = args.iter().any(|arg| arg == "--test");
        let mut cfg = values(args, "--cfg");
        if test && !cfg.iter().any(|value| value == "test") {
            cfg.push("test".into());
        }
        cfg.sort();
        cfg.dedup();
        Some(Self {
            crate_name,
            root_module,
            manifest_dir,
            out_dir,
            extra_filename,
            externs,
            edition: values(args, "--edition")
                .pop()
                .unwrap_or_else(|| "2015".into()),
            crate_types: values(args, "--crate-type")
                .iter()
                .flat_map(|kind| kind.split(',').map(str::to_owned))
                .collect(),
            target: values(args, "--target").pop(),
            env: environment,
            cfg,
            test,
        })
    }

    /// 识别可加载的过程宏库，排除其测试可执行入口。
    pub fn is_proc_macro(&self) -> bool {
        !self.test && self.crate_types.iter().any(|kind| kind == "proc-macro")
    }

    /// 编译命令记录与语义 sidecar 共用相同身份；不能按 crate 名混合 feature/test 变体。
    pub(super) fn identity(&self) -> u64 {
        let mut identity = DefaultHasher::new();
        (
            &self.out_dir,
            &self.crate_name,
            &self.extra_filename,
            self.test,
            &self.crate_types,
        )
            .hash(&mut identity);
        identity.finish()
    }

    /// 按输出目录和编译后缀匹配 Cargo 工件，兼容 build-script 的重命名产物。
    pub fn owns_artifact(&self, path: &Path) -> bool {
        if path.parent() != Some(self.out_dir.as_path()) {
            return false;
        }
        let base = format!("{}{}", self.crate_name, self.extra_filename);
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            return false;
        };
        // Cargo exposes the renamed build-script executable in its JSON stream.
        // Its containing output directory still identifies the exact feature
        // variant, even when several units compile the same build.rs.
        if self.crate_name == "build_script_build"
            && matches!(name, "build-script-build" | "build-script-build.exe")
        {
            return true;
        }
        name == base
            || name == format!("{base}.exe")
            || ["rmeta", "rlib", "so", "dylib", "dll"]
                .iter()
                .any(|ext| name == format!("lib{base}.{ext}") || name == format!("{base}.{ext}"))
    }
}

/// 基于捕获时目录解析路径，尽可能取得规范身份并保留尚不存在的路径。
fn absolute(cwd: &Path, path: impl AsRef<Path>) -> PathBuf {
    let path = path.as_ref();
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    path.canonicalize().unwrap_or(path)
}

/// 收集指定 rustc 选项的各次取值，兼容等号及 -C 附着语法。
fn values(args: &[String], flag: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == flag {
            if let Some(value) = args.next() {
                values.push(value.clone());
            }
        } else if let Some(value) = arg.strip_prefix(&format!("{flag}=")) {
            values.push(value.to_owned());
        } else if flag == "-C"
            && let Some(value) = arg.strip_prefix("-C")
        {
            values.push(value.to_owned());
        }
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_real_cfg_aliases_and_test_context_without_running_code() {
        let args = [
            "rustc",
            "--crate-name",
            "app",
            "src/main.rs",
            "--out-dir",
            "/tmp/deps",
            "--edition=2024",
            "--test",
            "--cfg",
            "feature=\"audit\"",
            "--cfg=has_database",
            "--extern",
            "renamed=/tmp/liboriginal-ab.rmeta",
            "-Cextra-filename=-cd",
        ]
        .map(str::to_owned);
        let unit = Unit::parse(
            &args,
            Path::new("src/main.rs"),
            Path::new("/project"),
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(unit.root_module, Path::new("/project/src/main.rs"));
        assert_eq!(unit.cfg, ["feature=\"audit\"", "has_database", "test"]);
        assert_eq!(
            unit.externs["renamed"],
            Path::new("/tmp/liboriginal-ab.rmeta")
        );
        assert!(unit.owns_artifact(Path::new("/tmp/deps/app-cd.rmeta")));
        assert!(!unit.owns_artifact(Path::new("/tmp/deps/app-ef.rmeta")));
        assert!(
            Unit::parse(
                &["rustc".into(), "-vV".into()],
                Path::new("src/main.rs"),
                Path::new("/"),
                BTreeMap::new()
            )
            .is_none()
        );
    }

    #[test]
    fn cargo_build_script_rename_preserves_distinct_feature_units() {
        let args = [
            "rustc",
            "--crate-name",
            "build_script_build",
            "build.rs",
            "--out-dir",
            "/target/debug/build/package-aaa",
            "-Cextra-filename=-aaa",
        ]
        .map(str::to_owned);
        let first = Unit::parse(
            &args,
            Path::new("build.rs"),
            Path::new("/project"),
            BTreeMap::new(),
        )
        .unwrap();
        let mut second = first.clone();
        second.out_dir = "/target/debug/build/package-bbb".into();
        second.extra_filename = "-bbb".into();
        second.cfg = vec!["feature=\"default\"".into()];
        let artifact = Path::new("/target/debug/build/package-bbb/build-script-build");
        assert!(!first.owns_artifact(artifact));
        assert!(second.owns_artifact(artifact));
    }
}
