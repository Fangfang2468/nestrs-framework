//! `#[inject(...)]` 属性与 key 字面量的唯一解析入口。
//!
//! `#[injectable]` 字段与 `#[factory]` 参数共用这里的实现，因此两者对
//! `#[inject]`、`#[inject("name")]`、`#[inject(1)]` 与 `#[inject(key = ...)]`
//! 的接受范围与诊断文案始终一致。key 值的字面量规则由
//! [`crate::injection::attrs::service_key`] 定义，这里只负责属性形态。

use crate::injection::attrs::service_key::{self, ServiceKey};
use zyn::syn::{self, Attribute, Lit, Meta, parse::Parser, punctuated::Punctuated};

/// 从属性列表中取出唯一的 `#[inject(...)]` 并返回它声明的 key。
///
/// 未标注 `#[inject]` 与裸 `#[inject]` 都返回 `Ok(None)`（默认 key）；重复标注与
/// 非法参数形态返回错误。属性本身不由这里移除，调用方按自己的 AST 改写职责处理。
pub(crate) fn inject_key(attributes: &[Attribute]) -> syn::Result<Option<ServiceKey>> {
    let mut found: Option<&Attribute> = None;

    for attribute in attributes {
        if !attribute.path().is_ident("inject") {
            continue;
        }

        if found.is_some() {
            return Err(syn::Error::new_spanned(attribute, "重复的 #[inject] 属性"));
        }
        found = Some(attribute);
    }

    found
        .map(parse_inject_attribute)
        .transpose()
        .map(Option::flatten)
}

/// 解析单个 `#[inject]` / `#[inject(...)]` 属性。
fn parse_inject_attribute(attribute: &Attribute) -> syn::Result<Option<ServiceKey>> {
    match &attribute.meta {
        Meta::Path(_) => Ok(None),
        Meta::List(list) => {
            if list.tokens.is_empty() {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "#[inject] 不接受空参数；请使用 #[inject]、#[inject(\"key\")] 或 #[inject(key = \"key\")]",
                ));
            }

            if let Ok(literal) = syn::parse2::<Lit>(list.tokens.clone()) {
                return service_key::from_literal(&literal).map(Some);
            }

            let metas = Punctuated::<Meta, syn::Token![,]>::parse_terminated
                .parse2(list.tokens.clone())
                .map_err(|_| {
                    syn::Error::new_spanned(attribute, "#[inject] 只接受一个字符串或整数 key")
                })?;

            if metas.len() != 1 {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "#[inject] 只接受一个 key 参数",
                ));
            }

            let Some(Meta::NameValue(value)) = metas.first() else {
                return Err(syn::Error::new_spanned(
                    attribute,
                    "#[inject] 只接受字符串或整数 key；命名形式请写为 key = ...",
                ));
            };

            if !value.path.is_ident("key") {
                return Err(syn::Error::new_spanned(
                    &value.path,
                    "#[inject] 只支持 key 参数",
                ));
            }

            service_key::from_expression(&value.value).map(Some)
        }
        Meta::NameValue(_) => Err(syn::Error::new_spanned(
            attribute,
            "#[inject] 参数必须写在括号中",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zyn::syn::parse_quote;

    fn key_of(attribute: Attribute) -> syn::Result<Option<ServiceKey>> {
        inject_key(&[attribute])
    }

    #[test]
    fn parses_every_supported_key_form() {
        assert_eq!(key_of(parse_quote!(#[inject])).expect("bare"), None);
        assert_eq!(
            key_of(parse_quote!(#[inject("named")])).expect("positional string"),
            Some(ServiceKey::Named("named".to_owned()))
        );
        assert_eq!(
            key_of(parse_quote!(#[inject(7)])).expect("positional integer"),
            Some(ServiceKey::Indexed(7))
        );
        assert_eq!(
            key_of(parse_quote!(#[inject(key = "named")])).expect("named string"),
            Some(ServiceKey::Named("named".to_owned()))
        );
        assert_eq!(
            key_of(parse_quote!(#[inject(key = 3)])).expect("named integer"),
            Some(ServiceKey::Indexed(3))
        );
    }

    #[test]
    fn ignores_attributes_that_are_not_inject_markers() {
        let attributes: Vec<Attribute> = vec![parse_quote!(#[value(1)])];

        assert_eq!(
            inject_key(&attributes).expect("non-inject attributes are ignored"),
            None
        );
    }

    #[test]
    fn rejects_duplicate_empty_and_invalid_keys() {
        let duplicate = inject_key(&[parse_quote!(#[inject]), parse_quote!(#[inject])])
            .expect_err("duplicate markers must fail");
        assert!(duplicate.to_string().contains("重复的 #[inject] 属性"));

        let empty = key_of(parse_quote!(#[inject()])).expect_err("empty list must fail");
        assert!(empty.to_string().contains("不接受空参数"));

        let empty_name = key_of(parse_quote!(#[inject(key = "")])).expect_err("empty key");
        assert!(empty_name.to_string().contains("key 字符串不可为空"));

        let float = key_of(parse_quote!(#[inject(key = 1.5)])).expect_err("float key");
        assert!(float.to_string().contains("key 必须是字符串或非负整数值字面量"));

        let unknown = key_of(parse_quote!(#[inject(name = "x")])).expect_err("unknown key name");
        assert!(unknown.to_string().contains("只支持 key 参数"));

        let two = key_of(parse_quote!(#[inject(a = 1, b = 2)])).expect_err("two keys");
        assert!(two.to_string().contains("只接受一个 key 参数"));

        let name_value = key_of(parse_quote!(#[inject = 1])).expect_err("name-value form");
        assert!(name_value.to_string().contains("参数必须写在括号中"));
    }
}
