//! `#[value(...)]` 子标注的唯一实现。
//!
//! 只有 `#[injectable]` 字段使用它：字段在宏生成的构造 adapter 被调用时按字段类型
//! 求值，因此它统一支持字面量、模块常量/静态项、可见路径、函数调用与普通组合表达式。
//! factory 参数没有默认构造阶段，外层宏会显式拒绝该子标注。
//!
//! 这里只保留表达式 AST：求值发生在 `injectable::field_initialization` 渲染出的构造
//! 代码里，而不是宏展开期。

use zyn::syn::{self, Attribute, Expr, Meta};

/// 该属性是否是 `#[value(...)]` 子标注。
pub(crate) fn is_marker(attribute: &Attribute) -> bool {
    attribute.path().is_ident("value")
}

/// 解析 `#[value(<Rust expression>)]`，保留表达式 AST 给构造 adapter 使用。
///
/// 未标注该子标注返回 `Ok(None)`；重复标注、裸 `#[value]`、空参数，以及已移除的
/// `expr = ...` / `func = ...` 写法都会得到定向诊断。
pub(crate) fn parse(attributes: &[Attribute]) -> syn::Result<Option<Expr>> {
    let Some(attribute) = single_marker(attributes)? else {
        return Ok(None);
    };

    let Meta::List(list) = &attribute.meta else {
        return Err(syn::Error::new_spanned(
            attribute,
            "#[value] 必须写为 #[value(<Rust expression>)]",
        ));
    };

    if list.tokens.is_empty() {
        return Err(syn::Error::new_spanned(
            attribute,
            "#[value] 必须包含一个 Rust 表达式，例如 #[value(1)]",
        ));
    }

    let expression = syn::parse2::<Expr>(list.tokens.clone())?;
    reject_removed_named_forms(attribute, &expression)?;

    Ok(Some(expression))
}

/// 取出唯一的 `#[value(...)]`；重复标注报错，未标注返回 `None`。
fn single_marker(attributes: &[Attribute]) -> syn::Result<Option<&Attribute>> {
    let mut found: Option<&Attribute> = None;

    for attribute in attributes {
        if !is_marker(attribute) {
            continue;
        }

        if found.is_some() {
            return Err(syn::Error::new_spanned(attribute, "重复的 #[value] 属性"));
        }
        found = Some(attribute);
    }

    Ok(found)
}

/// 拒绝已经移除的 `#[value(expr = ...)]` / `#[value(func = ...)]` 写法。
fn reject_removed_named_forms(attribute: &Attribute, expression: &Expr) -> syn::Result<()> {
    let Expr::Assign(assign) = expression else {
        return Ok(());
    };
    let Expr::Path(path) = assign.left.as_ref() else {
        return Ok(());
    };

    if path.path.is_ident("expr") {
        return Err(syn::Error::new_spanned(
            attribute,
            "#[value(expr = ...)] 已移除；请改用 #[value(...)]",
        ));
    }

    if path.path.is_ident("func") {
        return Err(syn::Error::new_spanned(
            attribute,
            "#[value(func = ...)] 已移除；请改用 #[value(path::to::function())]",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use zyn::{quote::ToTokens, syn::parse_quote};

    fn parse_one(attribute: Attribute) -> syn::Result<Option<Expr>> {
        parse(&[attribute])
    }

    #[test]
    fn ignores_attributes_without_the_value_marker() {
        let attributes: Vec<Attribute> = vec![parse_quote!(#[inject])];

        assert!(parse(&attributes).expect("no value marker").is_none());
    }

    #[test]
    fn keeps_the_expression_ast_for_the_constructor_adapter() {
        let expression = parse_one(parse_quote!(#[value(prefixed_name("user"))]))
            .expect("valid value")
            .expect("value marker present");

        assert_eq!(
            expression.to_token_stream().to_string(),
            "prefixed_name (\"user\")"
        );
    }

    #[test]
    fn rejects_bare_empty_and_duplicate_markers() {
        let bare = parse_one(parse_quote!(#[value])).expect_err("bare value must fail");
        assert!(bare.to_string().contains("#[value] 必须写为"));

        let empty = parse_one(parse_quote!(#[value()])).expect_err("empty value must fail");
        assert!(empty.to_string().contains("必须包含一个 Rust 表达式"));

        let duplicate = parse(&[parse_quote!(#[value(1)]), parse_quote!(#[value(2)])])
            .expect_err("duplicate markers must fail");
        assert!(duplicate.to_string().contains("重复的 #[value] 属性"));
    }

    #[test]
    fn rejects_removed_named_forms() {
        let expr = parse_one(parse_quote!(#[value(expr = 1)])).expect_err("legacy expr form");
        assert!(expr.to_string().contains("#[value(expr = ...)] 已移除"));

        let func = parse_one(parse_quote!(#[value(func = make_label)])).expect_err("legacy func");
        assert!(func.to_string().contains("#[value(func = ...)] 已移除"));
    }
}
