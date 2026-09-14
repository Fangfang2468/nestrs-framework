//! `#[injectable]` 的注册作用域组装。
//!
//! 构造 adapter 与 linkme provider factory 必须共享同一个匿名 `const` 的词法作用域：
//! provider 需要保存 adapter 的函数指针，而 adapter 又不能作为结构体的公开成员
//! 暴露给使用者。这个 element 仅承担该作用域关系，调用方通过 children 明确提供
//! 需要同域生成的节点。

use zyn::zyn;

/// 输出一个封装 children 的匿名注册作用域。
///
/// 构造 adapter 与 provider 分别由独立 element 生成；调用方把它们作为 children
/// 传入，以显式表达二者必须处在同一个词法作用域的关系。尾部 `()` 使 zyn 将包含
/// children 的 block 解析为 item scope；它对 const 的值是必要的自然 unit，因此由
/// 生成 const 的局部 lint allow 屏蔽，而不会泄漏到调用方。
#[zyn::element]
pub(crate) fn emit_injectable_registration(children: zyn::TokenStream) -> zyn::TokenStream {
    zyn! {
        #[allow(clippy::unused_unit)]
        const _: () = {
            {{ children }}
            ()
        };
    }
}

#[cfg(test)]
mod tests {
    use super::EmitInjectableRegistration;
    use zyn::{Render, quote::quote, syn};

    #[test]
    fn keeps_supplied_constructor_and_provider_in_one_lexical_scope() {
        let input = zyn::Input::default();
        let rendered = EmitInjectableRegistration {
            children: quote! {
                fn __nestrs_construct() {}

                fn __nestrs_reflect_provider() {
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
