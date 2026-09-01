use zyn::{syn, zyn};

/// 将 children 中的函数可见性规范化为私有。
#[zyn::element]
pub(crate) fn must_be_private_fn(children: zyn::TokenStream) -> zyn::TokenStream {
    let mut function: syn::ItemFn = match syn::parse2(children.clone()) {
        Ok(function) => function,
        Err(error) => return error.into_compile_error().into(),
    };

    function.vis = syn::Visibility::Inherited;

    zyn! {
        {{ function }}
    }
}
