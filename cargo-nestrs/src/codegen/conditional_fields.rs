//! 让 rustc 在分析 injectable 之前统一处理字段的 cfg/cfg_attr。
//!
//! 属性宏收到的字段尚未全部进行条件编译；derive 输入则已经过 rustc 筛选。
//! 因此仅对带条件字段的声明生成一个无运行期意义的占位结构体，并在隐藏 derive
//! 中恢复真实声明。字段类型和非 cfg 属性保存在带原始 span 的 helper token 中，
//! 不会提前解析禁用字段引用的类型，也不需要从宿主环境猜测目标 crate 的 cfg。

use zyn::{
    TokenStream,
    quote::quote,
    syn::{self, Attribute, ItemStruct, Meta, Token, parse::Parse, punctuated::Punctuated},
};

pub(super) fn needs_filtering(item: &ItemStruct) -> bool {
    item.fields.iter().any(|field| {
        field
            .attrs
            .iter()
            .any(|attr| attr.path().is_ident("cfg") || attr.path().is_ident("cfg_attr"))
    })
}

pub(super) fn defer(
    args: TokenStream,
    item: ItemStruct,
    binding_span: zyn::proc_macro2::Span,
) -> syn::Result<TokenStream> {
    let mut declaration = item.clone();
    match &mut declaration.fields {
        syn::Fields::Named(fields) => fields.named.clear(),
        syn::Fields::Unnamed(fields) => fields.unnamed.clear(),
        syn::Fields::Unit => {}
    }

    let mut carrier = item;
    // carrier 留在业务模块中以便 derive 恢复原声明；只隔离内部类型的名称，
    // 不移动真实声明，也不改变其字段、泛型与业务属性的卫生来源。
    carrier.ident = zyn::format_ident!(
        "__NestrsConditionalFieldsFor{}",
        carrier.ident,
        span = binding_span.located_at(carrier.ident.span()),
    );
    carrier.vis = syn::Visibility::Inherited;
    carrier.generics = syn::Generics::default();
    carrier.attrs = vec![
        syn::parse_quote!(#[derive(::nestrs::__NestrsConfiguredInjectable)]),
        syn::parse_quote!(#[__nestrs_declaration((#args) #declaration)]),
        syn::parse_quote!(#[doc(hidden)]),
        syn::parse_quote!(#[allow(dead_code, non_camel_case_types)]),
    ];
    for field in &mut carrier.fields {
        let ty = field.ty.clone();
        field.ty = syn::parse_quote!(());
        field.attrs = field
            .attrs
            .iter()
            .map(|attr| {
                let meta = defer_attribute(&attr.meta)?;
                Ok(syn::parse_quote!(#[#meta]))
            })
            .collect::<syn::Result<Vec<_>>>()?;
        field.attrs.push(syn::parse_quote!(#[__nestrs_type(#ty)]));
    }
    Ok(quote!(#carrier))
}

/// cfg/cfg_attr 仍由 rustc 执行；其他属性延迟到真实声明上处理。对 cfg_attr 的
/// 递归映射同时覆盖条件生成的 inject/value、嵌套 cfg 和普通字段属性。
fn defer_attribute(meta: &Meta) -> syn::Result<Meta> {
    if meta.path().is_ident("cfg") {
        return Ok(meta.clone());
    }
    if meta.path().is_ident("cfg_attr") {
        let Meta::List(list) = meta else {
            return Err(syn::Error::new_spanned(meta, "cfg_attr 需要条件与属性"));
        };
        let parts = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
        let mut parts = parts.into_iter();
        let predicate = parts
            .next()
            .ok_or_else(|| syn::Error::new_spanned(meta, "cfg_attr 缺少条件"))?;
        let attributes = parts
            .map(|attribute| defer_attribute(&attribute))
            .collect::<syn::Result<Vec<_>>>()?;
        return Ok(syn::parse_quote!(cfg_attr(#predicate, #(#attributes),*)));
    }
    Ok(syn::parse_quote!(__nestrs_attribute(#meta)))
}

struct Declaration {
    args: TokenStream,
    item: ItemStruct,
}

impl Parse for Declaration {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let args;
        syn::parenthesized!(args in input);
        Ok(Self {
            args: args.parse()?,
            item: input.parse()?,
        })
    }
}

pub(super) fn restore(input: TokenStream) -> syn::Result<(TokenStream, ItemStruct)> {
    let carrier = syn::parse2::<ItemStruct>(input)?;
    let marker = carrier
        .attrs
        .iter()
        .find(|attr| attr.path().is_ident("__nestrs_declaration"))
        .ok_or_else(|| syn::Error::new_spanned(&carrier, "缺少内部 injectable 声明"))?;
    let Declaration { args, mut item } = marker.parse_args()?;
    item.fields = carrier.fields;
    for field in &mut item.fields {
        let marker = field
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("__nestrs_type"))
            .ok_or_else(|| syn::Error::new_spanned(&*field, "缺少内部 injectable 字段类型"))?;
        field.ty = marker.parse_args()?;
        field.attrs = field
            .attrs
            .iter()
            .filter(|attr| attr.path().is_ident("__nestrs_attribute"))
            .map(|attr| {
                let meta: Meta = attr.parse_args()?;
                Ok(syn::parse_quote!(#[#meta]))
            })
            .collect::<syn::Result<Vec<Attribute>>>()?;
    }
    Ok((args, item))
}
