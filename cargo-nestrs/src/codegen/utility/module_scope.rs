use zyn::{syn, zyn};

/// 要求调用位置处于模块作用域。
///
/// 展开结果中插入两类零成本哨兵：
/// - `extern crate core as _;`：`extern crate` 不能出现在 `impl`/`trait` 体中，
///   因此在该类位置使用时直接报错；
/// - `use self::<ident> as _;`：函数/块中的局部项不能被 `self::` 引用，
///   因此嵌套在函数或块中的项会被拒绝。
///
/// `ident` 用于生成 `use self::` 哨兵；当调用方无法提供可安全引用的标识符时
/// （例如 `bind` 作用于泛型参数或全路径类型），可以传 `None`，此时仅保留
/// `extern crate` 哨兵。
#[zyn::element]
pub(crate) fn require_module_scope(
    ident: Option<syn::Ident>,
    children: zyn::TokenStream,
) -> zyn::TokenStream {
    let use_guard = ident.as_ref().map(|ident| -> syn::ItemUse {
        syn::parse_quote! {
            #[allow(unused_imports)]
            use self::#ident as _;
        }
    });

    zyn! {
        #[allow(unused_extern_crates)]
        extern crate core as _;

        {{ use_guard }}

        {{ children }}
    }
}

/// 从 impl 的 self 类型中提取可用于 `use self::` 哨兵的标识符。
///
/// 仅当 self 类型是「单段路径且不带泛型参数、且不是泛型参数本身」时返回 `Some`：
/// `impl Service` / `impl Trait for Service` 这类常规用法可以校验；
/// 全路径类型、泛型参数、带泛型实参的类型等返回 `None`，由调用方决定是否退化为仅
/// `extern crate` 哨兵。
pub(crate) fn impl_self_ident(item: &syn::ItemImpl) -> Option<syn::Ident> {
    let syn::Type::Path(type_path) = &*item.self_ty else {
        return None;
    };
    if type_path.qself.is_some() || type_path.path.segments.len() != 1 {
        return None;
    }
    let segment = &type_path.path.segments[0];
    if !matches!(segment.arguments, syn::PathArguments::None) {
        return None;
    }
    let ident = &segment.ident;
    let is_generic_param = item
        .generics
        .type_params()
        .any(|param| param.ident == *ident);
    if is_generic_param {
        return None;
    }
    Some(ident.clone())
}
