use zyn::{syn, zyn};

/// 校验给定的零参数函数路径调用后返回 `Future`。
///
/// 过程宏无法解析另一个模块中函数路径指向的 AST；因此以 Rust 的类型系统
/// 校验其调用结果。`async fn` 的调用结果必然实现 `Future`。
#[zyn::element]
pub(crate) fn should_be_async_fn(function_path: syn::Path) -> zyn::TokenStream {
    zyn! {
        const _: () = {
            #[allow(dead_code)]
            fn __nestrs_requires_async_fn<Future>(_: fn() -> Future)
            where
                Future: ::core::future::Future,
            {}

            #[allow(dead_code)]
            fn __nestrs_check_cleanup() {
                __nestrs_requires_async_fn({{ function_path }});
            }
        };
    }
}
