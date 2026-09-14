//! 服务 key 的定义与字面量语法。
//!
//! `ServiceKey` 同时被 provider 配置（`#[injectable(key = ...)]`、`#[factory(key = ...)]`）
//! 与依赖请求（`#[inject(key = ...)]`）使用，因此它的可接受写法只在这里实现一次。

use zyn::{
    Arg, FromArg,
    syn::{self, Expr, ExprLit, Lit, spanned::Spanned},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ServiceKey {
    /// 同一服务类型命名空间中的静态名称。
    Named(String),

    /// 同一服务类型命名空间中的稳定编号。
    Indexed(usize),
}

/// 解析 `key = ...` 的值表达式。
///
/// 只接受字符串或非负整数字面量；provider 配置与 `#[inject(key = ...)]` 共用该规则。
pub(crate) fn from_expression(expression: &Expr) -> syn::Result<ServiceKey> {
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
pub(crate) fn from_literal(literal: &Lit) -> syn::Result<ServiceKey> {
    match literal {
        Lit::Str(value) if value.value().is_empty() => {
            Err(syn::Error::new_spanned(value, "key 字符串不可为空"))
        }
        Lit::Str(value) => Ok(ServiceKey::Named(value.value())),
        Lit::Int(value) => value
            .base10_parse::<usize>()
            .map(ServiceKey::Indexed)
            .map_err(|_| {
                syn::Error::new_spanned(value, "key 整数必须是可表示为 usize 的非负字面量")
            }),
        _ => Err(syn::Error::new_spanned(
            literal,
            "key 必须是字符串或非负整数值字面量",
        )),
    }
}

impl FromArg for ServiceKey {
    fn from_arg(arg: &zyn::Arg) -> zyn::Result<Self> {
        // provider 配置只支持 `key = <字面量>`；`#[inject(key = ...)]` 由 request 前端
        // 调用同一个 `from_expression`，因此两条路径的接受范围与文案完全一致。
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
mod tests {
    use super::*;
    use zyn::syn::parse_quote;

    #[test]
    fn parses_string_and_integer_keys() {
        assert_eq!(
            from_expression(&parse_quote!("named")).expect("string key"),
            ServiceKey::Named("named".to_owned())
        );
        assert_eq!(
            from_expression(&parse_quote!(7)).expect("integer key"),
            ServiceKey::Indexed(7)
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
