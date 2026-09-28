//! 由多个属性宏共用的子标注。
//!
//! 子标注不是独立的属性宏，而是外层宏在自身展开过程中消费的 marker（例如
//! `#[injectable]` 字段与 `#[factory]` 参数上的 `#[inject]`）。它们没有自己的
//! 展开入口，因此共享的语法与语义集中定义在这里，外层宏只负责各自 AST 改写。

pub mod inject;
pub mod value;

/// Helpers can use the framework namespace without becoming standalone macros.
/// Keep other namespaces intact so an unrelated attribute is never consumed.
fn is_helper_attribute(attribute: &zyn::syn::Attribute, name: &str) -> bool {
    let path = attribute.path();
    path.is_ident(name)
        || (path.segments.len() == 2
            && path.segments[0].ident == "nestrs"
            && path.segments[1].ident == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zyn::syn;

    #[test]
    fn namespaced_and_bare_helpers_share_the_same_exact_marker_identity() {
        for attribute in [
            syn::parse_quote!(#[inject]),
            syn::parse_quote!(#[nestrs::inject]),
        ] {
            assert!(is_helper_attribute(&attribute, "inject"));
        }
        for attribute in [
            syn::parse_quote!(#[other::inject]),
            syn::parse_quote!(#[nestrs::other::inject]),
        ] {
            assert!(!is_helper_attribute(&attribute, "inject"));
        }
        let attributes = [
            syn::parse_quote!(#[inject]),
            syn::parse_quote!(#[nestrs::inject]),
        ];
        assert!(
            inject::inject_key(&attributes)
                .unwrap_err()
                .to_string()
                .contains("重复")
        );
    }
}
