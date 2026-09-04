//! `#[primary]` 与 `#[injectable]` 之间的属性协调。
//!
//! 相邻属性宏按源码顺序展开。`#[primary]` 位于 `#[injectable]` 上方时，
//! `primary` 必须把自己的配置暂存到尚未展开的结构体上；反过来则由
//! `injectable` 直接消费下面的 `#[primary]` 属性。两条路径最终都在
//! `injectable` 内汇合，避免 `primary` 再次展开或独立生成注册代码。

use zyn::{
    meta::Args,
    syn::{self, spanned::Spanned, Attribute, Meta},
    zyn,
};

/// 仅供 `primary` 与 `injectable` 交接的私有属性。
///
/// 它必须追加在既有 `#[injectable]` 属性之后，这样 Rust 会先展开
/// `injectable`，由后者将该 marker 清除，而不会将它当作未知属性处理。
const DEFERRED_PRIMARY_ATTRIBUTE: &str = "__nestrs_injectable_primary";

/// `#[primary]` 的已解析配置。
///
/// 当前公开语法只有开关语义（`#[primary]` / `#[primary()]`）；保留独立配置
/// 类型，让属性顺序协调与未来可能的 `primary` 配置扩展保持在同一处。
#[derive(Clone, Debug, Default)]
pub(crate) struct PrimaryConfig {
    primary: bool,
    consumed_attribute_path: Option<syn::Path>,
}

impl PrimaryConfig {
    /// 解析正在展开的 `#[primary(...)]` 参数。
    pub(crate) fn from_args(args: &Args) -> syn::Result<Self> {
        if let Some(arg) = args.iter().next() {
            return Err(primary_arguments_error(arg.span()));
        }

        Ok(Self::enabled())
    }

    /// `injectable` 先展开时，解析仍留在结构体上的 `#[primary(...)]`。
    fn from_attribute(attribute: &Attribute) -> syn::Result<Self> {
        match &attribute.meta {
            Meta::Path(_) => Ok(Self::consumed(attribute.path().clone())),
            Meta::List(list) if list.tokens.is_empty() => {
                Ok(Self::consumed(attribute.path().clone()))
            }
            Meta::List(list) => Err(primary_arguments_error(list.tokens.span())),
            Meta::NameValue(name_value) => Err(primary_arguments_error(name_value.value.span())),
        }
    }

    pub(crate) fn is_primary(&self) -> bool {
        self.primary
    }

    /// 生成一个不执行属性宏的匿名导入。
    ///
    /// `injectable` 直接移除下方 `#[primary]` 后，Rust 不再将原始 macro import
    /// 视为已使用。`use primary as _;` 只做名称解析，不会再次展开 `primary`，可
    /// 保持用户常见的 `use nestrs_macro::{injectable, primary};` 写法无 warning。
    pub(crate) fn consumed_attribute_use(&self) -> Option<syn::ItemUse> {
        let path = self.consumed_attribute_path.as_ref()?;
        Some(syn::parse_quote!(use #path as _;))
    }

    fn enabled() -> Self {
        Self {
            primary: true,
            consumed_attribute_path: None,
        }
    }

    fn consumed(path: syn::Path) -> Self {
        Self {
            primary: true,
            consumed_attribute_path: Some(path),
        }
    }
}

/// 当 `primary` 先展开且下方仍有 `injectable` 时，追加私有交接 marker。
///
/// 这是一个保留 struct 形状的 element：顶层 `primary` 只在 `zyn!` 中表达
/// `Item` 分支，具体的 AST marker 准备由此处完成。
#[zyn::element]
pub(crate) fn defer_primary_to_injectable(
    item: syn::ItemStruct,
    primary: PrimaryConfig,
) -> zyn::TokenStream {
    let mut item = item.clone();

    if primary.is_primary() && has_attribute_named(&item.attrs, "injectable") {
        item.attrs
            .push(syn::parse_quote!(#[__nestrs_injectable_primary]));
    }

    zyn! {
        {{ item }}
    }
}

/// 消费 `injectable` 负责的 primary 配置及其交接 marker。
///
/// 这会同时移除源码中的 `#[primary]` 和上方 `primary` 留下的内部 marker，确保
/// 结构体被 `injectable` 重写后不会触发一次额外的 `primary` 宏展开。
pub(crate) fn take_primary_for_injectable(
    attributes: &mut Vec<Attribute>,
) -> syn::Result<PrimaryConfig> {
    let mut primary = None;
    let mut retained = Vec::with_capacity(attributes.len());

    for attribute in std::mem::take(attributes) {
        let config = if attribute_is_named(&attribute, "primary") {
            PrimaryConfig::from_attribute(&attribute)?
        } else if attribute_is_named(&attribute, DEFERRED_PRIMARY_ATTRIBUTE) {
            PrimaryConfig::enabled()
        } else {
            retained.push(attribute);
            continue;
        };

        if primary.replace(config).is_some() {
            return Err(syn::Error::new(
                attribute.span(),
                "同一个 `#[injectable]` 结构体不能重复标注 `#[primary]`",
            ));
        }
    }

    *attributes = retained;
    Ok(primary.unwrap_or_default())
}

fn has_attribute_named(attributes: &[Attribute], name: &str) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute_is_named(attribute, name))
}

fn attribute_is_named(attribute: &Attribute, name: &str) -> bool {
    attribute
        .path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == name)
}

fn primary_arguments_error(span: zyn::proc_macro2::Span) -> syn::Error {
    syn::Error::new(span, "`#[primary]` 不接受参数")
}

#[cfg(test)]
mod tests {
    use super::*;
    use zyn::{syn, Render};

    #[test]
    fn defers_primary_after_a_lower_injectable_attribute() {
        let item: syn::ItemStruct = syn::parse_quote! {
            #[injectable]
            struct Service;
        };

        let rendered = DeferPrimaryToInjectable {
            item,
            primary: PrimaryConfig::from_args(
                &syn::parse_str("").expect("empty args should parse"),
            )
            .expect("bare primary should be valid"),
        }
        .render(&zyn::Input::default());
        let mut item: syn::ItemStruct =
            syn::parse2(rendered.tokens().clone()).expect("element should render a struct");

        assert_eq!(item.attrs.len(), 2);
        assert!(attribute_is_named(&item.attrs[0], "injectable"));
        assert!(attribute_is_named(
            &item.attrs[1],
            DEFERRED_PRIMARY_ATTRIBUTE
        ));

        let primary =
            take_primary_for_injectable(&mut item.attrs).expect("deferred primary should parse");
        assert!(primary.is_primary());
        assert_eq!(item.attrs.len(), 1);
        assert!(attribute_is_named(&item.attrs[0], "injectable"));
    }

    #[test]
    fn consumes_a_lower_empty_primary_attribute() {
        let mut item: syn::ItemStruct = syn::parse_quote! {
            #[primary()]
            struct Service;
        };

        let primary =
            take_primary_for_injectable(&mut item.attrs).expect("empty primary should be valid");
        assert!(primary.is_primary());
        assert!(item.attrs.is_empty());
    }
}
