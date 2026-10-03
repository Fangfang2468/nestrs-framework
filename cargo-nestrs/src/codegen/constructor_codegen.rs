//! 显式构造函数只提供 Class adapter 与输入描述，不再创建第二份 provider。
//!
//! 生成项先经过标准展开/名称解析。driver 随后按 impl 的真实类型身份把它们接到
//! injectable 的唯一注册上，并按名称解析后的字段来源改写结构体。参数、函数体以及最终
//! 字段赋值仍交给 Rust 的类型、借用和可见性检查。

use crate::protocol::constructor;
use zyn::{Render, quote::quote, syn};

use super::{
    constructor::{ConstructorResultKind, analyze_constructor},
    injection::render::EmitDependencyRequest,
};

pub(super) fn expand(
    args: zyn::TokenStream,
    input: zyn::TokenStream,
) -> syn::Result<zyn::TokenStream> {
    if !args.is_empty() {
        return Err(syn::Error::new_spanned(
            args,
            "#[constructor] 不接受参数；服务策略由 #[injectable] 声明",
        ));
    }
    let analysis = analyze_constructor(syn::parse2(input)?)?;
    let item = &analysis.item;
    let method = &item.sig.ident;
    let mapping = serde_json::to_string(&constructor::Metadata {
        method: method.to_string(),
        result: analysis.result_kind == ConstructorResultKind::Result,
    })
    .expect("constructor metadata contains only a string and boolean");
    let metadata = super::reflection::ident(constructor::METADATA);
    let dependencies_helper = syn::Ident::new(constructor::DEPENDENCIES, method.span());
    let origin = super::source::origin(
        crate::protocol::OriginKind::Constructor,
        0,
        &method.to_string(),
        method.span(),
    );
    let activate = super::reflection::ident(constructor::ACTIVATE);
    let mut dependencies = Vec::new();
    let mut arguments = Vec::new();
    let mut acquisitions = Vec::new();
    for parameter in &analysis.parameters {
        dependencies.push(
            EmitDependencyRequest {
                request: parameter.dependency_request(),
            }
            .render(&zyn::Input::default())
            .tokens()
            .clone(),
        );
        let ty = &parameter.service_type;
        let slot = parameter.input_slot;
        let take = syn::Ident::new(
            match (parameter.lazy, parameter.optional) {
                (false, false) => "take",
                (false, true) => "take_optional",
                (true, false) => "take_lazy",
                (true, true) => "take_optional_lazy",
            },
            zyn::proc_macro2::Span::call_site(),
        );
        acquisitions.push(quote!(
            __nestrs_inputs.#take::<#ty>(::nestrs_core::activation::InputSlot::new(#slot))?
        ));
        let index = syn::Index::from(slot);
        arguments.push(quote!(__nestrs_inputs.#index));
    }
    let call = quote!(Self::#method(#(#arguments),*));
    let construct = match analysis.result_kind {
        ConstructorResultKind::Direct => call,
        ConstructorResultKind::Result => {
            quote!(#call.map_err(|error| ::nestrs_core::activation::ConstructionError::ConstructorFailed {
            provider: ::core::any::type_name::<Self>(),
            provider_source: ::nestrs_core::service::ServiceSource::new(file!(), line!(), column!()),
            detail: ::std::format!("{error:?}"),
        })?)
        }
    };
    let acquire_inputs = if analysis.parameters.is_empty() {
        quote! {
            __nestrs_inputs.ensure_all_consumed()?;
            ::core::mem::drop(__nestrs_inputs);
        }
    } else {
        // 复用已有参数绑定名，避免逐参数临时量与调用点同名 const 冲突。
        quote! {
            let __nestrs_inputs = (
                #(#acquisitions,)*
                {
                    __nestrs_inputs.ensure_all_consumed()?;
                    ::core::mem::drop(__nestrs_inputs);
                },
            );
        }
    };
    let reflection = super::reflection::support(false);
    let mutable = (!analysis.parameters.is_empty()).then(|| quote!(mut));
    Ok(quote! {
        #item

        #[doc(hidden)]
        #[allow(dead_code)]
        const #metadata: &'static str = #mapping;

        #[doc(hidden)]
        #[allow(dead_code)]
        pub(crate) fn #dependencies_helper() -> ::std::vec::Vec<::nestrs_core::activation::adapter::InputAdapter> {
            #reflection
            #origin
            ::std::vec![#(#dependencies),*]
        }

        #[doc(hidden)]
        #[allow(dead_code)]
        pub(crate) fn #activate(
            #mutable __nestrs_inputs: ::nestrs_core::activation::ConstructionInputs,
        ) -> ::core::result::Result<::nestrs_core::activation::ErasedService, ::nestrs_core::activation::ConstructionError> {
            #acquire_inputs
            let __nestrs_instance = #construct;
            ::core::result::Result::Ok(::nestrs_core::activation::ErasedService::new(__nestrs_instance))
        }
    })
}
