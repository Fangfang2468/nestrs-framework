use zyn::{
    syn::{self, spanned::Spanned},
    zyn,
};

/// 解析 children 中的构造函数，校验签名并注入返回类型断言。
#[zyn::element]
pub(crate) fn check_constructor(macro_name: String, children: zyn::TokenStream) -> zyn::TokenStream {
    let item: syn::ItemFn = match syn::parse2(children.clone()) {
        Ok(item) => item,
        Err(error) => return error.into_compile_error().into(),
    };

    if let Some(asyncness) = &item.sig.asyncness {
        return syn::Error::new(
            asyncness.span(),
            format!("`#[{macro_name}]` 不能标注 `async` 函数"),
        )
        .into_compile_error()
        .into();
    }

    if item.sig.receiver().is_some() {
        return syn::Error::new(
            item.sig.ident.span(),
            format!("`#[{macro_name}]` 只能标注不带 `self` 的关联函数"),
        )
        .into_compile_error()
        .into();
    }

    let return_type = match &item.sig.output {
        syn::ReturnType::Type(_, ty) => ty.as_ref().clone(),
        syn::ReturnType::Default => {
            return syn::Error::new(
                item.sig.ident.span(),
                format!("`#[{macro_name}]` 标记的函数必须返回 `Self` 或当前 impl 的类型"),
            )
            .into_compile_error()
            .into();
        }
    };

    // 生成 `Option<返回类型> = Option<Self>` 的类型等式。
    // 该赋值仅在返回类型与当前 `Self` 相同时成立；`None` 不构造实例，也没有运行时副作用。
    let return_type_assertion: syn::Stmt = syn::parse_quote! {
        let _: ::core::option::Option<#return_type> = ::core::option::Option::<Self>::None;
    };

    // 将编译期断言置于函数体第一条语句，使 `-> Self` 与 `-> 当前 impl 类型` 都能通过校验。
    let mut output_item = item;
    output_item.block.stmts.insert(0, return_type_assertion);

    zyn! {
        {{ output_item }}
    }
}
