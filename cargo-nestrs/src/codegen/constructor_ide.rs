//! 把编译器已确认的构造选择应用到编辑器的同一份标准过程宏展开。
//!
//! 普通 rustc 展开保留两个候选，由 driver 在真实名称解析后选择；rust-analyzer 不装载
//! rustc hook，因此通过逐编译单元的语义模型选同一个候选。这里仅应用结果，不自行
//! 搜索 impl、不猜泛型或服务名称，不抑制类型检查。

use zyn::{quote::ToTokens, syn};

use crate::ide::constructor::{ConstructorModel, MODEL_ENV, Selection, SourceAnchor};

use super::injection::sub_macros::inject::{GrammarMessages, split_optional};

/// 无环境模型表示正常编译；有模型时必须精确匹配，不能在编辑内容变化后退回猜测。
pub(super) fn selection(item: &syn::ItemStruct) -> syn::Result<Option<Selection>> {
    let Some(environment) = std::env::var_os(MODEL_ENV) else {
        return Ok(None);
    };
    let environment = environment
        .into_string()
        .map_err(|_| syn::Error::new_spanned(&item.ident, "constructor IDE 模型必须为 UTF-8"))?;
    let model: ConstructorModel = serde_json::from_str(&environment).map_err(|error| {
        syn::Error::new_spanned(
            &item.ident,
            format!("constructor IDE 模型损坏：{error}；请重新运行 cargo nestrs init"),
        )
    })?;
    let model = normalize_model(model)?;
    let span = item.ident.span();
    let input = item.to_token_stream().to_string();
    // 原版 RA 的部分 proc-macro 协议不给 local_file；可用的真实 file 路径优先，
    // 完全无位置时让模型执行严格的整声明匹配，不伪造一个源码 anchor。
    let file = span.local_file().or_else(|| {
        let display = span.file();
        if display.is_empty() || display.starts_with('<') {
            return None;
        }
        std::path::PathBuf::from(display).canonicalize().ok()
    });
    let Some(file) = file else {
        return model
            .lookup_without_anchor(&input)
            .map(Some)
            .map_err(|message| syn::Error::new_spanned(&item.ident, message));
    };
    let start = span.start();
    let end = span.end();
    let anchor = SourceAnchor {
        file: file.canonicalize().unwrap_or(file),
        line: start.line,
        column: start.column,
        end_line: end.line,
        end_column: end.column,
    };
    model
        .lookup(&anchor, &input)
        .map(Some)
        .map_err(|message| syn::Error::new_spanned(&item.ident, message))
}

/// rustc 与不同版本的 RA 宏服务可能给 TokenStream::Display 使用不同空白格式。
/// 在当前宏宿主内重新解析并渲染已记录的完整声明，使比较发生在同一个打印协议下。
/// 保留所有 AST 内容；不能把指纹简化成服务名字或字段类型集合。
fn normalize_model(mut model: ConstructorModel) -> syn::Result<ConstructorModel> {
    for declaration in &mut model.declarations {
        let item = syn::parse_str::<syn::ItemStruct>(&declaration.input)?;
        declaration.input = item.to_token_stream().to_string();
    }
    Ok(model)
}

/// 在字段定义生成前应用已验证的存储映射，保留业务 AST 及其 span。
/// 构造候选由同一套 renderer 直接选择，不再重新解析和遍历生成后的 Rust 输出。
pub(super) fn apply_fields(
    structure: &mut syn::ItemStruct,
    selection: &Selection,
) -> syn::Result<()> {
    if selection.constructor {
        let service = &structure.ident;
        for plan in &selection.fields {
            let field = structure
                .fields
                .iter_mut()
                .find(|field| {
                    field.ident.as_ref().is_some_and(|ident| {
                        ident.to_string().trim_start_matches("r#")
                            == plan.name.trim_start_matches("r#")
                    })
                })
                .ok_or_else(|| {
                    syn::Error::new_spanned(
                        service,
                        format!(
                            "constructor IDE 模型中的字段 {} 已变化，请保存并刷新项目模型",
                            plan.name
                        ),
                    )
                })?;
            let (ty, optional) = split_optional(
                &field.ty,
                GrammarMessages {
                    optional_shape: "constructor 可选字段必须写为 Option<T>",
                },
            )?;
            if optional != plan.optional {
                return Err(syn::Error::new_spanned(
                    &field.ty,
                    "constructor IDE 模型的可选性与字段不一致，请保存并刷新项目模型",
                ));
            }
            let token: syn::Type = if plan.lazy {
                syn::parse_quote!(::nestrs_core::LazyInjection<#ty>)
            } else {
                syn::parse_quote!(::nestrs_core::Injection<#ty>)
            };
            field.ty = if optional {
                syn::parse_quote!(::core::option::Option<#token>)
            } else {
                token
            };
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ide::constructor::FieldPlan;
    use zyn::quote::quote;

    #[test]
    fn compiler_and_editor_token_printer_spacing_is_normalized_without_dropping_structure() {
        use crate::ide::constructor::{Declaration, MODEL_VERSION};
        let source = "struct Service { port: dyn Port, optional: Option<Other>, }";
        let item: syn::ItemStruct = syn::parse_str(source).unwrap();
        let model = ConstructorModel {
            version: MODEL_VERSION,
            declarations: vec![Declaration {
                anchor: SourceAnchor {
                    file: "/main.rs".into(),
                    line: 1,
                    column: 7,
                    end_line: 1,
                    end_column: 14,
                },
                input: source.into(),
                definition: "crate::Service".into(),
                selection: Selection {
                    constructor: true,
                    fields: Vec::new(),
                },
            }],
        };
        let model = normalize_model(model).unwrap();
        assert!(
            model
                .lookup_without_anchor(&item.to_token_stream().to_string())
                .unwrap()
                .constructor
        );
        let changed: syn::ItemStruct =
            syn::parse_str("struct Service { port: dyn Other, optional: Option<Other>, }").unwrap();
        assert!(
            model
                .lookup_without_anchor(&changed.to_token_stream().to_string())
                .is_err()
        );
    }

    #[test]
    fn editor_wraps_selected_fields_before_rendering_the_declaration() {
        let mut structure: syn::ItemStruct = syn::parse_quote! {
            struct Service<T: Clone> where T: Send {
                database: Database<T>,
                audit: Option<dyn Audit>,
                label: String,
            }
        };
        let generics = structure.generics.clone();
        let selection = Selection {
            constructor: true,
            fields: vec![
                FieldPlan {
                    name: "database".into(),
                    slot: 0,
                    lazy: false,
                    optional: false,
                },
                FieldPlan {
                    name: "audit".into(),
                    slot: 1,
                    lazy: true,
                    optional: true,
                },
            ],
        };
        apply_fields(&mut structure, &selection).unwrap();
        let tokens = structure.to_token_stream().to_string();
        assert_eq!(structure.generics, generics);
        assert!(tokens.contains("Injection < Database < T > >"));
        assert!(tokens.contains("Option < :: nestrs_core :: LazyInjection < dyn Audit > >"));
        assert!(tokens.contains("label : String"));
    }

    #[test]
    fn automatic_editor_mode_leaves_the_original_field_ast_unchanged() {
        let mut structure: syn::ItemStruct = syn::parse_quote! {
            struct Service { value: BusinessType }
        };
        let original = structure.clone();
        apply_fields(
            &mut structure,
            &Selection {
                constructor: false,
                fields: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(structure, original);
    }

    #[test]
    fn editor_field_mapping_preserves_raw_identifiers() {
        let output = quote!(
            struct Service {
                r#type: Database,
            }
        );
        // compiler 模型记录真实 Symbol；也兼容旧模型保留的源码 raw 拼写。
        for name in ["type", "r#type"] {
            let selection = Selection {
                constructor: true,
                fields: vec![FieldPlan {
                    name: name.into(),
                    slot: 0,
                    lazy: false,
                    optional: false,
                }],
            };
            let mut structure = syn::parse2(output.clone()).unwrap();
            apply_fields(&mut structure, &selection).unwrap();
            let tokens = structure.to_token_stream().to_string();
            assert!(tokens.contains("r#type : :: nestrs_core :: Injection < Database >"));
        }
    }

    #[test]
    fn changed_field_mapping_keeps_the_existing_editor_diagnostics() {
        for (name, optional, message) in [
            ("missing", false, "模型中的字段 missing 已变化"),
            ("database", true, "模型的可选性与字段不一致"),
        ] {
            let mut structure = syn::parse_quote!(
                struct Service {
                    database: Database,
                }
            );
            let selection = Selection {
                constructor: true,
                fields: vec![FieldPlan {
                    name: name.into(),
                    slot: 0,
                    lazy: false,
                    optional,
                }],
            };
            let error = apply_fields(&mut structure, &selection).unwrap_err();
            assert!(error.to_string().contains(message), "{error}");
        }
    }
}
