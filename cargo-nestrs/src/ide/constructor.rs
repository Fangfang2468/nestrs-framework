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

pub const MODEL_VERSION: u32 = 1;
pub const MODEL_ENV: &str = "NESTRS_IDE_CONSTRUCTORS";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ConstructorModel {
    pub version: u32,
    pub declarations: Vec<Declaration>,
}

impl Default for ConstructorModel {
    fn default() -> Self {
        Self {
            version: MODEL_VERSION,
            declarations: Vec::new(),
        }
    }
}

/// 定位原始结构体标识符；行从 1 开始，列从 0 开始，与 proc_macro2 相同。
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub struct SourceAnchor {
    pub file: PathBuf,
    pub line: usize,
    pub column: usize,
    pub end_line: usize,
    pub end_column: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Declaration {
    pub anchor: SourceAnchor,
    /// helper 已消费、字段尚未包装的结构体 token；只用于检查模型和编辑内容一致。
    pub input: String,
    /// 编译器中的完整真实定义身份，用于歧义说明；不用于按名称猜测匹配。
    pub definition: String,
    pub selection: Selection,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Selection {
    pub constructor: bool,
    pub fields: Vec<FieldPlan>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FieldPlan {
    pub name: String,
    pub slot: usize,
    pub lazy: bool,
    pub optional: bool,
}

impl ConstructorModel {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != MODEL_VERSION {
            return Err(format!(
                "constructor IDE 模型版本 {} 不受支持，请重新运行 cargo nestrs init",
                self.version
            ));
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

fn model_path(directory: &Path, unit: &Unit) -> PathBuf {
    directory
        .join("constructors")
        .join(format!("{:016x}.json", unit.identity()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlocated_editor_spans_require_unambiguous_complete_input_in_the_exact_unit() {
        let first = declaration("crate::first::Service", true);
        let mut second = declaration("crate::second::Service", false);
        second.anchor.file = "/another.rs".into();
        let mut model = ConstructorModel {
            version: MODEL_VERSION,
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
