//! 拒绝 `unsafe` 与 `extern` 写法。
//!
//! `unsafe` 函数/`unsafe impl` 带有安全契约，`extern` 函数带有 FFI ABI，
//! 它们都属于复杂且容器场景用不上的形态；这里统一拒绝，只保留规范化写法。

use zyn::{
    syn::{self, spanned::Spanned},
    zyn,
};

/// 拒绝 `unsafe` 与 `extern` 函数，校验通过后原样透传 children。
///
/// 被容器直接调用的工厂函数不能携带 `unsafe` 安全契约，也不能带有 FFI 使用的
/// `extern` ABI。
#[zyn::element]
pub(crate) fn reject_unsafe_and_extern_fn(
    macro_name: String,
    item: syn::ItemFn,
    children: zyn::TokenStream,
) -> zyn::TokenStream {
    if let Some(unsafety) = &item.sig.unsafety {
        return syn::Error::new(
            unsafety.span(),
            format!("`#[{macro_name}]` 不能标注 `unsafe` 函数"),
        )
        .into_compile_error()
        .into();
    }

    if let Some(abi) = &item.sig.abi {
        return syn::Error::new(
            abi.span(),
            format!("`#[{macro_name}]` 不能标注 `extern` 函数"),
        )
        .into_compile_error()
        .into();
    }

    zyn! {
        {{ children }}
    }
}

/// 拒绝 `unsafe impl` 块，校验通过后原样透传 children。
#[zyn::element]
pub(crate) fn reject_unsafe_impl(
    macro_name: String,
    item: syn::ItemImpl,
    children: zyn::TokenStream,
) -> zyn::TokenStream {
    if let Some(unsafety) = &item.unsafety {
        return syn::Error::new(
            unsafety.span(),
            format!("`#[{macro_name}]` 不能标注 `unsafe` impl"),
        )
        .into_compile_error()
        .into();
    }

    zyn! {
        {{ children }}
    }
}
