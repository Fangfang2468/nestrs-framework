use zyn::zyn;

/// 要求调用位置处于模块作用域。
///
/// `extern crate` 仅能声明在模块作用域；将这个零成本哨兵插入宏展开结果，
/// 可以让 Rust 在 `impl` 或 `trait` 体中使用时给出位置错误。
#[zyn::element]
pub(crate) fn require_module_scope(children: zyn::TokenStream) -> zyn::TokenStream {
    zyn! {
        #[allow(unused_extern_crates)]
        extern crate core as _;
        {{ children }}
    }
}
