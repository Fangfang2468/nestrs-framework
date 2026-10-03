//! Version-pinned Nestrs compiler driver for semantic automatic binding.
//!
//! Cargo invokes this as a RUSTC_WRAPPER. Semantic analysis discovers
//! pairs; a fresh compiler invocation checks generated ordinary Rust through a
//! virtual source overlay. No application source file is rewritten.

#![feature(rustc_private)]

extern crate rustc_ast;
extern crate rustc_const_eval;
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_parse;
extern crate rustc_session;
extern crate rustc_span;

#[path = "../protocol.rs"]
mod protocol;

#[path = "../compiler/arguments.rs"]
mod arguments;

#[path = "../compiler/autobind_codegen.rs"]
mod autobind_codegen;

#[path = "../compiler/autobind_semantic.rs"]
mod autobind_semantic;

#[path = "../compiler/constructor.rs"]
mod constructor;

#[path = "../compiler/di_plan/mod.rs"]
mod di_plan;

#[path = "../compiler/diagnostics.rs"]
mod diagnostics;

#[path = "../compiler/documentation.rs"]
mod documentation;

#[path = "../compiler/graph_entry.rs"]
mod graph_entry;

#[path = "../compiler/internal_access.rs"]
mod internal_access;

#[path = "../compiler/query_roots.rs"]
mod query_roots;

#[path = "../compiler/reflection.rs"]
mod reflection;

#[path = "../compiler/registration_codegen.rs"]
mod registration_codegen;

#[path = "../compiler/registration_reachability.rs"]
mod registration_reachability;

#[path = "../compiler/type_source.rs"]
mod type_source;

use autobind_codegen::OverlayFileLoader;
use autobind_semantic::Analysis;
use cargo_nestrs::bridge::{Bridge, has_extern, inject_dependency_search, inject_extern};
use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface;
use rustc_middle::ty::TyCtxt;
use rustc_span::source_map::{FileLoader, RealFileLoader};
use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    fmt::Write as _,
    fs,
    hash::{Hash, Hasher},
    io,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    sync::{Arc, Mutex},
};

/// 第一轮真实读取的规范源码快照，两轮编译共享并校验其内容。
type Sources = Arc<Mutex<BTreeMap<PathBuf, String>>>;

/// 记录第一轮首次读取内容的文件加载器，重复读取时拒绝源码变化。
struct SnapshotLoader(Sources);

impl FileLoader for SnapshotLoader {
    /// 按文档来源映射查询真实文件是否存在。
    fn file_exists(&self, path: &Path) -> bool {
        RealFileLoader.file_exists(&documentation::remap_source_path(path))
    }

    /// 读取并固定首个源码快照，后续读取发现变化时拒绝继续。
    fn read_file(&self, path: &Path) -> io::Result<String> {
        let path = documentation::remap_source_path(path);
        let path = path.as_path();
        let contents = RealFileLoader.read_file(path)?;
        let mut sources = self.0.lock().expect("source snapshot lock poisoned");
        let canonical = path.canonicalize()?;
        if let Some(original) = sources.get(&canonical) {
            if original != &contents {
                return Err(source_changed(path));
            }
        } else {
            sources.insert(canonical, contents.clone());
        }
        Ok(contents)
    }

    /// 通过文档来源映射读取二进制 include 输入。
    fn read_binary_file(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        RealFileLoader.read_binary_file(&documentation::remap_source_path(path))
    }

    /// 沿用标准文件加载器的当前目录语义。
    fn current_directory(&self) -> io::Result<PathBuf> {
        RealFileLoader.current_directory()
    }
}

/// 第一轮语义发现状态，只收集待生成关系，不完成最终代码输出。
struct Discover {
    /// 首轮语义分析结果；未进入分析阶段时仍为 None。
    analysis: Option<Result<Analysis, String>>,

    /// 首轮文件加载器记录的不可替换来源快照。
    sources: Sources,
}

impl Callbacks for Discover {
    /// 启用发现阶段的查询扩展、IDE 捕获和源码快照，并延后最终 lint。
    fn config(&mut self, config: &mut interface::Config) {
        di_plan::enable(false);
        constructor::capture_ide(true);
        configure_compiler(config);
        // Generated bindings can make previously unused declarations live.
        // The final compiler invocation remains the authority for all lints.
        config.opts.lint_cap = Some(rustc_session::lint::Level::Allow);
        // Downstream `cargo check` targets also need closed blueprint MIR;
        // rustc otherwise omits it from metadata-only compilations.
        config.opts.unstable_opts.always_encode_mir = true;
        config.file_loader = Some(Box::new(SnapshotLoader(self.sources.clone())));
    }

    /// 在标准展开前准备工具反射及计划占位声明。
    fn after_crate_root_parsing(
        &mut self,
        compiler: &interface::Compiler,
        krate: &mut rustc_ast::Crate,
    ) -> Compilation {
        reflection::prepare(compiler, krate);
        di_plan::prepare(compiler, krate);
        Compilation::Continue
    }

    /// 认证私有访问与构造关系，完成自动绑定发现后停止首轮编译。
    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        if !internal_access::validate(tcx) || !constructor::validate(tcx) {
            return Compilation::Stop;
        }
        registration_codegen::validate(tcx);
        self.analysis = Some(autobind_semantic::analyze(tcx));
        Compilation::Stop
    }
}

/// 第二轮覆盖源码编译状态，要求语义输入与首轮预期一致后才继续输出。
struct Generate {
    /// 本轮尚未交给 rustc 的虚拟源码覆盖层。
    loader: Option<OverlayFileLoader>,

    /// 首轮已读取的全部原始源码，用于第二轮输入一致性校验。
    snapshots: BTreeMap<PathBuf, String>,

    /// 预期 provider、request、显式/自动 binding 与 blueprint 数量。
    expected: (usize, usize, usize, usize, usize),

    /// 第二轮语义和计划验证结果，未进入对应阶段时为 None。
    validation: Option<Result<(), String>>,
}

impl Callbacks for Generate {
    /// 启用最终计划生成并把覆盖层包装为受首轮快照约束的加载器。
    fn config(&mut self, config: &mut interface::Config) {
        di_plan::enable(true);
        constructor::capture_ide(false);
        configure_compiler(config);
        // Downstream `cargo check` targets also need closed blueprint MIR;
        // rustc otherwise omits it from metadata-only compilations.
        config.opts.unstable_opts.always_encode_mir = true;
        config.file_loader = self.loader.take().map(|loader| {
            Box::new(CheckedLoader {
                loader,
                snapshots: self.snapshots.clone(),
            }) as Box<dyn FileLoader + Send + Sync>
        });
    }

    /// 为最终编译准备同一反射和执行计划声明。
    fn after_crate_root_parsing(
        &mut self,
        compiler: &interface::Compiler,
        krate: &mut rustc_ast::Crate,
    ) -> Compilation {
        reflection::prepare(compiler, krate);
        di_plan::prepare(compiler, krate);
        Compilation::Continue
    }

    /// 核对语义数量与闭合结果，再完成全图验证及文档捕获。
    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        if !internal_access::validate(tcx) || !constructor::validate(tcx) {
            return Compilation::Stop;
        }
        registration_codegen::validate(tcx);
        self.validation = Some(autobind_semantic::analyze(tcx).and_then(|analysis| {
            let observed = (
                analysis.providers,
                analysis.requests,
                analysis.explicit_bindings,
                analysis.automatic_bindings,
                analysis.blueprints
            );

            if analysis.generated_bindings != 0
                || analysis.generated_blueprints != 0
                || observed != self.expected
            {
                Err(format!(
                    "DI semantic inputs changed between compiler passes: expected {:?}, observed {:?}, still missing {} bindings and {} blueprints",
                    self.expected, observed, analysis.generated_bindings, analysis.generated_blueprints,
                ))
            } else {
                di_plan::validate(tcx).and_then(|()| documentation::capture(tcx))
            }
        }));
        if self.validation.as_ref().is_some_and(Result::is_ok) {
            Compilation::Continue
        } else {
            Compilation::Stop
        }
    }
}

/// 安装受控语义查询扩展并保留跨 crate 原生 MIR，不替换标准 Rust 类型检查。
fn configure_compiler(config: &mut interface::Config) {
    config.opts.unstable_opts.always_encode_mir = true;
    config.override_queries = Some(|_, providers| {
        internal_access::install_queries(providers);
        constructor::provide(providers);
        registration_codegen::provide(providers);
        registration_reachability::provide(providers);
        di_plan::provide(providers);
    });
}

/// 普通依赖保持一次标准 rustc 编译。只在真实解析结果中发现传递依赖的 core
/// 时才中止本次早期探测并进入 Nestrs 两阶段管线，避免按 Cargo 的直接 extern
/// 名称漏掉使用上游重导出 ServiceProvider 的业务库。识别结果来自实际 DefId 与
/// crate source，不信任额外 sidecar 文件，也不依赖上一次构建留下的缓存标记。
#[derive(Default)]
struct TransitiveCoreProbe {
    /// 真实解析所得的唯一传递 core 工件路径。
    runtime: Option<PathBuf>,
}

impl Callbacks for TransitiveCoreProbe {
    /// 为普通外部依赖保留原生 MIR，供下游闭合查询使用。
    fn config(&mut self, config: &mut interface::Config) {
        // 即使本库没有 core 依赖，下游也可能把它的泛型 trait 转发闭合为查询。
        // cargo check 默认可省略这些原生 MIR，导致 check 与 build 得到不同的
        // 查询图。保留编译器自己的 metadata，不注入服务声明或运行期依赖。
        config.opts.unstable_opts.always_encode_mir = true;
    }

    /// 从真实依赖 crate 身份定位唯一 core，发现后转入两轮语义流程。
    fn after_expansion<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        let runtimes: Vec<_> = tcx
            .crates(())
            .iter()
            .copied()
            .filter(|&krate| tcx.crate_name(krate).as_str() == "nestrs_core")
            .collect();
        if runtimes.len() > 1 {
            tcx.dcx()
                .fatal("Nestrs requires one compatible nestrs-core identity");
        }
        let Some(&runtime) = runtimes.first() else {
            return Compilation::Continue;
        };
        let source = tcx.used_crate_source(runtime);
        self.runtime = source
            .rlib
            .as_ref()
            .or(source.rmeta.as_ref())
            .or(source.dylib.as_ref())
            .cloned();
        if self.runtime.is_none() {
            tcx.dcx()
                .fatal("cannot locate the resolved transitive nestrs-core artifact");
        }
        Compilation::Stop
    }
}

/// 编译 core 本身时启用内部计划协议和访问校验的单轮回调。
struct RuntimeCompiler;

impl Callbacks for RuntimeCompiler {
    /// 启用 core 内部计划编译协议，关闭面向应用的 IDE constructor 捕获。
    fn config(&mut self, config: &mut interface::Config) {
        di_plan::enable(true);
        constructor::capture_ide(false);
        configure_compiler(config);
    }

    /// 在 core 标准编译管线中准备反射和计划声明。
    fn after_crate_root_parsing(
        &mut self,
        compiler: &interface::Compiler,
        krate: &mut rustc_ast::Crate,
    ) -> Compilation {
        reflection::prepare(compiler, krate);
        di_plan::prepare(compiler, krate);
        Compilation::Continue
    }

    /// 认证内部访问与声明，并在输出前验证计划及收集文档。
    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        if !internal_access::validate(tcx) || !constructor::validate(tcx) {
            return Compilation::Stop;
        }
        registration_codegen::validate(tcx);
        if let Err(error) = di_plan::validate(tcx).and_then(|()| documentation::capture(tcx)) {
            tcx.dcx().err(error);
            return Compilation::Stop;
        }
        Compilation::Continue
    }
}

/// 第二轮虚拟源码加载器；源码文本必须来自首轮快照且磁盘内容未变。
struct CheckedLoader {
    /// 第二轮读取生成源码时使用的虚拟覆盖层。
    loader: OverlayFileLoader,

    /// 首轮已读取的全部原始源码，用于第二轮输入一致性校验。
    snapshots: BTreeMap<PathBuf, String>,
}

impl FileLoader for CheckedLoader {
    /// 把存在性查询交给已生成的覆盖层。
    fn file_exists(&self, path: &Path) -> bool {
        self.loader.file_exists(path)
    }

    /// 确认文件属于首轮输入且未变化，再返回对应虚拟覆盖内容。
    fn read_file(&self, path: &Path) -> io::Result<String> {
        let path = documentation::remap_source_path(path);
        let path = path.as_path();
        let canonical = path.canonicalize()?;
        let original = self.snapshots.get(&canonical).ok_or_else(|| io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Nestrs second compilation introduced a source file absent from semantic analysis: {}", path.display()),
        ))?;
        if &RealFileLoader.read_file(path)? != original {
            return Err(source_changed(path));
        }
        self.loader.read_file(path)
    }

    /// 委托覆盖层读取原始二进制资源，不应用源码文本快照校验。
    fn read_binary_file(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        self.loader.read_binary_file(path)
    }

    /// 沿用虚拟覆盖加载器的当前目录语义。
    fn current_directory(&self) -> io::Result<PathBuf> {
        self.loader.current_directory()
    }
}

/// 将编译期间输入变化报告为确定的文件加载错误。
fn source_changed(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "Nestrs source changed between compiler reads: {}",
            path.display()
        ),
    )
}

/// 提供 driver 身份查询或执行编译，并将工具错误转换为进程退出状态。
fn main() -> ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--nestrs-driver-info") {
        return match cargo_nestrs::toolchain::CompilerIdentity::pinned() {
            Ok(identity) => {
                println!("{}", identity.to_json());
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("error: {error}");
                ExitCode::FAILURE
            }
        };
    }
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: Nestrs automatic binding: {error}");
            ExitCode::FAILURE
        }
    }
}

/// 分流 rustc/rustdoc 调用，认证工具链并编排发现、覆盖生成和最终语义验证。
fn run() -> Result<ExitCode, String> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = documentation::run_builder(&mut args)? {
        return Ok(code);
    }
    if let Some(rustdoc) = std::env::var_os("NESTRS_REAL_RUSTDOC")
        && !is_rustc_wrapper_invocation(&args)
    {
        return run_rustdoc(&rustdoc, args);
    }
    if args.is_empty() || args[0].starts_with('-') {
        return Err("invoke this driver as RUSTC_WRAPPER with rustc as its first argument".into());
    }
    // Record the original compilation unit before our editor-only/tool-owned
    // extern is added. This includes all dependency and build-script invocations.
    if std::env::var_os("NESTRS_IDE_CAPTURE").is_some()
        && let Some(source) = arguments::source_file(&args[1..])?
    {
        cargo_nestrs::ide::capture_rustc(&args, &source)?;
    }
    let rustc = args[0].clone();
    let crate_name = flag_value(&args, "--crate-name").map(str::to_owned);
    let mut uses_core = has_extern(&args, "nestrs_core");
    let executable = flag_value(&args, "--crate-type")
        .is_some_and(|kinds| kinds.split(',').any(|kind| kind == "bin"));
    let graph_binary = if executable {
        crate_name
            .as_deref()
            .map(|name| graph_entry::matches_arguments(name, &args))
            .transpose()?
            .unwrap_or(false)
    } else {
        false
    };
    if graph_binary && !uses_core {
        return Err("cargo nestrs graph requires the selected binary to depend directly on nestrs-core; refusing to execute an unmodified application entry".into());
    }
    let is_probe = crate_name.is_none()
        || args
            .iter()
            .any(|arg| arg == "--print" || arg.starts_with("--print="));
    // Every downstream crate may need the bridge while decoding an upstream
    // crate's metadata, even when it does not depend on nestrs-core itself.
    let bridge = if !is_probe {
        let driver = std::env::current_exe().map_err(|error| error.to_string())?;
        let bridge = Bridge::locate(&driver)?;
        inject_dependency_search(&mut args, &bridge)?;
        internal_access::set_bridge_path(bridge.clone());
        Some(bridge)
    } else {
        None
    };
    let local_core = crate_name.as_deref() == Some("nestrs_core") && !uses_core;
    if local_core && !is_probe {
        check_toolchain(&rustc)?;
        args.extend(["--cfg".into(), "nestrs_compiler".into()]);
        internal_access::set_trusted_ranges(Vec::new());
        if !args.iter().any(|arg| arg == "--test") {
            return Ok(rustc_driver::catch_with_exit_code(|| {
                rustc_driver::run_compiler(&args, &mut RuntimeCompiler);
            }));
        }
    }
    if is_probe {
        // Cargo's version/sysroot/capability probes must retain rustc behavior.
        let status = Command::new(&rustc)
            .args(&args[1..])
            .env_remove("RUSTC_BOOTSTRAP")
            .env_remove("NESTRS_GRAPH_TARGET")
            .env_remove("NESTRS_GRAPH_BINARY")
            .env_remove("NESTRS_GRAPH_MANIFEST")
            .env_remove("NESTRS_GRAPH_SOURCE")
            .env_remove("NESTRS_GRAPH_PROOF")
            .env_remove("NESTRS_GRAPH_PLAN")
            .status()
            .map_err(|error| error.to_string())?;
        return Ok(if status.success() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        });
    }
    if !uses_core && !local_core && !documentation::requires_pipeline() && !graph_binary {
        check_toolchain(&rustc)?;
        let mut probe = TransitiveCoreProbe::default();
        let result = rustc_driver::catch_with_exit_code(|| {
            rustc_driver::run_compiler(&args, &mut probe);
        });
        if result != ExitCode::SUCCESS {
            return Ok(result);
        }
        let Some(runtime) = probe.runtime else {
            // 不含 DI 的普通第三方库已经在同一次调用内正常完成 metadata/codegen。
            return Ok(result);
        };
        // 生成 adapter 使用 core 的固定内部路径。将本次 rustc 已解析的同一 artifact
        // 作为规范 extern 别名传给重编译阶段，不重新按名称搜索另一个 core 版本。
        args.push("--extern".into());
        args.push(format!("nestrs_core={}", runtime.display()));
        uses_core = true;
    }
    let crate_name = crate_name.expect("checked compiler crate name");
    check_toolchain(&rustc)?;
    if uses_core || local_core {
        inject_extern(
            &mut args,
            &bridge.expect("compilation units locate the bridge"),
        )?;
    }
    if flag_value(&args, "--sysroot").is_none() {
        args.push("--sysroot".into());
        args.push(compiler_output(&rustc, &["--print", "sysroot"])?);
    }
    let output = PathBuf::from(
        std::env::var_os("NESTRS_COMPILER_OUTPUT")
            .ok_or("NESTRS_COMPILER_OUTPUT is required; invoke the build through cargo nestrs")?,
    );
    let mut identity = DefaultHasher::new();
    args.hash(&mut identity);
    let artifacts = output.join(format!("{crate_name}-{:016x}", identity.finish()));
    fs::create_dir_all(&artifacts).map_err(|error| error.to_string())?;
    for name in ["analysis.json", "compilation.json"] {
        let path = artifacts.join(name);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }

    let sources = Sources::default();
    internal_access::set_trusted_ranges(Vec::new());
    let mut discover = Discover {
        analysis: None,
        sources: sources.clone(),
    };
    let first = rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&args, &mut discover);
    });
    if first != ExitCode::SUCCESS {
        return Ok(first);
    }
    let analysis = discover
        .analysis
        .ok_or("compiler did not reach semantic analysis")??;
    let snapshots = {
        let snapshots = sources.lock().map_err(|error| error.to_string())?;
        for (path, original) in snapshots.iter() {
            if &fs::read_to_string(path).map_err(|error| error.to_string())? != original {
                return Err(source_changed(path).to_string());
            }
        }
        for insertion in &analysis.insertions {
            let path = insertion
                .path
                .canonicalize()
                .map_err(|error| error.to_string())?;
            let original = snapshots.get(&path).ok_or_else(|| {
                format!(
                    "source was not read in the first compiler pass: {}",
                    path.display()
                )
            })?;
            if &insertion.expected_source != original {
                return Err(format!(
                    "source changed during semantic analysis: {}",
                    path.display()
                ));
            }
        }
        snapshots.clone()
    };
    let manifest = analysis_json(&crate_name, &analysis);
    fs::write(artifacts.join("analysis.json"), manifest).map_err(|error| error.to_string())?;
    let expected = (
        analysis.providers,
        analysis.requests,
        analysis.explicit_bindings,
        analysis.automatic_bindings + analysis.generated_bindings,
        analysis.blueprints + analysis.generated_blueprints,
    );
    let loader = OverlayFileLoader::from_insertions(analysis.insertions, &artifacts)
        .map_err(|error| error.to_string())?;
    internal_access::set_trusted_ranges(
        loader
            .trusted_ranges
            .iter()
            .map(|(path, start, end)| internal_access::TrustedRange {
                path: path.clone(),
                start: *start,
                end: *end,
            })
            .collect(),
    );
    let mut generate = Generate {
        loader: Some(loader),
        snapshots,
        expected,
        validation: None,
    };
    let second = rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&args, &mut generate);
    });
    let validated = generate
        .validation
        .unwrap_or_else(|| Err("final compiler did not reach semantic verification".into()));
    fs::write(
        artifacts.join("compilation.json"),
        format!(
            "{{\"passed\":{},\"passes\":2}}\n",
            second == ExitCode::SUCCESS && validated.is_ok(),
        ),
    )
    .map_err(|error| error.to_string())?;
    if second == ExitCode::SUCCESS {
        validated?;
        if graph_binary {}
    }
    Ok(second)
}

/// 依据首参程序名区分 Cargo rustc wrapper 和直接 rustdoc 调用。
fn is_rustc_wrapper_invocation(args: &[String]) -> bool {
    args.first().is_some_and(|first| {
        Path::new(first)
            .file_name()
            .is_some_and(|name| name == "rustc" || name == "rustc.exe")
    })
}

/// rustdoc 不会使用 RUSTC_WRAPPER，因此所有 doctest 必须显式配置 driver builder。
/// 即使文档所属 crate 没有直接依赖 core，示例也可能通过上游库执行 DI；只注入 bridge
/// 搜索路径不能生成 registry。其他 rustdoc 请求继续保留普通转发行为。
fn run_rustdoc(rustdoc: &std::ffi::OsStr, mut args: Vec<String>) -> Result<ExitCode, String> {
    let uses_core = has_extern(&args, "nestrs_core");
    if args.iter().any(|argument| argument == "--test") {
        return documentation::run(rustdoc, args);
    }
    if flag_value(&args, "--crate-name").is_some() || uses_core {
        let driver = std::env::current_exe().map_err(|error| error.to_string())?;
        let bridge = Bridge::locate(&driver)?;
        inject_dependency_search(&mut args, &bridge)?;
        if uses_core {
            inject_extern(&mut args, &bridge)?;
        }
    }
    let status = Command::new(rustdoc)
        .args(args)
        .env_remove("RUSTC_BOOTSTRAP")
        .status()
        .map_err(|error| format!("cannot start the pinned rustdoc: {error}"))?;
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// 读取 rustc 选项的分隔或等号形式，不改写原始参数。
fn flag_value<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.windows(2)
        .find_map(|pair| (pair[0] == flag).then_some(pair[1].as_str()))
        .or_else(|| {
            args.iter()
                .find_map(|arg| arg.strip_prefix(&format!("{flag}=")))
        })
}

/// 执行选定 rustc 的只读查询，移除 bootstrap 并要求成功 UTF-8 输出。
fn compiler_output(rustc: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(rustc)
        .args(args)
        .env_remove("RUSTC_BOOTSTRAP")
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "rustc query failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout)
        .map(|text| text.trim().to_owned())
        .map_err(|error| error.to_string())
}

/// 将实际 rustc 的完整身份与 driver 固定身份逐字段比较。
fn check_toolchain(rustc: &str) -> Result<(), String> {
    let version = compiler_output(rustc, &["-vV"])?;
    let expected = cargo_nestrs::toolchain::CompilerIdentity::pinned()?;
    let actual = cargo_nestrs::toolchain::CompilerIdentity::parse(&version)?;
    expected.verify(&actual)
}

/// 编码本次语义发现与插入位置，作为 target 中的调试审阅记录。
fn analysis_json(crate_name: &str, analysis: &Analysis) -> String {
    let mut bindings = Vec::new();
    for insertion in &analysis.insertions {
        for binding in &insertion.bindings {
            bindings.push(format!(
                "{{\"concrete\":{},\"interface\":{},\"source\":{},\"line\":{},\"column\":{},\"insertion_file\":{},\"insertion_offset\":{}}}",
                json(&binding.concrete), json(&binding.interface), json(&binding.source_file),
                binding.source_line, binding.source_column, json(&insertion.path.to_string_lossy()), insertion.offset,
            ));
        }
    }
    format!(
        "{{\"crate\":{},\"providers\":{},\"requests\":{},\"generated_bindings\":{},\"explicit_bindings\":{},\"automatic_bindings\":{},\"bindings\":[{}],\"automatic_projections\":{},\"explicit_projections\":{}}}\n",
        json(crate_name),
        analysis.providers,
        analysis.requests,
        analysis.generated_bindings,
        analysis.explicit_bindings,
        analysis.automatic_bindings,
        bindings.join(","),
        serde_json::to_string(&analysis.automatic_projections).unwrap(),
        serde_json::to_string(&analysis.explicit_projections).unwrap(),
    )
}

/// 为分析记录转义 JSON 字符串，保留 Unicode 并编码控制字符。
fn json(value: &str) -> String {
    let mut encoded = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            c if c <= '\u{1f}' => write!(&mut encoded, "\\u{:04x}", c as u32).unwrap(),
            c => encoded.push(c),
        }
    }
    encoded.push('"');
    encoded
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn rustdoc_arguments_are_not_mistaken_for_cargo_rustc_wrapper_calls() {
        for first in ["rustc", "/toolchain/bin/rustc", "C:/toolchain/rustc.exe"] {
            assert!(is_rustc_wrapper_invocation(&[
                first.into(),
                "--crate-name".into()
            ]));
        }
        for first in ["--edition=2024", "--crate-name", "src/lib.rs", "rustc.rs"] {
            assert!(!is_rustc_wrapper_invocation(&[first.into()]));
        }
        assert!(!is_rustc_wrapper_invocation(&[]));
    }

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "nestrs-source-snapshot-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn repeated_reads_cannot_replace_the_original_snapshot() {
        let directory = Directory::new();
        let path = directory.0.join("module.rs");
        fs::write(&path, "pub struct Original;").unwrap();
        let sources = Sources::default();
        let loader = SnapshotLoader(sources.clone());
        assert_eq!(loader.read_file(&path).unwrap(), "pub struct Original;");
        fs::write(&path, "pub struct Changed;").unwrap();
        assert_eq!(
            loader.read_file(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            sources.lock().unwrap()[&path.canonicalize().unwrap()],
            "pub struct Original;"
        );
    }

    #[test]
    fn second_pass_checks_unmodified_modules_too_and_rejects_new_sources() {
        let directory = Directory::new();
        let path = directory.0.join("without_generated_bindings.rs");
        fs::write(&path, "pub struct Original;").unwrap();
        let sources = Sources::default();
        SnapshotLoader(sources.clone()).read_file(&path).unwrap();
        let loader = CheckedLoader {
            loader: OverlayFileLoader::from_insertions(vec![], &directory.0.join("artifacts"))
                .unwrap(),
            snapshots: sources.lock().unwrap().clone(),
        };
        assert_eq!(loader.read_file(&path).unwrap(), "pub struct Original;");
        fs::write(&path, "pub struct Changed;").unwrap();
        assert_eq!(
            loader.read_file(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        let new_path = directory.0.join("unexpected.rs");
        fs::write(&new_path, "pub struct Unexpected;").unwrap();
        assert_eq!(
            loader.read_file(&new_path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
