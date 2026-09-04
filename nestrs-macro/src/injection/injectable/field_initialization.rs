//! `#[injectable]` 非注入字段的初始化表达式生成。
//!
//! 这个模块刻意不生成构造函数、也不改写结构体声明。它只把字段分析阶段已经
//! 确定的 `#[value(...)]` / `Default` 策略转换为有字段类型上下文的表达式；真正
//! 的构造函数由 `constructor` 模块生成。

use super::field_analyze::FieldStrategy;
use zyn::{
    syn::{Expr, ExprLit, Lit, Type},
    zyn,
};

/// 重写一个非注入字段的构造表达式。
///
/// 这是一个 expression-level element，而非 `pipe`：它同时需要字段类型和
/// `FieldStrategy`，并且在模板中直接表达 `#[value]` 与 `Default` 的两个输出形态。
/// 注入字段由构造 adapter 从 `ConstructionContext` 取得，不能也不应到达这里。
#[zyn::element]
pub(crate) fn rewrite_value_field(field_type: Type, strategy: FieldStrategy) -> zyn::TokenStream {
    let expression = match &strategy {
        FieldStrategy::Value { expression } => Some(expression.clone()),
        FieldStrategy::Default => None,
        FieldStrategy::Inject { .. } => {
            unreachable!("inject fields must be initialized by the constructor adapter")
        }
    };
    let uses_into_conversion = expression
        .as_ref()
        .map(|expression| should_use_into_conversion(expression))
        .unwrap_or(false);

    zyn! {
        @if (expression.is_some()) {
            @if (uses_into_conversion) {
                ::core::convert::Into::<{{ field_type }}>::into(
                    {{ expression.as_ref().unwrap() }}
                )
            } @else {
                { {{ expression.as_ref().unwrap() }} }
            }
        } @else {
            ::core::default::Default::default()
        }
    }
}

fn should_use_into_conversion(expression: &Expr) -> bool {
    matches!(
        expression,
        Expr::Lit(ExprLit {
            lit: Lit::Str(_),
            ..
        }) | Expr::Path(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::injection::injectable::field_analyze::collect_field_specs;
    use zyn::{syn, Render};

    fn render_initializer(field_type: Type, strategy: FieldStrategy) -> String {
        RewriteValueField {
            field_type,
            strategy,
        }
        .render(&zyn::Input::default())
        .tokens()
        .to_string()
    }

    #[test]
    fn retains_type_context_for_values_and_defaults() {
        let item: syn::ItemStruct = syn::parse_str(
            r#"
            struct Values {
                #[value("label")]
                label: String,
                #[value({ let retries = 2; retries + 1 })]
                retries: usize,
                enabled: bool,
            }
            "#,
        )
        .expect("test input should parse");
        let specs = collect_field_specs(&item.fields).expect("fields should be valid");
        let fields = item.fields.iter().collect::<Vec<_>>();

        assert_eq!(
            render_initializer(fields[0].ty.clone(), specs[0].strategy.clone()),
            ":: core :: convert :: Into :: < String > :: into (\"label\")"
        );
        assert_eq!(
            render_initializer(fields[1].ty.clone(), specs[1].strategy.clone()),
            "{ let retries = 2 ; retries + 1 }"
        );
        assert_eq!(
            render_initializer(fields[2].ty.clone(), specs[2].strategy.clone()),
            ":: core :: default :: Default :: default ()"
        );
    }
}
