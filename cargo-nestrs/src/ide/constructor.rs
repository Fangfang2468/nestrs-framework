//! 编译器验证过的显式构造选择，供原版 rust-analyzer 的私有桥接复用。
//!
//! 每份记录绑定实际 rustc 编译单元，声明身份来自已解析的真实 DefId 和源码 anchor。
//! 不扫描源码、不按类型短名称合并，也不为编辑器重新选择 constructor。完整 JSON 放入
//! rust-project 的逐 crate env；模型更新因此会触发标准项目重载，不依赖额外文件监听。

use std::{
    collections::BTreeMap,
    env, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::capture::Unit;

/// 编辑器 constructor 模型的格式版本；版本变化要求重新生成项目。
pub const MODEL_VERSION: u32 = 2;

/// 承载当前编译单元完整 constructor 模型的编辑器环境变量。
pub const MODEL_ENV: &str = "NESTRS_IDE_CONSTRUCTORS";

/// driver 已验证的声明、方法与辅助项名称集合，供编辑器只重放语义选择。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConstructorModel {
    /// 当前模型格式版本，用于拒绝过期捕获记录。
    pub version: u32,

    /// 当前编译单元内已验证的结构体声明。
    pub declarations: Vec<Declaration>,

    /// 为本单元分配且不冲突的辅助项名称；没有构造方法时可缺省。
    pub helpers: Option<HelperNames>,

    /// 已验证显式构造方法的完整输入及位置。
    pub methods: Vec<MethodDeclaration>,
}

impl Default for ConstructorModel {
    /// 创建当前版本的空模型，用于没有 constructor 捕获记录的编译单元。
    fn default() -> Self {
        Self {
            version: MODEL_VERSION,
            declarations: Vec::new(),
            helpers: None,
            methods: Vec::new(),
        }
    }
}

/// 由当前编译单元标准展开后的标识符目录分配，两个属性展开只重放同一已验证结果。
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct HelperNames {
    /// 生成的激活辅助方法名称。
    pub activate: String,

    /// 生成的依赖描述辅助方法名称。
    pub dependencies: String,
}

/// 显式构造方法的源码位置和完整输入，用于识别编辑器中的同一声明。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MethodDeclaration {
    /// 原始声明的位置；用于与编辑器输入建立来源对应。
    pub anchor: SourceAnchor,

    /// 完整方法 token 文本；位置回退仍要求输入完全一致。
    pub input: String,
}

/// 定位原始声明标识符；行从 1 开始，列从 0 开始，与 proc_macro2 相同。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct SourceAnchor {
    /// 原始声明所在文件的路径。
    pub file: PathBuf,

    /// 起始行，使用 1 起始编号。
    pub line: usize,

    /// 起始列，使用 0 起始编号。
    pub column: usize,

    /// 结束行，使用 1 起始编号。
    pub end_line: usize,

    /// 结束列，使用 0 起始编号。
    pub end_column: usize,
}

/// 结构体原始声明与编译器确认的字段存储选择。
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Declaration {
    /// 原始声明的位置；用于与编辑器输入建立来源对应。
    pub anchor: SourceAnchor,

    /// helper 已消费、字段尚未包装的结构体 token；只用于检查模型和编辑内容一致。
    pub input: String,

    /// 编译器中的完整真实定义身份，用于歧义说明；不用于按名称猜测匹配。
    pub definition: String,

    /// 真实编译器解析并验证的构造与字段选择。
    pub selection: Selection,
}

/// 是否采用显式构造，以及该选择所需的字段到输入槽位映射。
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Selection {
    /// 是否由显式 constructor 决定结构体的注入字段存储。
    pub constructor: bool,

    /// 显式参数映射到返回字段的列表；自动字段构造时为空。
    pub fields: Vec<FieldPlan>,
}

/// 显式构造成功返回中，一个字段对应的完整输入交付形态。
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FieldPlan {
    /// 返回结构体中需要改写存储类型的字段名称。
    pub name: String,

    /// 该字段的整值来源参数在构造输入中的槽位。
    pub slot: usize,

    /// 该输入是否按值交付 LazyInjection 句柄。
    pub lazy: bool,

    /// 该输入是否允许缺席并保留 Option 形态。
    pub optional: bool,
}

impl ConstructorModel {
    /// 检查版本、位置、辅助项身份和字段映射一致性，拒绝不完整或歧义模型。
    pub fn validate(&self) -> Result<(), String> {
        if self.version != MODEL_VERSION {
            return Err(format!(
                "constructor IDE 模型版本 {} 不受支持，请重新运行 cargo nestrs init",
                self.version
            ));
        }
        if self.helpers.is_none() && !self.methods.is_empty() {
            return Err("constructor IDE 模型缺少已分配的辅助项身份".into());
        }
        if let Some(helpers) = &self.helpers {
            for name in [&helpers.activate, &helpers.dependencies] {
                if name.starts_with("r#") || zyn::syn::parse_str::<zyn::syn::Ident>(name).is_err() {
                    return Err("constructor IDE 模型包含无效辅助项身份".into());
                }
            }
            if helpers.activate == helpers.dependencies {
                return Err("constructor IDE 模型包含重复辅助项身份".into());
            }
        }
        for method in &self.methods {
            if method.anchor.line == 0 || method.anchor.end_line == 0 {
                return Err("constructor IDE 模型缺少准确构造方法位置".into());
            }
        }
        for declaration in &self.declarations {
            if declaration.anchor.line == 0 || declaration.anchor.end_line == 0 {
                return Err(
                    "constructor IDE 模型缺少准确源码位置，请重新运行 cargo nestrs init".into(),
                );
            }
            if !declaration.selection.constructor && !declaration.selection.fields.is_empty() {
                return Err("constructor IDE 模型在自动构造模式下包含显式参数映射".into());
            }
            let mut fields = std::collections::BTreeSet::new();
            for field in &declaration.selection.fields {
                if !fields.insert(&field.name) {
                    return Err(format!("constructor IDE 模型包含重复字段 {}", field.name));
                }
            }
        }
        Ok(())
    }

    /// 按完整方法输入和来源定位已验证 helper，允许同文件内未保存编辑造成的位置平移。
    pub fn lookup_method(
        &self,
        anchor: Option<&SourceAnchor>,
        input: &str,
    ) -> Result<HelperNames, String> {
        self.validate()?;
        let matched = self.methods.iter().any(|method| {
            method.input == input && anchor.is_none_or(|anchor| method.anchor == *anchor)
        }) || anchor.is_some_and(|anchor| {
            self.methods
                .iter()
                .any(|method| method.input == input && method.anchor.file == anchor.file)
        });
        if !matched {
            return Err("当前构造方法与编译器 constructor 模型不一致，请保存文件并运行 cargo nestrs init check 刷新 IDE 模型".into());
        }
        self.helpers
            .clone()
            .ok_or_else(|| "constructor IDE 模型缺少已分配的辅助项身份".into())
    }

    /// 同一宏定义可能贡献多个同位置声明。只有选择完全一致才可安全复用，否则报错。
    pub fn lookup(&self, anchor: &SourceAnchor, input: &str) -> Result<Selection, String> {
        self.validate()?;
        let mut matches: Vec<_> = self
            .declarations
            .iter()
            .filter(|declaration| declaration.anchor == *anchor && declaration.input == input)
            .collect();
        // 未保存的普通业务编辑会整体移动后续声明的行号。位置失配时，只允许同一真实
        // 文件中完整声明 token 相同且语义选择一致的记录；绝不退回按服务名搜索。
        if matches.is_empty() {
            matches = self
                .declarations
                .iter()
                .filter(|declaration| {
                    declaration.anchor.file == anchor.file && declaration.input == input
                })
                .collect();
        }
        consistent_selection(&matches)
    }

    /// 某些原版 RA / proc-macro server 组合不给 Span 返回文件位置。此时只能在当前
    /// 精确编译单元内匹配完整声明 token，且所有命中的已验证语义必须完全一致。
    /// 这不是按类型名恢复身份；存在不同选择时必须报歧义，不能任取一个候选。
    pub fn lookup_without_anchor(&self, input: &str) -> Result<Selection, String> {
        self.validate()?;
        let matches = self
            .declarations
            .iter()
            .filter(|declaration| declaration.input == input)
            .collect::<Vec<_>>();
        consistent_selection(&matches)
    }
}

/// 要求全部同输入候选具有一致选择；缺失或分歧时提示刷新模型。
fn consistent_selection(matches: &[&Declaration]) -> Result<Selection, String> {
    let Some(first) = matches.first() else {
        return Err("当前服务声明与编译器 constructor 模型不一致，请保存文件并运行 cargo nestrs init check 刷新 IDE 模型".into());
    };
    if matches
        .iter()
        .any(|declaration| declaration.selection != first.selection)
    {
        let definitions = matches
            .iter()
            .map(|declaration| declaration.definition.as_str())
            .collect::<Vec<_>>()
            .join("、");
        return Err(format!(
            "标准过程宏无法区分具有相同输入的 constructor 声明：{definitions}；请为这些服务使用可区分的声明，或使用能提供准确 Span 位置的编辑器宏服务"
        ));
    }
    Ok(first.selection.clone())
}

/// 只在 cargo nestrs init 捕获期间输出；driver 在完成真实类型关联后调用。
pub fn write_constructor_model(
    args: &[String],
    source: &Path,
    model: &ConstructorModel,
) -> Result<(), String> {
    let Some(directory) = env::var_os("NESTRS_IDE_CAPTURE") else {
        return Ok(());
    };
    let cwd = env::current_dir().map_err(|error| error.to_string())?;
    let Some(unit) = Unit::parse(args, source, &cwd, BTreeMap::new()) else {
        return Ok(());
    };
    model.validate()?;
    let mut model = model.clone();
    for declaration in &mut model.declarations {
        declaration.anchor.file = declaration
            .anchor
            .file
            .canonicalize()
            .unwrap_or_else(|_| declaration.anchor.file.clone());
        declaration
            .selection
            .fields
            .sort_by(|left, right| left.name.cmp(&right.name));
    }
    for method in &mut model.methods {
        method.anchor.file = method
            .anchor
            .file
            .canonicalize()
            .unwrap_or_else(|_| method.anchor.file.clone());
    }
    model.declarations.sort_by(|left, right| {
        (&left.anchor, &left.input, &left.definition).cmp(&(
            &right.anchor,
            &right.input,
            &right.definition,
        ))
    });
    super::write_atomic(
        &model_path(Path::new(&directory), &unit),
        &serde_json::to_vec(&model).map_err(|error| error.to_string())?,
    )
}

/// 读取精确编译单元的模型并编码为 env 值；无记录时返回有效空模型。
pub(super) fn environment(captures: &Path, unit: &Unit) -> Result<String, String> {
    let path = model_path(captures, unit);
    let model = match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<ConstructorModel>(&bytes).map_err(|error| {
            format!("constructor IDE 模型无法读取：{}：{error}", path.display())
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => ConstructorModel::default(),
        Err(error) => {
            return Err(format!(
                "constructor IDE 模型无法读取：{}：{error}",
                path.display()
            ));
        }
    };
    model.validate()?;
    serde_json::to_string(&model).map_err(|error| error.to_string())
}

/// 按编译单元身份定位 constructor sidecar，隔离 feature/test 变体。
fn model_path(directory: &Path, unit: &Unit) -> PathBuf {
    directory
        .join("constructors")
        .join(format!("{:016x}.json", unit.identity()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_model_requires_complete_tokens_and_retains_file_scoped_shift_fallback() {
        let anchor = declaration("crate::Service", true).anchor;
        let input = "fn new(input: Dependency) -> Self { Self { input } }";
        let helpers = HelperNames {
            activate: "__nestrs_ide_constructor_1_activate".into(),
            dependencies: "__nestrs_ide_constructor_1_dependencies".into(),
        };
        let mut model = ConstructorModel {
            helpers: Some(helpers.clone()),
            methods: vec![MethodDeclaration {
                anchor: anchor.clone(),
                input: input.into(),
            }],
            ..ConstructorModel::default()
        };
        assert_eq!(model.lookup_method(Some(&anchor), input).unwrap(), helpers);
        assert_eq!(model.lookup_method(None, input).unwrap(), helpers);
        let mut moved = anchor.clone();
        moved.line += 4;
        moved.end_line += 4;
        assert_eq!(model.lookup_method(Some(&moved), input).unwrap(), helpers);
        moved.file = "/different.rs".into();
        assert!(model.lookup_method(Some(&moved), input).is_err());
        assert!(
            model
                .lookup_method(None, "fn new(input: Other) -> Self { Self { input } }")
                .is_err()
        );
        assert!(
            model
                .lookup_method(
                    None,
                    "fn new(input: Dependency) -> Self { Self::other(input) }"
                )
                .is_err()
        );
        model.methods.push(model.methods[0].clone());
        assert_eq!(model.lookup_method(None, input).unwrap(), helpers);
        model.helpers = None;
        assert!(model.lookup_method(None, input).is_err());
    }

    #[test]
    fn invalid_editor_helper_identities_are_rejected_before_codegen() {
        for invalid in ["", "123", "_", "fn", "a::b", "a b", "r#type", "r#valid"] {
            let model = ConstructorModel {
                helpers: Some(HelperNames {
                    activate: invalid.into(),
                    dependencies: "valid_dependencies".into(),
                }),
                ..ConstructorModel::default()
            };
            assert!(model.validate().is_err(), "invalid helper {invalid}");
        }
        let mut model = ConstructorModel {
            version: 1,
            ..ConstructorModel::default()
        };
        assert!(model.validate().unwrap_err().contains("版本"));
        model.version = MODEL_VERSION;
        model.helpers = Some(HelperNames {
            activate: "same".into(),
            dependencies: "same".into(),
        });
        assert!(model.validate().is_err());
    }

    #[test]
    fn unlocated_editor_spans_require_unambiguous_complete_input_in_the_exact_unit() {
        let first = declaration("crate::first::Service", true);
        let mut second = declaration("crate::second::Service", false);
        second.anchor.file = "/another.rs".into();
        let mut model = ConstructorModel {
            version: MODEL_VERSION,
            helpers: None,
            methods: Vec::new(),
            declarations: vec![first.clone(), second],
        };
        assert!(
            model
                .lookup_without_anchor(&first.input)
                .unwrap_err()
                .contains("无法区分")
        );
        assert!(model.lookup_without_anchor("struct Missing;").is_err());
        model.declarations[1].selection = first.selection.clone();
        assert!(
            model
                .lookup_without_anchor(&first.input)
                .unwrap()
                .constructor
        );
    }

    fn declaration(definition: &str, constructor: bool) -> Declaration {
        Declaration {
            anchor: SourceAnchor {
                file: "/source.rs".into(),
                line: 3,
                column: 7,
                end_line: 3,
                end_column: 14,
            },
            input: "struct Service;".into(),
            definition: definition.into(),
            selection: Selection {
                constructor,
                fields: Vec::new(),
            },
        }
    }

    #[test]
    fn model_matches_source_and_input_without_guessing_service_names() {
        let first = declaration("crate::first::Service", true);
        let model = ConstructorModel {
            version: MODEL_VERSION,
            helpers: None,
            methods: Vec::new(),
            declarations: vec![first.clone()],
        };
        assert!(
            model
                .lookup(&first.anchor, &first.input)
                .unwrap()
                .constructor
        );
        assert!(
            model
                .lookup(&first.anchor, "struct Service { changed: Value }")
                .is_err()
        );
        let mut moved = first.anchor.clone();
        moved.line += 1;
        assert!(model.lookup(&moved, &first.input).unwrap().constructor);
        moved.file = "/other-source.rs".into();
        assert!(model.lookup(&moved, &first.input).is_err());
    }

    #[test]
    fn conflicting_macro_expansions_are_rejected_instead_of_selecting_arbitrarily() {
        let first = declaration("crate::first::Service", true);
        let mut model = ConstructorModel {
            version: MODEL_VERSION,
            helpers: None,
            methods: Vec::new(),
            declarations: vec![first.clone(), declaration("crate::second::Service", false)],
        };
        assert!(
            model
                .lookup(&first.anchor, &first.input)
                .unwrap_err()
                .contains("无法区分")
        );
        model.declarations[1].selection = first.selection.clone();
        assert!(
            model
                .lookup(&first.anchor, &first.input)
                .unwrap()
                .constructor
        );
    }
}
