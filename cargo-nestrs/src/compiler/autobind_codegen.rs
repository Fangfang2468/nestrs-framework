//! Second-pass binding emission and source overlays for the Nestrs compiler driver.
//!
//! Semantic discovery supplies type expressions and an insertion point inside a
//! real Rust module. This module deliberately does not discover modules, parse
//! `impl` text, infer trait relations, or modify the user's source files.

use rustc_span::source_map::{FileLoader, RealFileLoader};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A semantic concrete/interface capability selected by the first compiler pass.
#[derive(Clone, Debug)]
pub struct BindingSpec {
    /// A fully qualified, closed, source-expressible concrete type.
    pub concrete: String,

    /// The complete `dyn` type, including associated types and auto traits.
    pub interface: String,

    /// Original declaration location, independent of generated source offsets.
    pub source_file: String,

    /// 声明位置的 1 基行号，用于生成投影的来源诊断。
    pub source_line: u32,

    /// 声明位置的 1 基列号。
    pub source_column: u32,
}

/// An insertion into the source version inspected by semantic discovery.
#[derive(Clone, Debug)]
pub struct SourceInsertion {
    /// 需要覆盖的真实源码路径；第二轮编译仍使用该文件身份。
    pub path: PathBuf,

    /// Original UTF-8 byte offset, not rustc's normalized source offset.
    pub offset: usize,

    /// 第一轮读取的完整文本，用于拒绝两轮之间发生的源码变化。
    pub expected_source: String,

    /// 此插入点能够合法命名并生成的闭合投影能力。
    pub bindings: Vec<BindingSpec>,
}

/// Reuse the existing audited binding ABI; rustc checks each ordinary coercion.
///
/// The local alias gives the trait object its normal `'static` alias lifetime.
/// Typed projectors keep the existing strong leases; no raw vtable construction
/// or reference-lifetime conversion is introduced by the compiler adapter.
/// Capabilities remain separate from explicit registrations until graph
/// compilation activates an interface demanded by the actual link unit.
/// A private child module isolates generated bindings from business constants
/// and statics. Type paths are already fully qualified, and the anonymous const
/// keeps the module itself out of the business namespace.
pub fn binding_source(binding: &BindingSpec) -> String {
    let module = crate::protocol::REFLECTION_MODULE;
    let marker = crate::protocol::Marker::AutomaticBinding.name();
    let concrete = &binding.concrete;
    let interface = &binding.interface;
    let origin = format!(
        "{:?}",
        format!(
            "{}:{}:{}",
            binding.source_file, binding.source_line, binding.source_column
        )
    );
    format!(
        r#"#[allow(clippy::unused_unit)]
#[doc = {origin}]
const _: () = {{
    #[allow(dead_code)]
    mod __nestrs_binding {{
    #[allow(dead_code)]
    mod {module} {{
        #[inline(never)]
        pub const fn {marker}<C: ?Sized, I: ?Sized>() {{
            let _ = (core::marker::PhantomData::<C>, core::marker::PhantomData::<I>);
        }}
    }}
    type __NestrsBoundInterface = {interface};

    fn __nestrs_project_bound_service(
        service: &{concrete},
    ) -> &__NestrsBoundInterface {{
        let projected: &__NestrsBoundInterface = service;
        projected
    }}
        #[allow(dead_code)]
    #[allow(clippy::needless_borrow)]
    fn __nestrs_reflect_automatic_binding() -> ::nestrs_core::activation::adapter::ProjectionAdapter {{
        {module}::{marker}::<{concrete}, __NestrsBoundInterface>();
        ::nestrs_core::activation::adapter::ProjectionAdapter {{
            trait_type: ::nestrs_core::service::ServiceType::create::<__NestrsBoundInterface>(),
            concrete_type: ::nestrs_core::service::ServiceType::create::<{concrete}>(),
            project: (|
                slot: ::nestrs_core::activation::InputSlot,
                input: ::nestrs_core::activation::ErasedServiceRef,
                target: &mut ::nestrs_core::activation::ProjectionTarget<'_>,
            | {{
                ::nestrs_core::activation::project_bound::<
                    {concrete}, __NestrsBoundInterface,
                >(slot, input, target, __nestrs_project_bound_service)
            }}) as ::nestrs_core::activation::ServiceProjector,
        }}
    }}
    }}

    ()
}};
"#,
    )
}

/// 单个文件的两阶段快照与替换文本；不回写业务源码。
struct Overlay {
    /// 第一轮分析使用的原始文件内容。
    expected_source: String,

    /// 包含自动绑定代码的第二轮编译输入。
    replacement: String,
}

/// Preserve original file identities so normal module and include resolution
/// work during the second full compilation. Artifacts are reviewable copies;
/// rustc still sees the original paths through its `FileLoader` boundary.
pub struct OverlayFileLoader {
    /// 按规范路径保存的覆盖文本与对应快照。
    files: BTreeMap<PathBuf, Overlay>,

    /// 未覆盖文件和二进制资源沿用的原生加载器。
    real: RealFileLoader,

    /// 生成片段的精确 UTF-8 字节范围，供私有访问审计使用。
    pub trusted_ranges: Vec<(PathBuf, usize, usize)>,
}

impl OverlayFileLoader {
    /// 校验所有第一轮快照与插入位置，再生成覆盖层和可审阅工件；任一源码变化均返回错误。
    pub fn from_insertions(
        insertions: Vec<SourceInsertion>,
        artifact_dir: &Path,
    ) -> io::Result<Self> {
        let mut grouped: BTreeMap<PathBuf, Vec<SourceInsertion>> = BTreeMap::new();
        for insertion in insertions {
            if insertion.bindings.is_empty() {
                continue;
            }
            let path = insertion.path.canonicalize()?;
            grouped.entry(path).or_default().push(insertion);
        }

        // Validate every file and location before writing any artifacts.
        let mut prepared = Vec::new();
        let mut trusted_ranges = Vec::new();
        for (path, mut insertions) in grouped {
            let original = fs::read_to_string(&path)?;
            for insertion in &insertions {
                if original != insertion.expected_source {
                    return Err(stale_source(&path));
                }
                if !original.is_char_boundary(insertion.offset) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!(
                            "Nestrs binding insertion for {} is outside the file or inside a UTF-8 character at byte {}",
                            path.display(),
                            insertion.offset,
                        ),
                    ));
                }
            }
            insertions.sort_by_key(|insertion| insertion.offset);

            let mut replacement = String::new();
            let mut generated = String::new();
            let mut previous = 0;
            for insertion in insertions {
                replacement.push_str(&original[previous..insertion.offset]);
                previous = insertion.offset;
                if insertion.offset == original.len()
                    && !replacement.is_empty()
                    && !replacement.ends_with('\n')
                {
                    // A whole-file module may end in a // comment without a
                    // trailing newline. At EOF there is no subsequent original
                    // source whose line numbers could be affected.
                    replacement.push('\n');
                }
                for source in insertion.bindings.iter().map(binding_source) {
                    let start = replacement.len();
                    generated.push_str(&source);
                    generated.push('\n');

                    // Keeping the insertion on one physical line preserves
                    // original line!() values and subsequent diagnostic lines.
                    // Only columns after the insertion on that same line move.
                    replacement.push(' ');
                    for line in source.lines() {
                        replacement.push_str(line.trim());
                        replacement.push(' ');
                    }
                    trusted_ranges.push((path.clone(), start, replacement.len()));
                }
            }
            replacement.push_str(&original[previous..]);
            prepared.push((
                path,
                Overlay {
                    expected_source: original,
                    replacement,
                },
                generated,
            ));
        }

        fs::create_dir_all(artifact_dir)?;
        let mut manifest = String::new();
        let mut files = BTreeMap::new();
        for (index, (path, overlay, generated)) in prepared.into_iter().enumerate() {
            let overlay_path = artifact_dir.join(format!("{index:04}-overlay.rs"));
            let generated_path = artifact_dir.join(format!("{index:04}-bindings.rs"));
            fs::write(&overlay_path, &overlay.replacement)?;
            fs::write(&generated_path, generated)?;
            manifest.push_str(&format!(
                "source: {path:?}\noverlay: {overlay_path:?}\nbindings: {generated_path:?}\n\n",
            ));
            files.insert(path, overlay);
        }
        fs::write(artifact_dir.join("sources.txt"), manifest)?;
        Ok(Self {
            files,
            real: RealFileLoader,
            trusted_ranges,
        })
    }

    /// 按真实路径查找覆盖文本；读取时再次确认源码未在两轮间变化。
    fn overlay(&self, path: &Path) -> io::Result<Option<&Overlay>> {
        let path = crate::documentation::remap_source_path(path);
        let path = path.as_path();
        let Ok(canonical) = path.canonicalize() else {
            return Ok(None);
        };
        let Some(overlay) = self.files.get(&canonical) else {
            return Ok(None);
        };
        // A source edit between passes must fail, never consume stale offsets.
        if fs::read_to_string(path)? != overlay.expected_source {
            return Err(stale_source(path));
        }
        Ok(Some(overlay))
    }
}

impl FileLoader for OverlayFileLoader {
    /// 沿文档来源映射查询真实文件是否存在。
    fn file_exists(&self, path: &Path) -> bool {
        self.real
            .file_exists(&crate::documentation::remap_source_path(path))
    }

    /// 返回已校验的覆盖源码，未覆盖路径交给原生加载器。
    fn read_file(&self, path: &Path) -> io::Result<String> {
        let path = crate::documentation::remap_source_path(path);
        let path = path.as_path();
        match self.overlay(path)? {
            Some(overlay) => Ok(overlay.replacement.clone()),
            None => self.real.read_file(path),
        }
    }

    /// 始终读取原始资源字节，避免 include_bytes! 误读源码覆盖文本。
    fn read_binary_file(&self, path: &Path) -> io::Result<Arc<[u8]>> {
        // include_bytes! observes the user's actual asset bytes, even when the
        // same path is a Rust source file being compiled with an overlay.
        self.real
            .read_binary_file(&crate::documentation::remap_source_path(path))
    }

    /// 沿用原生加载器的当前工作目录。
    fn current_directory(&self) -> io::Result<PathBuf> {
        self.real.current_directory()
    }
}

/// 将两阶段快照不一致转换为包含实际路径的 I/O 错误。
fn stale_source(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "Nestrs source changed between semantic analysis and binding compilation: {}",
            path.display(),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "nestrs-overlay-test-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed),
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn source(&self, source: &str) -> PathBuf {
            let path = self.0.join("input.rs");
            fs::write(&path, source).unwrap();
            path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn insertion(path: &Path, source: &str, offset: usize) -> SourceInsertion {
        SourceInsertion {
            path: path.into(),
            offset,
            expected_source: source.into(),
            bindings: vec![BindingSpec {
                concrete: "crate::Service".into(),
                interface: "dyn crate::Port + Send + Sync".into(),
                source_file: path.to_string_lossy().into_owned(),
                source_line: 1,
                source_column: 1,
            }],
        }
    }

    #[test]
    fn generated_projection_is_a_latent_capability_not_an_explicit_binding() {
        use zyn::syn::{self, Expr, Item, Stmt};

        let spec = BindingSpec {
            concrete: "crate::private::Repository<std::string::String>".into(),
            interface: "dyn crate::Port<Entity = std::string::String> + Send + Sync".into(),
            source_file: "source/with \"quotes\"/service.rs".into(),
            source_line: 17,
            source_column: 9,
        };
        let file = syn::parse_file(&binding_source(&spec)).unwrap();
        let Item::Const(generated) = &file.items[0] else {
            panic!("generated projection must remain inside its private anonymous const");
        };
        let Expr::Block(block) = &*generated.expr else {
            panic!("generated anonymous const must contain the projection items");
        };
        let isolated = block
            .block
            .stmts
            .iter()
            .find_map(|statement| match statement {
                Stmt::Item(Item::Mod(module)) if module.ident == "__nestrs_binding" => {
                    module.content.as_ref().map(|(_, items)| items)
                }
                _ => None,
            })
            .expect("generated bindings need a module boundary without business imports");
        assert!(isolated.iter().all(|item| !matches!(item, Item::Use(_))));
        let callback = isolated
            .iter()
            .find_map(|item| match item {
                Item::Fn(function)
                    if function.sig.ident == "__nestrs_reflect_automatic_binding" =>
                {
                    Some(function)
                }
                _ => None,
            })
            .expect("automatic capability callback should exist");
        assert!(callback.attrs.iter().all(|attribute| {
            !attribute
                .path()
                .segments
                .iter()
                .any(|segment| segment.ident == "distributed_slice")
        }));
        let syn::ReturnType::Type(_, output) = &callback.sig.output else {
            panic!("the compiler catalog callback must return a typed binding");
        };
        let syn::Type::Path(output) = &**output else {
            panic!("typed binding path");
        };
        assert_eq!(
            output.path.segments.last().unwrap().ident,
            "ProjectionAdapter"
        );

        let Stmt::Expr(Expr::Call(marker), _) = &callback.block.stmts[0] else {
            panic!("callback must retain a type-bearing capability marker");
        };
        let Expr::Path(marker) = &*marker.func else {
            panic!("capability marker must be a direct function call");
        };
        assert_eq!(
            marker.path.segments.last().unwrap().ident,
            "compiler_automatic_binding"
        );
    }

    #[test]
    fn overlays_multiple_offsets_without_editing_the_original_or_its_line_count() {
        let directory = TestDirectory::new();
        let original = "\u{feff}mod inner { /* 中文 */ }\r\nfn main() {}\r\n";
        let path = directory.source(original);
        let first = original.find(" }\r\n").unwrap();
        let second = original.len();
        let loader = OverlayFileLoader::from_insertions(
            vec![
                insertion(&path, original, second),
                insertion(&path, original, first),
            ],
            &directory.0.join("artifacts"),
        )
        .unwrap();
        let compiled_source = loader.read_file(&path).unwrap();
        assert!(compiled_source.starts_with(&original[..first]));
        assert!(compiled_source.contains(&original[first..second]));
        assert_eq!(
            compiled_source.lines().count(),
            original.lines().count() + 1
        );
        assert_eq!(
            compiled_source
                .bytes()
                .filter(|byte| *byte == b'\n')
                .count(),
            original.bytes().filter(|byte| *byte == b'\n').count(),
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert_eq!(
            &*loader.read_binary_file(&path).unwrap(),
            original.as_bytes()
        );
        assert!(directory.0.join("artifacts/0000-bindings.rs").exists());
    }

    #[test]
    fn rejects_offsets_inside_a_multibyte_character_and_beyond_the_file() {
        let directory = TestDirectory::new();
        let original = "// 中文\n";
        let path = directory.source(original);
        for offset in [4, original.len() + 1] {
            let result = OverlayFileLoader::from_insertions(
                vec![insertion(&path, original, offset)],
                &directory.0.join("artifacts"),
            );
            assert!(matches!(result, Err(error) if error.kind() == io::ErrorKind::InvalidInput));
        }
        assert!(!directory.0.join("artifacts").exists());
    }

    #[test]
    fn eof_insertions_cannot_be_swallowed_by_an_unterminated_line_comment() {
        let directory = TestDirectory::new();
        let original = "// module ends without a newline";
        let path = directory.source(original);
        let loader = OverlayFileLoader::from_insertions(
            vec![insertion(&path, original, original.len())],
            &directory.0.join("artifacts"),
        )
        .unwrap();
        assert!(
            loader
                .read_file(&path)
                .unwrap()
                .starts_with(&format!("{original}\n #[allow(clippy::unused_unit)]"))
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
    }

    #[test]
    fn source_changes_are_rejected_before_generation_and_before_second_pass_reads() {
        let directory = TestDirectory::new();
        let original = "mod inner {}";
        let path = directory.source(original);
        let stale = OverlayFileLoader::from_insertions(
            vec![insertion(&path, "old source", 0)],
            &directory.0.join("stale"),
        );
        assert!(matches!(stale, Err(error) if error.kind() == io::ErrorKind::InvalidData));
        assert!(!directory.0.join("stale").exists());

        let loader = OverlayFileLoader::from_insertions(
            vec![insertion(&path, original, original.len())],
            &directory.0.join("artifacts"),
        )
        .unwrap();
        fs::write(&path, "mod changed {}").unwrap();
        assert_eq!(
            loader.read_file(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData,
        );
    }
}
