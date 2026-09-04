//! `#[injectable]` 的注册作用域组装。
//!
//! 构造 adapter 与 linkme metadata factory 必须共享同一个匿名 `const` 的词法作用域：
//! metadata 需要保存 adapter 的函数指针，而 adapter 又不能作为结构体的公开成员
//! 暴露给使用者。这个 element 仅承担该作用域关系，调用方通过 children 明确提供
//! 需要同域生成的节点。

use zyn::zyn;

/// 输出一个封装 children 的匿名注册作用域。
///
/// 构造 adapter 与 metadata 分别由独立 element 生成；调用方把它们作为 children
/// 传入，以显式表达二者必须处在同一个词法作用域的关系。尾部的 `()` 既是 const
/// block 的自然 unit 值，也避免 zyn 将只含 `{{ children }}` 的花括号识别成插值。
#[zyn::element]
pub(crate) fn emit_injectable_registration(children: zyn::TokenStream) -> zyn::TokenStream {
    zyn! {
        const _: () = {
            {{ children }}
            ()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::EmitInjectableRegistration;
    use zyn::{quote::quote, syn, Render};

    #[test]
    fn keeps_supplied_constructor_and_metadata_in_one_lexical_scope() {
        let input = zyn::Input::default();
        let rendered = EmitInjectableRegistration {
            children: quote! {
                fn __nestrs_construct() {}

                fn __nestrs_reflect_metadata_injectable() {
                    let _ = __nestrs_construct;
                }
            },
        }
        .render(&input);
        let output = rendered.tokens();

        let scope: syn::ItemConst = syn::parse2(output.clone())
            .expect("registration output should be an anonymous const item");
        let syn::Expr::Block(block) = scope.expr.as_ref() else {
            panic!("registration const should contain a block");
        };

        assert_eq!(block.block.stmts.len(), 3);
        assert!(matches!(
            &block.block.stmts[0],
            syn::Stmt::Item(syn::Item::Fn(_))
        ));
        assert!(matches!(
            &block.block.stmts[1],
            syn::Stmt::Item(syn::Item::Fn(_))
        ));
        assert!(matches!(
            &block.block.stmts[2],
            syn::Stmt::Expr(syn::Expr::Tuple(tuple), None) if tuple.elems.is_empty()
        ));
    }
}
