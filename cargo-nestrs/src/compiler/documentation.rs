//! Documentation tests use the real application types and compiler pipeline.
//!
//! The pinned rustdoc cannot install the driver's private-access audit. We first
//! type-check the original crate with the driver and extract its expanded HIR
//! documentation. A documentation-only carrier lets unmodified rustdoc discover
//! and run the examples without compiling application adapters a second way.
//! Every example is then compiled by this driver, including newly declared DI
//! services and closed roots. No runtime type or implementation is substituted.

use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    ffi::OsStr,
    fmt::Write as _,
    fs,
    hash::{Hash, Hasher},
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use cargo_nestrs::bridge::{Bridge, has_extern, inject_dependency_search, inject_extern};
use rustc_ast::token::{CommentKind, DocFragmentKind};
use rustc_hir::{
    Attribute, HirId, Node,
    attrs::AttributeKind,
    def::DefKind,
    def_id::{CRATE_DEF_ID, LocalDefId},
};
use rustc_middle::ty::{self, TyCtxt};

const CAPTURE: &str = "NESTRS_DOCUMENTATION_CAPTURE";
const BUILDER: &str = "NESTRS_DOCUMENTATION_BUILDER";
const BRIDGE: &str = "NESTRS_DOCUMENTATION_BRIDGE";
const SNIPPET_FILE: &str = "NESTRS_DOCUMENTATION_SNIPPET_FILE";
const SOURCE_DIRECTORY: &str = "NESTRS_DOCUMENTATION_SOURCE_DIRECTORY";

/// 文档源分析需要导出 HIR，文档示例需要生成最终入口的 registry；这两种编译都不能
/// 仅因当前 crate 没有直接声明 nestrs-core 依赖而转发给普通 rustc。上游业务库可以
/// 把 DI 完整封装在公开函数后面，是否需要注册入口不能从 --extern 列表的直接依赖判断。
pub fn requires_pipeline() -> bool {
    std::env::var_os(CAPTURE).is_some() || std::env::var_os(SNIPPET_FILE).is_some()
}

/// Called after final semantic validation, while the original crate's HIR is live.
/// Ordinary compiler invocations do nothing. The caller must propagate errors.
pub fn capture(tcx: TyCtxt<'_>) -> Result<(), String> {
    let Some(output) = std::env::var_os(CAPTURE).map(PathBuf::from) else {
        return Ok(());
    };
    let mut root = DocumentationNode::default();
    let mut origins = Vec::new();
    let crate_symbol = tcx.crate_name(rustc_hir::def_id::LOCAL_CRATE);
    let crate_name = crate_symbol.as_str();
    for owner in tcx.hir_crate_items(()).owners() {
        for (local_id, attributes) in tcx.hir_attr_map(owner).map.iter() {
            let id = HirId {
                owner,
                local_id: *local_id,
            };
            let mut fragments = Vec::new();
            for attribute in *attributes {
                if let Attribute::Parsed(AttributeKind::DocComment {
                    kind,
                    comment,
                    span,
                    ..
                }) = attribute
                {
                    let source_span = match kind {
                        DocFragmentKind::Raw(span) => *span,
                        _ => *span,
                    };
                    let source = tcx
                        .sess
                        .source_map()
                        .lookup_source_file(source_span.lo())
                        .name
                        .clone()
                        .into_local_path()
                        .ok_or("doctest 文档来源没有真实路径，不能保留相对 include 语义")?;
                    fragments.push(Fragment {
                        text: comment.as_str().to_owned(),
                        kind: match kind {
                            DocFragmentKind::Sugared(CommentKind::Line) => FragmentKind::Line,
                            DocFragmentKind::Sugared(CommentKind::Block) => FragmentKind::Block,
                            DocFragmentKind::Raw(_) => FragmentKind::Raw,
                        },
                        source: Some(
                            source.canonicalize().map_err(|error| {
                                format!("读取 doctest 文档来源路径失败：{error}")
                            })?,
                        ),
                    });
                }
            }
            if fragments.is_empty() {
                continue;
            }
            let definition = match tcx.hir_node(id) {
                Node::Crate(_) => CRATE_DEF_ID,
                Node::Item(item) => item.owner_id.def_id,
                Node::TraitItem(item) => item.owner_id.def_id,
                Node::ImplItem(item) => item.owner_id.def_id,
                Node::ForeignItem(item) => item.owner_id.def_id,
                Node::Variant(variant) => variant.def_id,
                Node::Field(field) => field.def_id,
                // Rustdoc does not turn expression/statement documentation into tests.
                _ => continue,
            };
            let path = tcx.def_path_str(definition);
            let is_root = definition == CRATE_DEF_ID;
            let components = if is_root {
                vec![]
            } else {
                documentation_components(tcx, definition, crate_name).unwrap_or_else(|_| {
                    // Arbitrary impl self types and tuple fields do not have a
                    // legal module path. Keep every example under a stable HIR
                    // identity; origins.json retains its real source and name.
                    vec![format!(
                        "__nestrs_documentation_item_{}",
                        definition.local_def_index.as_u32()
                    )]
                })
            };
            let mut node = &mut root;
            for component in &components {
                node = node.children.entry(component.clone()).or_default();
            }
            node.fragments.extend(fragments);
            origins.push(serde_json::json!({
                "item": path,
                "carrier_path": components.join("::"),
                "source": tcx.sess.source_map().span_to_diagnostic_string(tcx.hir_span(id)),
            }));
        }
    }
    let mut source = String::from(
        "// Extracted from fully type-checked application HIR. Documentation carrier only.\n",
    );
    let mut no_crate_inject = false;
    for attribute in tcx.hir_krate_attrs() {
        let Attribute::Parsed(AttributeKind::Doc(documentation)) = attribute else {
            continue;
        };
        if documentation.no_crate_inject.is_some() {
            no_crate_inject = true;
            source.push_str("#![doc(test(no_crate_inject))]\n");
        }
        for &span in &documentation.test_attrs {
            let attribute = tcx.sess.source_map().span_to_snippet(span).map_err(|error| {
                format!("无法保留 doctest crate 属性：{error:?}；请将 doc(test(attr(...))) 写在可读取源码中")
            })?;
            writeln!(source, "#![doc(test(attr({attribute})))]").unwrap();
        }
    }
    let mut source_locations = Vec::new();
    root.emit_files(&output, source, &mut source_locations, &mut 0)?;
    fs::write(
        output.with_extension("settings.json"),
        serde_json::json!({
            "no_crate_inject": no_crate_inject,
            "sources": source_locations,
        })
        .to_string(),
    )
    .map_err(|error| format!("写入 doctest 测试设置失败：{error}"))?;
    fs::write(
        output.with_extension("origins.json"),
        serde_json::to_vec_pretty(&origins).unwrap(),
    )
    .map_err(|error| format!("写入 doctest 来源索引失败：{error}"))?;
    Ok(())
}

fn documentation_components(
    tcx: TyCtxt<'_>,
    definition: LocalDefId,
    crate_name: &str,
) -> Result<Vec<String>, String> {
    let parent = tcx.parent(definition.to_def_id());
    if matches!(tcx.def_kind(parent), DefKind::Impl { .. }) {
        // Rustdoc names an impl method by its source module, the self type's
        // short name and the method. `<Concrete as Trait>` is a compiler path,
        // not the test label (`Concrete::method`) used by standard rustdoc.
        let module = tcx.parent(parent);
        let mut components = if module.is_crate_root() {
            vec![]
        } else {
            item_components(&tcx.def_path_str(module), crate_name)?
        };
        let self_type = tcx
            .type_of(parent)
            .instantiate_identity()
            .skip_normalization();
        let name = match self_type.kind() {
            ty::Adt(definition, _) => tcx.item_name(definition.did()).as_str().to_owned(),
            ty::Param(parameter) => parameter.name.as_str().to_owned(),
            ty::Bool | ty::Char | ty::Str | ty::Int(_) | ty::Uint(_) | ty::Float(_) => {
                self_type.to_string()
            }
            _ => {
                return Err(format!(
                    "当前 doctest 文档载体无法保留 impl self 类型 `{self_type}` 的测试名称；拒绝略过其示例"
                ));
            }
        };
        components.extend(item_components(&name, crate_name)?);
        components.extend(item_components(
            tcx.item_name(definition.to_def_id()).as_str(),
            crate_name,
        )?);
        Ok(components)
    } else {
        item_components(&tcx.def_path_str(definition), crate_name)
    }
}

#[derive(Default)]
struct DocumentationNode {
    fragments: Vec<Fragment>,
    children: BTreeMap<String, DocumentationNode>,
}

struct Fragment {
    text: String,
    kind: FragmentKind,
    source: Option<PathBuf>,
}

enum FragmentKind {
    Line,
    Block,
    Raw,
}

impl DocumentationNode {
    fn emit_files(
        &self,
        path: &Path,
        mut output: String,
        sources: &mut Vec<serde_json::Value>,
        next: &mut usize,
    ) -> Result<(), String> {
        let directory = path.parent().ok_or("doctest 文档载体缺少目录")?;
        let mut ordinary_source = None;
        for fragment in &self.fragments {
            let source = fragment.source.as_ref().ok_or("doctest 文档来源缺失")?;
            if matches!(fragment.kind, FragmentKind::Raw) {
                *next += 1;
                let markdown = directory.join(format!("fragment-{next}.md"));
                fs::write(&markdown, &fragment.text).map_err(|error| error.to_string())?;
                writeln!(output, "#![doc = include_str!({:?})]", markdown).unwrap();
                sources.push(serde_json::json!({"carrier": markdown, "source": source}));
            } else {
                if ordinary_source
                    .as_ref()
                    .is_some_and(|previous| previous != source)
                {
                    return Err(
                        "同一文档项的普通注释来自多个源文件，无法保持 include 的原始目录".into(),
                    );
                }
                ordinary_source = Some(source.clone());
                match fragment.kind {
                    FragmentKind::Line => {
                        for line in fragment.text.split('\n') {
                            writeln!(output, "//!{line}").unwrap();
                        }
                    }
                    FragmentKind::Block => writeln!(output, "/*!{}*/", fragment.text).unwrap(),
                    FragmentKind::Raw => unreachable!(),
                }
            }
        }
        if let Some(source) = ordinary_source {
            sources.push(serde_json::json!({"carrier": path, "source": source}));
        }
        for (name, child) in &self.children {
            *next += 1;
            let child_path = directory.join(format!("item-{next}.rs"));
            writeln!(output, "#[path = {:?}]\npub mod {name};", child_path).unwrap();
            child.emit_files(&child_path, String::new(), sources, next)?;
        }
        fs::write(path, output).map_err(|error| format!("写入 doctest 文档载体失败：{error}"))
    }
}

fn item_components(path: &str, crate_name: &str) -> Result<Vec<String>, String> {
    // Local def-path display may omit the crate prefix; downstream paths retain it.
    let suffix = path
        .strip_prefix(crate_name)
        .and_then(|tail| tail.strip_prefix("::"))
        .unwrap_or(path);
    suffix.split("::").map(|segment| {
        let plain = segment.strip_prefix("r#").unwrap_or(segment);
        let mut characters = plain.chars();
        let valid = characters.next().is_some_and(|first| first == '_' || first.is_alphabetic())
            && characters.all(|character| character == '_' || character.is_alphanumeric());
        if !valid || matches!(plain, "crate" | "self" | "Self" | "super" | "_") {
            return Err(format!("当前 doctest 文档载体不能保留项目路径 `{path}`；不跳过其示例，请将示例移到具名 module/type/function 文档中"));
        }
        Ok(format!("r#{plain}"))
    }).collect()
}

/// 对 cargo nestrs 管理的所有 doctest 使用同一编译流程，包括只经上游业务库使用 DI
/// 的 crate。应在向 rustdoc 参数注入 bridge 之前调用，文档源分析自行执行正常注入；
/// 无直接 core 的普通 crate 不会被额外注入业务声明宏。
pub fn run(rustdoc: &OsStr, arguments: Vec<String>) -> Result<ExitCode, String> {
    if !arguments.iter().any(|argument| argument == "--test") {
        return Err("Nestrs 文档适配当前只支持 rustdoc --test".into());
    }
    for reserved in [
        "--test-builder",
        "--test-builder-wrapper",
        "--merge-doctests",
    ] {
        if arguments
            .iter()
            .any(|argument| argument == reserved || argument.starts_with(&format!("{reserved}=")))
        {
            return Err(format!(
                "Nestrs doctest 管理 `{reserved}`，不能同时配置另一个 builder 或合并多个服务图"
            ));
        }
    }
    let driver = std::env::current_exe().map_err(|error| error.to_string())?;
    let bridge = Bridge::locate(&driver)?;
    let original_source = source_argument(&arguments)?;
    let crate_name = option_value(&arguments, "--crate-name")
        .ok_or("Nestrs doctest 需要 Cargo 提供 --crate-name")?;
    let mut identity = DefaultHasher::new();
    arguments.hash(&mut identity);
    let output_root = std::env::var_os("NESTRS_COMPILER_OUTPUT")
        .map(PathBuf::from)
        .ok_or("NESTRS_COMPILER_OUTPUT 缺失；请通过 cargo nestrs test 运行")?;
    let directory = output_root.join(format!(
        "documentation-{crate_name}-{:016x}-{}",
        identity.finish(),
        std::process::id()
    ));
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let carrier = directory.join("documentation.rs");
    let compiler = compiler_path(rustdoc, &arguments)?;
    let compiler_arguments = analysis_arguments(&arguments, &original_source, &directory)?;
    let analysis = Command::new(&driver)
        .arg(&compiler)
        .args(compiler_arguments)
        .env(CAPTURE, &carrier)
        .env_remove(BUILDER)
        .env_remove("RUSTC_BOOTSTRAP")
        .env_remove(SNIPPET_FILE)
        .env_remove(SOURCE_DIRECTORY)
        .status()
        .map_err(|error| format!("启动 doctest 文档类型检查失败：{error}"))?;
    if !analysis.success() {
        return Ok(ExitCode::FAILURE);
    }
    if !carrier.is_file() {
        return Err("编译器没有导出经过类型检查的 doctest 文档；拒绝运行空报告".into());
    }
    let mut runner = arguments.clone();
    let source = runner
        .iter_mut()
        .find(|argument| **argument == original_source)
        .ok_or("doctest 输入丢失")?;
    *source = carrier.to_string_lossy().into_owned();
    // 载体只有文档目录，不是原 crate 的实现工件。尤其 proc-macro 不允许导出载体
    // 使用的普通 module，因此 runner 应按 lib 读取它。上面的真实源码分析仍保留
    // 原 crate-type，下面的 snippet 也仍通过原 --extern 加载真实库/过程宏工件。
    for index in 0..runner.len() {
        if runner[index] == "--crate-type" {
            runner[index + 1] = "lib".into();
        } else if runner[index].starts_with("--crate-type=") {
            runner[index] = "--crate-type=lib".into();
        }
    }
    let settings: serde_json::Value = serde_json::from_slice(
        &fs::read(carrier.with_extension("settings.json"))
            .map_err(|error| format!("读取 doctest 测试设置失败：{error}"))?,
    )
    .map_err(|error| format!("解析 doctest 测试设置失败：{error}"))?;
    let runner_crate_name = if settings["no_crate_inject"] == true {
        // The pinned rustdoc ignores no_crate_inject for non-merged tests. DI
        // snippets must remain separate link units, so retain the real --extern
        // and choose a carrier identity absent from all extracted documentation.
        // Rustdoc then cannot inject the documented library implicitly. No test
        // source or runtime type is rewritten, including edition-2015 snippets.
        let mut documentation = fs::read_to_string(&carrier).map_err(|error| error.to_string())?;
        for source in settings["sources"]
            .as_array()
            .ok_or("doctest 来源映射缺失")?
        {
            let path = source["carrier"].as_str().ok_or("doctest 载体路径缺失")?;
            documentation.push_str(&fs::read_to_string(path).map_err(|error| error.to_string())?);
        }
        let mut name = "__nestrs_doctest_carrier".to_owned();
        while documentation.contains(&name) {
            name.push('_');
        }
        for index in 0..runner.len() {
            if runner[index] == "--crate-name" {
                runner[index + 1] = name.clone();
            } else if runner[index].starts_with("--crate-name=") {
                runner[index] = format!("--crate-name={name}");
            }
        }
        name
    } else {
        crate_name.to_owned()
    };
    inject_dependency_search(&mut runner, &bridge)?;
    if has_extern(&runner, "nestrs_core") {
        inject_extern(&mut runner, &bridge)?;
    }
    runner.extend([
        "-Zunstable-options".into(),
        format!("--test-builder={}", compiler.display()),
        format!("--test-builder-wrapper={}", driver.display()),
        "--merge-doctests=no".into(),
    ]);
    let status = Command::new(rustdoc)
        .args(runner)
        // This narrowly enables the pinned rustdoc's wrapper option. The driver
        // strips this rustdoc-only flag and bootstrap never reaches snippet rustc.
        .env("RUSTC_BOOTSTRAP", runner_crate_name)
        .env(BUILDER, &directory)
        .env(BRIDGE, &bridge)
        .env_remove(CAPTURE)
        .status()
        .map_err(|error| format!("启动标准 rustdoc runner 失败：{error}"))?;
    Ok(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Normalize only rustdoc-owned builder calls, before driver dispatch examines
/// crate-name/extern/source arguments. Retain the real source in target so the
/// ordinary two-pass compiler can snapshot, authenticate and overlay it.
pub fn prepare_builder_arguments(arguments: &mut Vec<String>) -> Result<(), String> {
    let Some(directory) = std::env::var_os(BUILDER).map(PathBuf::from) else {
        return Ok(());
    };
    let Some(compiler) = arguments.first() else {
        return Err("doctest builder 缺少 rustc 参数".into());
    };
    if !Path::new(compiler)
        .file_name()
        .is_some_and(|name| name == "rustc" || name == "rustc.exe")
    {
        return Ok(());
    }
    let mut expanded = vec![compiler.clone()];
    for argument in &arguments[1..] {
        if let Some(path) = argument.strip_prefix('@') {
            if path.starts_with("shell:") {
                return Err("Nestrs doctest 不接受 shell 形式的 rustc 参数文件".into());
            }
            expanded.extend(
                fs::read_to_string(path)
                    .map_err(|error| format!("读取 doctest 参数文件失败：{error}"))?
                    .lines()
                    .map(str::to_owned),
            );
        } else {
            expanded.push(argument.clone());
        }
    }
    let bridge = std::env::var_os(BRIDGE)
        .map(PathBuf::from)
        .ok_or("doctest builder 缺少已确认 bridge 路径")?;
    let mut normalized = vec![expanded[0].clone()];
    let mut index = 1;
    while index < expanded.len() {
        let argument = &expanded[index];
        if argument == "-Zunstable-options" {
            index += 1;
            continue;
        }
        if argument == "-Z"
            && expanded
                .get(index + 1)
                .is_some_and(|value| value == "unstable-options")
        {
            index += 2;
            continue;
        }
        let (external, count) = if argument == "--extern" {
            (expanded.get(index + 1).map(String::as_str), 2)
        } else {
            (argument.strip_prefix("--extern="), 1)
        };
        if let Some(external) = external
            && let Some(path) = external.strip_prefix("nestrs=")
        {
            if Path::new(path) != bridge {
                return Err("doctest 的 nestrs extern 与工具 bridge 不匹配".into());
            }
            index += count;
            continue;
        }
        normalized.push(argument.clone());
        index += 1;
    }
    if let Some(index) = normalized.iter().position(|argument| argument == "-") {
        let mut source = String::new();
        io::stdin()
            .read_to_string(&mut source)
            .map_err(|error| error.to_string())?;
        let input = directory.join(format!("snippet-{}", std::process::id()));
        fs::create_dir_all(&input).map_err(|error| error.to_string())?;
        let path = input.join("rust_out.rs");
        fs::write(&path, source).map_err(|error| error.to_string())?;
        let test_path = std::env::var("UNSTABLE_RUSTDOC_TEST_PATH")
            .map_err(|_| "rustdoc builder 没有提供测试文档路径")?;
        let settings: serde_json::Value = serde_json::from_slice(
            &fs::read(directory.join("documentation.settings.json"))
                .map_err(|error| format!("读取 doctest 来源映射失败：{error}"))?,
        )
        .map_err(|error| format!("解析 doctest 来源映射失败：{error}"))?;
        let source_path = settings["sources"]
            .as_array()
            .and_then(|sources| {
                sources
                    .iter()
                    .find(|source| source["carrier"] == test_path)
                    .and_then(|source| source["source"].as_str())
            })
            .ok_or_else(|| {
                format!("doctest 文档 {test_path} 没有对应真实源文件，拒绝改变相对 include 语义")
            })?;
        fs::write(
            input.join("origin.json"),
            serde_json::json!({
                "snippet": path,
                "source": source_path,
            })
            .to_string(),
        )
        .map_err(|error| error.to_string())?;
        normalized[index] = path.to_string_lossy().into_owned();
    }
    if option_value(&normalized, "--crate-name").is_none() {
        normalized.extend(["--crate-name".into(), "rust_out".into()]);
    }
    *arguments = normalized;
    Ok(())
}

/// Re-execute a normalized builder call in a clean environment. This prevents
/// even a package named `rust_out` from inheriting rustdoc's narrowly scoped
/// bootstrap permission. Return `None` for ordinary driver/rustdoc invocations.
pub fn run_builder(arguments: &mut Vec<String>) -> Result<Option<ExitCode>, String> {
    if std::env::var_os(BUILDER).is_none()
        || !arguments.first().is_some_and(|argument| {
            Path::new(argument)
                .file_name()
                .is_some_and(|name| name == "rustc" || name == "rustc.exe")
        })
    {
        return Ok(None);
    }
    prepare_builder_arguments(arguments)?;
    let mut command = Command::new(std::env::current_exe().map_err(|error| error.to_string())?);
    command
        .args(&*arguments)
        .env_remove("RUSTC_BOOTSTRAP")
        .env_remove(BUILDER)
        .env_remove(BRIDGE)
        .env_remove(CAPTURE);
    for argument in arguments.iter() {
        let input = Path::new(argument);
        if input.file_name().is_some_and(|name| name == "rust_out.rs") {
            let origin_path = input.with_file_name("origin.json");
            if origin_path.is_file() {
                let origin: serde_json::Value = serde_json::from_slice(
                    &fs::read(origin_path).map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
                let source = Path::new(origin["source"].as_str().ok_or("doctest 原始路径缺失")?);
                command.env(SNIPPET_FILE, input).env(
                    SOURCE_DIRECTORY,
                    source.parent().ok_or("doctest 原始路径没有目录")?,
                );
            }
        }
    }
    let status = command
        .status()
        .map_err(|error| format!("启动隔离 doctest 编译器失败：{error}"))?;
    Ok(Some(if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }))
}

/// Resolve a doctest's relative includes against its real documentation source.
/// The saved main snippet keeps its target identity for two-pass authentication;
/// included files use the original paths, including nested and binary includes.
pub fn remap_source_path(path: &Path) -> PathBuf {
    let Some(snippet) = std::env::var_os(SNIPPET_FILE).map(PathBuf::from) else {
        return path.to_owned();
    };
    let Some(directory) = std::env::var_os(SOURCE_DIRECTORY).map(PathBuf::from) else {
        return path.to_owned();
    };
    remap_relative_source(path, &snippet, &directory)
}

fn remap_relative_source(path: &Path, snippet: &Path, directory: &Path) -> PathBuf {
    if path == snippet {
        return path.to_owned();
    }
    snippet
        .parent()
        .and_then(|parent| path.strip_prefix(parent).ok())
        .map_or_else(|| path.to_owned(), |relative| directory.join(relative))
}

fn compiler_path(rustdoc: &OsStr, arguments: &[String]) -> Result<PathBuf, String> {
    let executable = if cfg!(windows) { "rustc.exe" } else { "rustc" };
    if let Some(sysroot) = option_value(arguments, "--sysroot") {
        return Ok(Path::new(sysroot).join("bin").join(executable));
    }
    let path = Path::new(rustdoc);
    if path.is_absolute() {
        return Ok(path.with_file_name(executable));
    }
    let output = Command::new(rustdoc).args(["--print", "sysroot"]).output();
    if let Ok(output) = output
        && output.status.success()
        && let Ok(root) = String::from_utf8(output.stdout)
    {
        return Ok(Path::new(root.trim()).join("bin").join(executable));
    }
    // The driver validates this compiler against the full toolchain pin before
    // performing any semantic work; no unrelated compiler is accepted silently.
    Ok(std::env::var_os("NESTRS_RUSTC")
        .map(PathBuf::from)
        .unwrap_or_else(|| executable.into()))
}

fn option_value<'a>(arguments: &'a [String], option: &str) -> Option<&'a str> {
    arguments.iter().enumerate().find_map(|(index, argument)| {
        if argument == option {
            arguments.get(index + 1).map(String::as_str)
        } else {
            argument.strip_prefix(&format!("{option}="))
        }
    })
}

fn source_argument(arguments: &[String]) -> Result<String, String> {
    let sources: Vec<_> = arguments
        .iter()
        .filter(|argument| {
            !argument.starts_with('-') && argument.ends_with(".rs") && Path::new(argument).is_file()
        })
        .collect();
    if sources.len() != 1 {
        return Err(format!(
            "Nestrs doctest 需要唯一 Rust 源文件，发现 {} 个候选",
            sources.len()
        ));
    }
    Ok(sources[0].clone())
}

fn analysis_arguments(
    arguments: &[String],
    source: &str,
    directory: &Path,
) -> Result<Vec<String>, String> {
    let compiler_options = [
        "--crate-name",
        "--crate-type",
        "--edition",
        "--target",
        "--sysroot",
        "--cfg",
        "--check-cfg",
        "--extern",
        "--error-format",
        "--json",
        "--diagnostic-width",
        "--cap-lints",
        "--allow",
        "--warn",
        "--force-warn",
        "--deny",
        "--forbid",
        "--library-path",
        "--codegen",
        "--remap-path-prefix",
        "--remap-path-scope",
        "--color",
        "-L",
        "-C",
        "-A",
        "-W",
        "-D",
        "-F",
    ];
    let rustdoc_values = [
        "--test-args",
        "--test-run-directory",
        "--out-dir",
        "--output",
        "-o",
        "--crate-version",
        "--persist-doctests",
        "--test-runtool",
        "--test-runtool-arg",
        "--doctest-build-arg",
        "--extern-html-root-url",
        "--markdown-playground-url",
        "--playground-url",
    ];
    let rustdoc_switches = [
        "--test",
        "--no-run",
        "--no-capture",
        "--display-doctest-warnings",
        "--document-private-items",
        "--document-hidden-items",
        "--enable-index-page",
    ];
    let mut output = vec![];
    let crate_name = option_value(arguments, "--crate-name");
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        // Cargo supplies the already built library to rustdoc so examples can
        // import it. Source analysis must compile the actual local crate, not
        // import a second copy of itself (especially nestrs_core's private ABI).
        let (external, consumed) = if argument == "--extern" {
            (arguments.get(index + 1).map(String::as_str), 2)
        } else {
            (argument.strip_prefix("--extern="), 1)
        };
        if external.is_some_and(|external| {
            crate_name.is_some_and(|name| external.split('=').next() == Some(name))
        }) {
            index += consumed;
            continue;
        }
        if argument == source {
            output.push(argument.clone());
            index += 1;
            continue;
        }
        if let Some(option) = compiler_options
            .iter()
            .find(|option| argument.as_str() == **option)
        {
            let value = arguments
                .get(index + 1)
                .ok_or_else(|| format!("{option} 缺少参数"))?;
            output.extend([argument.clone(), value.clone()]);
            index += 2;
            continue;
        }
        if compiler_options.iter().any(|option| {
            argument.starts_with(&format!("{option}="))
                || (option.len() == 2 && argument.starts_with(option) && argument.len() > 2)
        }) {
            output.push(argument.clone());
            index += 1;
            continue;
        }
        if let Some(option) = rustdoc_values
            .iter()
            .find(|option| argument.as_str() == **option)
        {
            if arguments.get(index + 1).is_none() {
                return Err(format!("{option} 缺少参数"));
            }
            index += 2;
            continue;
        }
        if rustdoc_values
            .iter()
            .any(|option| argument.starts_with(&format!("{option}=")))
            || rustdoc_switches.contains(&argument.as_str())
        {
            index += 1;
            continue;
        }
        return Err(format!(
            "Nestrs doctest 尚未适配参数 `{argument}`；拒绝静默忽略编译或测试语义"
        ));
    }
    if option_value(&output, "--crate-type").is_none() {
        output.extend(["--crate-type".into(), "lib".into()]);
    }
    output.extend([
        "--cfg".into(),
        "doc".into(),
        "--cfg".into(),
        "doctest".into(),
        "--emit=metadata".into(),
        "-o".into(),
        directory
            .join("analysis.rmeta")
            .to_string_lossy()
            .into_owned(),
    ]);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_analysis_preserves_compile_flags_without_running_tests() {
        let arguments = [
            "--crate-name",
            "example",
            "--edition=2024",
            "src/lib.rs",
            "--test",
            "--extern",
            "nestrs_core=core.rlib",
            "--cfg",
            "feature=\"special\"",
            "--test-args",
            "--nocapture",
            "--no-run",
            "--warn=unexpected_cfgs",
            "--deny",
            "unsafe_code",
            "--extern=example=example.rlib",
            "--test-run-directory",
            "/project",
        ];
        let arguments: Vec<_> = arguments.into_iter().map(str::to_owned).collect();
        let output =
            analysis_arguments(&arguments, "src/lib.rs", Path::new("/target/docs")).unwrap();
        assert!(
            output
                .windows(2)
                .any(|pair| pair == ["--cfg", "feature=\"special\""])
        );
        assert!(
            output
                .windows(2)
                .any(|pair| pair == ["--extern", "nestrs_core=core.rlib"])
        );
        assert!(
            !output
                .iter()
                .any(|value| value == "--test" || value == "--no-run" || value == "--nocapture")
        );
        assert!(output.windows(2).any(|pair| pair == ["--cfg", "doc"]));
        assert!(output.iter().any(|value| value == "--emit=metadata"));
        assert!(output.iter().any(|value| value == "--warn=unexpected_cfgs"));
        assert!(
            output
                .windows(2)
                .any(|pair| pair == ["--deny", "unsafe_code"])
        );
        assert!(!output.iter().any(|value| value.contains("example.rlib")));
    }

    #[test]
    fn unknown_documentation_flags_fail_instead_of_weakening_validation() {
        assert!(
            analysis_arguments(
                &["--unrecognized-policy".into()],
                "src/lib.rs",
                Path::new("/target")
            )
            .unwrap_err()
            .contains("--unrecognized-policy")
        );
    }

    #[test]
    fn carrier_keeps_code_flags_hidden_lines_and_rust_documentation_kinds() {
        let mut node = DocumentationNode::default();
        node.fragments.push(Fragment {
            text: " ```rust,compile_fail,E0308\n # let kept = 1;\n let _: &str = kept;\n ```"
                .into(),
            kind: FragmentKind::Line,
            source: Some(PathBuf::from("/project/src/lib.rs")),
        });
        node.fragments.push(Fragment {
            text: "\n * ```no_run\n * loop {}\n * ```\n ".into(),
            kind: FragmentKind::Block,
            source: Some(PathBuf::from("/project/src/lib.rs")),
        });
        node.fragments.push(Fragment {
            text: "```should_panic\npanic!(\"expected\");\n```".into(),
            kind: FragmentKind::Raw,
            source: Some(PathBuf::from("/project/src/lib.rs")),
        });
        let directory = std::env::temp_dir().join(format!(
            "nestrs-doc-carrier-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("documentation.rs");
        let mut sources = vec![];
        node.emit_files(&path, String::new(), &mut sources, &mut 0)
            .unwrap();
        let output = fs::read_to_string(&path).unwrap();
        assert!(output.contains("//! ```rust,compile_fail,E0308\n//! # let kept = 1;"));
        assert!(output.contains("/*!\n * ```no_run"));
        assert!(output.contains("#![doc = include_str!"));
        assert_eq!(
            fs::read_to_string(directory.join("fragment-1.md")).unwrap(),
            "```should_panic\npanic!(\"expected\");\n```"
        );
        assert_eq!(sources.len(), 2);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ordinary_documentation_paths_preserve_names_and_unrepresentable_paths_are_explicit() {
        assert_eq!(
            item_components("app::checkout::run", "app").unwrap(),
            ["r#checkout", "r#run"]
        );
        assert!(item_components("app::{impl#0}::run", "app").is_err());
    }

    #[test]
    fn snippet_includes_keep_the_real_document_directory_without_moving_the_main_input() {
        let snippet = Path::new("/target/docs/snippet/rust_out.rs");
        let source = Path::new("/project/src/guides");
        assert_eq!(remap_relative_source(snippet, snippet, source), snippet);
        assert_eq!(
            remap_relative_source(
                Path::new("/target/docs/snippet/nested/value.txt"),
                snippet,
                source
            ),
            Path::new("/project/src/guides/nested/value.txt")
        );
        assert_eq!(
            remap_relative_source(
                Path::new("/target/docs/snippet/../value.txt"),
                snippet,
                source
            ),
            Path::new("/project/src/guides/../value.txt")
        );
        assert_eq!(
            remap_relative_source(Path::new("/library/other.rs"), snippet, source),
            Path::new("/library/other.rs")
        );
    }
}
