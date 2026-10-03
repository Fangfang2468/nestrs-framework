//! 服务声明级 `#[lazy]` 的三态配置与属性顺序交接。
//!
//! None 继承容器策略，Some(true) 退出默认预热，Some(false) 主动参加预热。这里只
//! 描述服务本身的策略；字段 helper 的 `#[lazy]` 仍由独立模块解析，不接受布尔参数。
//! 在 provider 上方展开时只留下私有 marker，在下方时由 provider 直接消费原属性。
//! 最终类型描述和编译器 marker 都使用同一份配置，不会受属性顺序影响。

use zyn::{
    TokenStream,
    quote::quote,
    syn::{self, Attribute, Meta, spanned::Spanned},
};

use super::primary::has_attribute_named;

const DEFERRED_ATTRIBUTE: &str = "__nestrs_service_lazy";

#[derive(Clone, Debug, Default)]
pub(crate) struct ServiceLazyConfig {
    value: Option<bool>,
    consumed_attribute_path: Option<syn::Path>,
}

impl ServiceLazyConfig {
    pub(crate) fn from_tokens(tokens: TokenStream) -> syn::Result<Self> {
        let value = if tokens.is_empty() {
            // 标准过程宏对 #[lazy] 和 #[lazy()] 给出相同的空输入，二者必须一致。
            true
        } else {
            syn::parse2::<syn::LitBool>(tokens.clone())
                .map_err(|_| arguments_error(tokens.span()))?
                .value
        };
        Ok(Self {
            value: Some(value),
            consumed_attribute_path: None,
        })
    }

    pub(crate) fn value(&self) -> Option<bool> {
        self.value
    }

    /// provider 提前消费下方属性时仍做一次名称解析，保留用户宏 import 的已使用状态。
    pub(crate) fn consumed_attribute_use(&self) -> Option<syn::ItemUse> {
        let path = self.consumed_attribute_path.as_ref()?;
        Some(syn::parse_quote!(use #path as _;))
    }

    fn from_attribute(attribute: &Attribute) -> syn::Result<Self> {
        let mut config = match &attribute.meta {
            Meta::Path(_) => Self::from_tokens(TokenStream::new())?,
            Meta::List(list) => Self::from_tokens(list.tokens.clone())?,
            Meta::NameValue(value) => return Err(arguments_error(value.span())),
        };
        if !attribute.path().is_ident(DEFERRED_ATTRIBUTE) {
            config.consumed_attribute_path = Some(attribute.path().clone());
        }
        Ok(config)
    }
}

/// lazy 位于 provider 上方时仅追加 marker；真正声明仍只由 injectable/factory 生成。
pub(crate) fn defer_to_provider(
    mut item: syn::Item,
    config: ServiceLazyConfig,
) -> syn::Result<TokenStream> {
    let attributes = match &mut item {
        syn::Item::Struct(item) => {
            if !has_attribute_named(&item.attrs, "injectable") {
                return Err(syn::Error::new(
                    item.ident.span(),
                    "结构体上的 #[lazy] 必须与 #[injectable] 配合使用",
                ));
            }
            &mut item.attrs
        }
        syn::Item::Fn(item) => {
            if !has_attribute_named(&item.attrs, "factory") {
                return Err(syn::Error::new(
                    item.sig.ident.span(),
                    "函数上的 #[lazy] 必须与 #[factory] 配合使用",
                ));
            }
            &mut item.attrs
        }
        other => {
            return Err(syn::Error::new_spanned(
                other,
                "#[lazy] 只能标注 #[injectable] 结构体或 #[factory] 函数",
            ));
        }
    };
    if has_attribute_named(attributes, DEFERRED_ATTRIBUTE)
        || has_attribute_named(attributes, "lazy")
    {
        return Err(syn::Error::new(item.span(), DUPLICATE_ERROR));
    }
    let value = config.value.expect("正在展开的 lazy 属性必须有明确值");
    attributes.push(syn::parse_quote!(#[__nestrs_service_lazy(#value)]));
    Ok(quote!(#item))
}

/// 原属性与已展开 marker 共用一次解析；名称识别边界与 primary 一致。
/// 保留末段 lazy 的 qualified 路径和模块别名可识别，任意重命名宏不在推断范围内。
pub(crate) fn take_lazy_for_provider(
    attributes: &mut Vec<Attribute>,
) -> syn::Result<ServiceLazyConfig> {
    let mut lazy = None;
    let mut retained = Vec::with_capacity(attributes.len());
    for attribute in std::mem::take(attributes) {
        let is_lazy = attribute
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "lazy");
        if !is_lazy && !attribute.path().is_ident(DEFERRED_ATTRIBUTE) {
            retained.push(attribute);
            continue;
        }
        let config = ServiceLazyConfig::from_attribute(&attribute)?;
        if lazy.replace(config).is_some() {
            return Err(syn::Error::new_spanned(attribute, DUPLICATE_ERROR));
        }
    }
    *attributes = retained;
    Ok(lazy.unwrap_or_default())
}

const DUPLICATE_ERROR: &str = "同一个服务声明不能重复标注 #[lazy]";

fn arguments_error(span: zyn::proc_macro2::Span) -> syn::Error {
    syn::Error::new(
        span,
        "#[lazy] 只接受空参数或单个布尔字面量：#[lazy]、#[lazy(true)]、#[lazy(false)]",
    )
}
