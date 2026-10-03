//! 用真实 trait bound 校验显式绑定接口，不按路径名称推断类型身份。

use zyn::{syn, zyn};

/// 通过泛型 trait bound 校验给定路径表示 Rust trait（即接口）类型。
#[zyn::element]
pub(crate) fn check_interface_type(
    interface: syn::Path,
    children: zyn::TokenStream,
) -> zyn::TokenStream {
    zyn! {
        const _: () = {
            #[allow(dead_code)]
            fn __nestrs_requires_interface<T: ?Sized + {{ interface }}>() {}
        };

        {{ children }}
    }
}
