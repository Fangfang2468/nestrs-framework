//! 延迟注入字段 / factory 参数的辅助属性。它由所属声明一次性消费，不是独立过程宏。
//!
//! 这里只接受裸 `#[lazy]`；它描述输入槽位的激活时机，不代表服务级预热策略。

use zyn::syn::{self, Attribute, Meta};

pub(crate) fn is_marker(attribute: &Attribute) -> bool {
    super::is_helper_attribute(attribute, "lazy")
}

/// 校验并读取唯一的 marker，保留原属性以供调用方统一执行 AST 清理。
pub(crate) fn parse(attributes: &[Attribute]) -> syn::Result<bool> {
    parse_with_message(attributes, "延迟注入字段只接受无参数的 #[lazy] 属性")
}

/// factory 参数默认就是依赖请求，因此无需同时出现 `#[inject]`。
/// 解析规则与字段一致，只调整错误的上下文，避免把参数误报为字段。
pub(crate) fn parse_parameter(attributes: &[Attribute]) -> syn::Result<bool> {
    parse_with_message(attributes, "延迟注入参数只接受无参数的 #[lazy] 属性")
}

fn parse_with_message(attributes: &[Attribute], message: &str) -> syn::Result<bool> {
    let mut found = false;
    for attribute in attributes.iter().filter(|attribute| is_marker(attribute)) {
        if found {
            return Err(syn::Error::new_spanned(attribute, "重复的 #[lazy] 属性"));
        }
        if !matches!(attribute.meta, Meta::Path(_)) {
            return Err(syn::Error::new_spanned(attribute, message));
        }
        found = true;
    }
    Ok(found)
}
