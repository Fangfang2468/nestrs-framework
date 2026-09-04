//! `#[injectable]` 字段的 AST 改写。

use super::field_analyze::{AnalyzedFields, FieldSpec, FieldStrategy};
use zyn::{
    syn::{self, Fields},
    zyn,
};

/// 将已分析的 `#[inject]` 字段改写为稳定的 `Inject<T>` 宏 ABI。
///
/// 仅依据 [`FieldSpec`] 改写，绝不再次读取字段属性，从而保持 metadata、构造输入
/// 和字段形状最终都来自同一份分析结果。
pub(crate) fn rewrite_injection_fields(specs: &[FieldSpec], fields: &mut Fields) {
    for spec in specs {
        let FieldStrategy::Inject {
            service_type,
            optional,
            ..
        } = &spec.strategy
        else {
            continue;
        };

        let field = fields
            .iter_mut()
            .nth(spec.index)
            .expect("FieldSpec index must reference the original field list");

        field.ty = if *optional {
            syn::parse_quote! {
                ::core::option::Option<::nestrs_core::__private::Inject<#service_type>>
            }
        } else {
            syn::parse_quote! {
                ::nestrs_core::__private::Inject<#service_type>
            }
        };
    }
}

/// 将字段分析结果转换为最终的结构体定义。
///
/// marker 已由 [`super::field_analyze::analyze_fields`] 统一移除；这个阶段只依据
/// `FieldSpec` 改写类型，绝不重新读取字段属性。
///
/// 这是一个输出结构体定义的 element。`AnalyzedFields` 是 typed IR，不能自然地
/// 经由 zyn template pipe 传递；用 element 直接表达“把分析结果渲染为改写后的
/// struct”，调用点也不需要额外的中间变量。
#[zyn::element]
pub(crate) fn rewrite_injection_field(analysis: AnalyzedFields) -> zyn::TokenStream {
    let mut item = analysis.item.clone();
    rewrite_injection_fields(&analysis.specs, &mut item.fields);

    zyn! {
        {{ item }}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::injection::injectable::field_analyze::{analyze_fields, collect_field_specs};
    use zyn::{quote::ToTokens, Render};

    #[test]
    fn rewrites_required_and_optional_inject_fields_only() {
        let mut item: syn::ItemStruct = syn::parse_str(
            r#"
            struct Consumer {
                #[inject]
                database: Database,
                #[inject]
                audit: Option<dyn Audit>,
                #[value(1)]
                retries: usize,
            }
            "#,
        )
        .expect("test input should parse");
        let specs = collect_field_specs(&item.fields).expect("fields should be valid");

        rewrite_injection_fields(&specs, &mut item.fields);

        let fields: Vec<_> = item.fields.iter().collect();
        assert_eq!(
            fields[0].ty.to_token_stream().to_string(),
            ":: nestrs_core :: __private :: Inject < Database >"
        );
        assert_eq!(
            fields[1].ty.to_token_stream().to_string(),
            ":: core :: option :: Option < :: nestrs_core :: __private :: Inject < dyn Audit > >"
        );
        assert_eq!(fields[0].attrs.len(), 1);
        assert_eq!(fields[1].attrs.len(), 1);
        assert_eq!(fields[2].attrs.len(), 1);
    }

    #[test]
    fn element_renders_the_rewritten_struct_from_analysis() {
        let item: syn::ItemStruct = syn::parse_str(
            r#"
            struct Consumer {
                #[inject]
                database: Database,
                #[value("configured")]
                name: String,
            }
            "#,
        )
        .expect("test input should parse");
        let analysis = analyze_fields(item).expect("fields should be valid");

        let input = zyn::Input::default();
        let rendered = RewriteInjectionField { analysis }.render(&input);
        let item: syn::ItemStruct =
            syn::parse2(rendered.tokens().clone()).expect("rewrite element should emit a struct");
        let fields: Vec<_> = item.fields.iter().collect();

        assert_eq!(
            fields[0].ty.to_token_stream().to_string(),
            ":: nestrs_core :: __private :: Inject < Database >"
        );
        assert_eq!(fields[1].ty.to_token_stream().to_string(), "String");
        assert!(fields.iter().all(|field| {
            field.attrs.iter().all(|attribute| {
                !attribute.path().is_ident("inject") && !attribute.path().is_ident("value")
            })
        }));
    }
}
