//! 按声明生成的反射支持代码；不构成应用依赖或 core 的注册 API。
//!
//! 每个局部模块和对应服务一起编译。marker 的真实类型参数、字面量和来源进入
//! rlib MIR，供工具链在最终入口做选择与验证；它们不执行用户构造或查找服务。
//! 开放泛型使用本声明专属 trait，因此私有类型无需通过一个公共反射 crate 暴露。

use crate::protocol::{self, Marker};
use zyn::{TokenStream, quote::quote, syn};

pub(crate) fn ident(name: &str) -> syn::Ident {
    syn::Ident::new(name, zyn::proc_macro2::Span::call_site())
}

pub(crate) fn support(generic: bool) -> TokenStream {
    let module = ident(protocol::REFLECTION_MODULE);
    let key = ident(protocol::COMPILER_KEY);
    let definition = ident(protocol::PROVIDER_DEFINITION);
    let helper = ident(protocol::PROVIDER_HELPER);
    let [
        provider,
        dependency,
        binding,
        automatic_binding,
        plan_provider,
        plan_input,
        plan_factory,
    ] = [
        Marker::Provider,
        Marker::Dependency,
        Marker::Binding,
        Marker::AutomaticBinding,
        Marker::PlanProvider,
        Marker::PlanInput,
        Marker::PlanFactory,
    ]
    .map(|marker| ident(marker.name()));
    let blueprint = generic.then(|| quote! {
        pub trait #definition {
            fn provider() -> ::nestrs_core::activation::adapter::ActivationAdapter;
        }
        pub fn #helper<T: #definition>() -> ::nestrs_core::activation::adapter::ActivationAdapter {
            T::provider()
        }
    });
    quote! {
        #[allow(dead_code)]
        mod #module {
            #[derive(Clone, Copy)]
            pub enum #key { Default, Named(&'static str), Indexed(usize) }

            #[inline(never)]
            pub const fn #provider<T: ?Sized>(_key: #key) {}
            #[inline(never)]
            pub const fn #dependency<T: ?Sized, const SLOT: usize>() {}
            #[inline(never)]
            pub const fn #binding<C: ?Sized, I: ?Sized>() {}
            #[inline(never)]
            pub const fn #automatic_binding<C: ?Sized, I: ?Sized>() {}
            #[inline(never)]
            pub const fn #plan_provider<T: ?Sized, const LIFETIME: u8, const PRIMARY: bool, const INITIALIZATION: u8>(_key: #key) {}
            #[inline(never)]
            pub const fn #plan_input<T: ?Sized, const SLOT: usize, const OPTIONAL: bool, const LAZY: bool>(_key: #key, _label: &'static str) {}
            #[inline(never)]
            pub const fn #plan_factory<const ASYNC: bool>() {}
            #blueprint
        }
    }
}
