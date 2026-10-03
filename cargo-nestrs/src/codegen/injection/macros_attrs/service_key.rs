//! 服务 key 的定义与字面量语法。
//!
//! `ServiceKeySpec` 同时被 provider 配置（`#[injectable(key = ...)]`、`#[factory(key = ...)]`）
//! 与依赖请求（`#[inject("name")]`、`#[inject(123)]`）使用，字面量规则只在这里实现一次。
//! 依赖请求只接受单个字面量，不改变 provider 的 `key = ...` 配置语法。

use zyn::{
    Arg, FromArg,
    proc_macro2::Span,
    syn::{self, Expr, ExprLit, Lit, spanned::Spanned},
};

/// key 的身份与它在用户源码中的位置分别保存。位置只用于诊断，不参与匹配。
#[derive(Debug, Clone)]
pub(crate) struct ServiceKeySpec {
    pub(crate) kind: ServiceKeyKind,
    pub(crate) span: Span,
}

impl PartialEq for ServiceKeySpec {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

impl Eq for ServiceKeySpec {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServiceKeyKind {
    /// 属性字面量中声明的服务名称。
    Named(String),

    /// 同一服务类型命名空间中的稳定编号。
    Indexed(usize),
}

/// 解析 `key = ...` 的值表达式。
///
/// 只接受字符串或非负整数字面量，供 provider 的命名配置使用。
pub(crate) fn from_expression(expression: &Expr) -> syn::Result<ServiceKeySpec> {
    let Expr::Lit(ExprLit { lit, .. }) = expression else {
        return Err(syn::Error::new_spanned(
            expression,
            "key 必须是字符串或非负整数值字面量",
        ));
    };

    from_literal(lit)
}

/// 解析一个字面量 token 形式的 key。
///
/// `#[inject("name")]` 这类位置参数直接以字面量 token 出现，因此与
/// [`from_expression`] 共用同一份规则。
pub(crate) fn from_literal(literal: &Lit) -> syn::Result<ServiceKeySpec> {
    match literal {
        Lit::Str(value) if value.value().is_empty() => {
            Err(syn::Error::new_spanned(value, "key 字符串不可为空"))
        }
        Lit::Str(value) => Ok(ServiceKeySpec {
            kind: ServiceKeyKind::Named(value.value()),
            span: value.span(),
        }),
        Lit::Int(value) => value
            .base10_parse::<usize>()
            .map(|index| ServiceKeySpec {
                kind: ServiceKeyKind::Indexed(index),
                span: value.span(),
            })
            .map_err(|_| {
                syn::Error::new_spanned(value, "key 整数必须是可表示为 usize 的非负字面量")
            }),
        _ => Err(syn::Error::new_spanned(
            literal,
            "key 必须是字符串或非负整数值字面量",
        )),
    }
}

impl FromArg for ServiceKeySpec {
    fn from_arg(arg: &zyn::Arg) -> zyn::Result<Self> {
        // provider 配置使用 `key = <字面量>`；注入字面量由 request 前端调用
        // `from_literal`，共用相同值校验，不接受 provider 的命名配置语法。
        let Arg::Expr(_, expression) = arg else {
            return Err(zyn::mark::error("key 必须是字符串或非负整数值字面量")
                .span(arg.span())
                .build());
        };

        from_expression(expression).map_err(|error| {
            zyn::mark::error(error.to_string())
                .span(error.span())
                .build()
        })
    }
}

#[cfg(test)]
impl ServiceKeySpec {
    pub(crate) fn named(value: &str) -> Self {
        Self {
            kind: ServiceKeyKind::Named(value.to_owned()),
            span: Span::call_site(),
        }
    }
    pub(crate) fn indexed(value: usize) -> Self {
        Self {
            kind: ServiceKeyKind::Indexed(value),
            span: Span::call_site(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zyn::syn::parse_quote;

    #[test]
    fn parses_string_and_integer_keys() {
        assert_eq!(
            from_expression(&parse_quote!("named")).expect("string key"),
            ServiceKeySpec::named("named")
        );
        assert_eq!(
            from_expression(&parse_quote!(7)).expect("integer key"),
            ServiceKeySpec::indexed(7)
        );
    }

    #[test]
    fn rejects_empty_and_non_literal_keys() {
        let empty = from_expression(&parse_quote!("")).expect_err("empty key must fail");
        assert!(empty.to_string().contains("key 字符串不可为空"));

        for rejected in [parse_quote!(1.5), parse_quote!(-1), parse_quote!(name)] {
            let error = from_expression(&rejected).expect_err("non-literal key must fail");
            assert!(
                error
                    .to_string()
                    .contains("key 必须是字符串或非负整数值字面量"),
                "unexpected diagnostic: {error}"
            );
        }
    }
}
