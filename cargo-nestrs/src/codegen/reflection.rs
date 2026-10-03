//! 按声明生成的反射支持代码；不构成应用依赖或 core 的注册 API。
//!
//! 每个局部模块和对应服务一起编译。marker 的真实类型参数、字面量和来源进入
//! rlib MIR，供工具链在最终入口做选择与验证；它们不执行用户构造或查找服务。
//! 开放泛型使用本声明专属 trait，因此私有类型无需通过一个公共反射 crate 暴露。

use zyn::{TokenStream, quote::quote};

pub(crate) fn support(generic: bool) -> TokenStream {
    let blueprint = generic.then(|| quote! {
        pub trait ProviderDefinition {
            fn provider() -> ::nestrs_core::activation::adapter::ActivationAdapter;
        }
        pub fn provider_definition<T: ProviderDefinition>() -> ::nestrs_core::activation::adapter::ActivationAdapter {
            T::provider()
        }
    });
    quote! {
        #[allow(dead_code)]
        mod __nestrs_reflect {
            #[derive(Clone, Copy)]
            pub enum CompilerKey { Default, Named(&'static str), Indexed(usize) }

            #[inline(never)]
            pub const fn compiler_provider<T: ?Sized>(_key: CompilerKey) {}
            #[inline(never)]
            pub const fn compiler_dependency<T: ?Sized, const SLOT: usize>() {}
            #[inline(never)]
            pub const fn compiler_binding<C: ?Sized, I: ?Sized>() {}
            #[inline(never)]
            pub const fn compiler_automatic_binding<C: ?Sized, I: ?Sized>() {}
            #[inline(never)]
            pub const fn compiler_plan_provider<T: ?Sized, const LIFETIME: u8, const PRIMARY: bool, const INITIALIZATION: u8>(_key: CompilerKey) {}
            #[inline(never)]
            pub const fn compiler_plan_input<T: ?Sized, const SLOT: usize, const OPTIONAL: bool, const LAZY: bool>(_key: CompilerKey, _label: &'static str) {}
            #[inline(never)]
            pub const fn compiler_plan_factory<const ASYNC: bool>() {}
            #blueprint
        }
    }
}
