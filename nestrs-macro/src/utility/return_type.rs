use zyn::{
    syn::{self, spanned::Spanned},
    zyn,
};

/// 要求函数返回一个非 unit 的类型。
///
/// 没有显式 `->` 的函数在 Rust 中默认返回 `()`；`-> ()` 以及多余括号包裹的
/// unit 类型也同样会被拒绝。`macro_name` 由调用方提供，因此这个约束可以被多个
/// 函数属性宏复用。
#[zyn::element]
pub(crate) fn require_non_unit_return_type(
    macro_name: String,
    item: syn::ItemFn,
    children: zyn::TokenStream,
) -> zyn::TokenStream {
    let unit_return_span = match &item.sig.output {
        syn::ReturnType::Default => Some(item.sig.ident.span()),
        syn::ReturnType::Type(_, return_type) if is_unit_type(return_type) => {
            Some(return_type.span())
        }
        syn::ReturnType::Type(_, _) => None,
    };

    if let Some(span) = unit_return_span {
        return syn::Error::new(span, format!("`#[{macro_name}]` 标记的函数不能返回 `()`"))
            .into_compile_error()
            .into();
    }

    zyn! {
        {{ children }}
    }
}

/// 要求 `Result` 返回类型的 `Ok` 类型不是 unit。
///
/// 仅识别裸 `Result`、`core::result::Result` 与 `std::result::Result`，避免将
/// 任意同名的用户类型误判为标准库 `Result`。`macro_name` 由调用方提供，因此
/// 这个约束可以被多个函数属性宏复用。
#[zyn::element]
pub(crate) fn require_non_unit_result_ok_type(
    macro_name: String,
    item: syn::ItemFn,
    children: zyn::TokenStream,
) -> zyn::TokenStream {
    if let Some(span) = explicit_return_type(&item).and_then(unit_result_success_span) {
        return syn::Error::new(
            span,
            format!("`#[{macro_name}]` 标记的函数不能返回成功值为 `()` 的 `Result`"),
        )
        .into_compile_error()
        .into();
    }

    zyn! {
        {{ children }}
    }
}

/// 要求显式 `Future` 返回类型的 `Output` 不是 unit。
///
/// 识别 `impl Future<Output = _>` 与 `dyn Future<Output = _>` 中的裸
/// `Future`、`core::future::Future`、`std::future::Future` 约束。`macro_name`
/// 由调用方提供，因此这个约束可以被多个函数属性宏复用。
#[zyn::element]
pub(crate) fn require_non_unit_future_output_type(
    macro_name: String,
    item: syn::ItemFn,
    children: zyn::TokenStream,
) -> zyn::TokenStream {
    if let Some(span) = explicit_return_type(&item).and_then(unit_future_output_span) {
        return syn::Error::new(
            span,
            format!("`#[{macro_name}]` 标记的函数不能返回 `Output` 为 `()` 的 `Future`"),
        )
        .into_compile_error()
        .into();
    }

    zyn! {
        {{ children }}
    }
}

fn explicit_return_type(item: &syn::ItemFn) -> Option<&syn::Type> {
    match &item.sig.output {
        syn::ReturnType::Type(_, return_type) => Some(return_type),
        syn::ReturnType::Default => None,
    }
}

fn unit_result_success_span(return_type: &syn::Type) -> Option<::zyn::proc_macro2::Span> {
    let syn::Type::Path(type_path) = unparenthesized_type(return_type) else {
        return None;
    };
    if type_path.qself.is_some()
        || !is_standard_library_type_path(&type_path.path, "result", "Result")
    {
        return None;
    }

    let segment = type_path.path.segments.last()?;
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let syn::GenericArgument::Type(success_type) = arguments.args.iter().next()? else {
        return None;
    };

    is_unit_type(success_type).then(|| success_type.span())
}

fn unit_future_output_span(return_type: &syn::Type) -> Option<::zyn::proc_macro2::Span> {
    match unparenthesized_type(return_type) {
        syn::Type::ImplTrait(impl_trait) => impl_trait
            .bounds
            .iter()
            .find_map(unit_future_output_span_in_bound),
        syn::Type::TraitObject(trait_object) => trait_object
            .bounds
            .iter()
            .find_map(unit_future_output_span_in_bound),
        _ => None,
    }
}

fn unit_future_output_span_in_bound(
    bound: &syn::TypeParamBound,
) -> Option<::zyn::proc_macro2::Span> {
    let syn::TypeParamBound::Trait(trait_bound) = bound else {
        return None;
    };
    if !is_standard_library_type_path(&trait_bound.path, "future", "Future") {
        return None;
    }

    let segment = trait_bound.path.segments.last()?;
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    arguments.args.iter().find_map(|argument| {
        let syn::GenericArgument::AssocType(association) = argument else {
            return None;
        };
        (association.ident == "Output" && is_unit_type(&association.ty))
            .then(|| association.ty.span())
    })
}

fn is_standard_library_type_path(path: &syn::Path, module: &str, terminal: &str) -> bool {
    let mut segments = path.segments.iter();
    let Some(first) = segments.next() else {
        return false;
    };
    let Some(second) = segments.next() else {
        return first.ident == terminal;
    };
    let Some(third) = segments.next() else {
        return false;
    };

    segments.next().is_none()
        && matches!(first.ident.to_string().as_str(), "core" | "std")
        && second.ident == module
        && third.ident == terminal
}

fn is_unit_type(return_type: &syn::Type) -> bool {
    matches!(unparenthesized_type(return_type), syn::Type::Tuple(tuple) if tuple.elems.is_empty())
}

fn unparenthesized_type(return_type: &syn::Type) -> &syn::Type {
    match return_type {
        syn::Type::Paren(parenthesized) => unparenthesized_type(&parenthesized.elem),
        syn::Type::Group(grouped) => unparenthesized_type(&grouped.elem),
        _ => return_type,
    }
}
